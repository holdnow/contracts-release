use crate::{error::HoldNowCode, BondingCurve, BONDING_CURVE, SOL_VAULT_SEED, VAULT_SEED};
use anchor_lang::prelude::*;
use anchor_spl::token::{burn, Burn, Mint, Token, TokenAccount};
use solana_program::program::invoke_signed;
use solana_system_interface::instruction as system_instruction;

/// The launch reserves a fresh AMM pool budget even when accounts are prefunded.
/// This policy is shared with migration so the exit modes cannot disagree.
/// OpenBook market preparation is funded separately by the migration operator.
pub fn migration_budget(config: &AccountInfo, rent: &Rent) -> Result<u64> {
    let expected =
        Pubkey::find_program_address(&[b"amm_config_account_seed"], &crate::AMM_PROGRAM).0;
    require_keys_eq!(config.key(), expected, HoldNowCode::InvalidMigrationAccount);
    require_keys_eq!(
        *config.owner,
        crate::AMM_PROGRAM,
        HoldNowCode::InvalidMigrationAccount
    );
    let data = config.try_borrow_data()?;
    require!(data.len() == 544, HoldNowCode::InvalidMigrationAccount);
    let mut budget = u64::from_le_bytes(data[536..544].try_into().unwrap());
    // Shared with both client preflights; surplus prefunding enters pool liquidity.
    for size in crate::MIGRATION_ACCOUNT_SIZES {
        budget = budget
            .checked_add(rent.minimum_balance(size))
            .ok_or(HoldNowCode::MathOverflow)?;
    }
    Ok(budget)
}

pub fn has_minimum_liquidity(tokens: u64, sol: u64, decimals: u8) -> Result<bool> {
    // Raydium locks 10^coin_decimals initial LP units. At least one LP unit must
    // remain to burn: floor(sqrt(tokens * sol)) >= locked + 1.
    let minimum = 10u128
        .checked_pow(decimals as u32)
        .and_then(|n| n.checked_add(1))
        .ok_or(HoldNowCode::MathOverflow)?;
    let product = (tokens as u128)
        .checked_mul(sol as u128)
        .ok_or(HoldNowCode::MathOverflow)?;
    Ok(product
        >= minimum
            .checked_mul(minimum)
            .ok_or(HoldNowCode::MathOverflow)?)
}

/// Both exits use the same net-liquidity rule and the launch's immutable terms.
pub fn migration_liquidity(
    tokens: u64,
    balance: u64,
    reserved: u64,
    decimals: u8,
    minimum_sol: u64,
) -> Result<(u64, u64, bool)> {
    require!(
        minimum_sol >= crate::MIN_MIGRATION_SOL,
        HoldNowCode::InvalidConfiguration
    );
    let spendable = balance.saturating_sub(reserved);
    let fee = ((spendable as u128) * crate::MIGRATION_FEE_BPS as u128 / 10_000) as u64;
    let liquidity = spendable - fee;
    Ok((
        liquidity,
        fee,
        liquidity >= minimum_sol && has_minimum_liquidity(tokens, liquidity, decimals)?,
    ))
}

/// Deposit only the unsold inventory backed by pool SOL at the final curve price.
/// Flooring guarantees the opening marginal price cannot decrease; cap donations
/// at available inventory rather than minting or touching holder/reward tokens.
pub fn migration_pool_tokens(
    available: u64,
    pool_sol: u64,
    virtual_tokens: u64,
    virtual_sol: u64,
) -> Result<u64> {
    require!(virtual_sol > 0, HoldNowCode::MathOverflow);
    let matched = (pool_sol as u128)
        .checked_mul(virtual_tokens as u128)
        .ok_or(HoldNowCode::MathOverflow)?
        / virtual_sol as u128;
    Ok(matched.min(available as u128) as u64)
}

/// Migration and early redemption must use the same price-preserving LP floor.
pub fn price_matched_migration_liquidity(
    available: u64,
    balance: u64,
    reserved: u64,
    decimals: u8,
    minimum_sol: u64,
    virtual_tokens: u64,
    virtual_sol: u64,
) -> Result<(u64, u64, bool)> {
    let (liquidity, fee, _) =
        migration_liquidity(available, balance, reserved, decimals, minimum_sol)?;
    let pool_tokens = migration_pool_tokens(available, liquidity, virtual_tokens, virtual_sol)?;
    Ok((
        liquidity,
        fee,
        liquidity >= minimum_sol && has_minimum_liquidity(pool_tokens, liquidity, decimals)?,
    ))
}

#[derive(Accounts)]
pub struct ActivateRedemption<'info> {
    pub mint: Account<'info, Mint>,
    #[account(mut, seeds = [BONDING_CURVE, mint.key().as_ref()], bump)]
    pub bonding_curve: Box<Account<'info, BondingCurve>>,
    #[account(seeds = [SOL_VAULT_SEED, mint.key().as_ref()], bump)]
    pub vault: SystemAccount<'info>,
    #[account(seeds = [VAULT_SEED, mint.key().as_ref()], bump,
        token::mint = mint, token::authority = vault)]
    pub curve_tokens: Account<'info, TokenAccount>,
    /// CHECK: Validated for the underfunded exit only. Ignored after the fixed deadline.
    pub amm_config: AccountInfo<'info>,
    pub user: Signer<'info>,
}

pub fn activate_redemption(ctx: Context<ActivateRedemption>) -> Result<()> {
    let a = ctx.accounts;
    let c = &mut a.bonding_curve;
    c.assert_current()?;
    require!(c.complete, HoldNowCode::BondingCurveNotComplete);
    require!(!c.migrated, HoldNowCode::AlreadyMigrated);
    require!(!c.redemption_mode, HoldNowCode::RedemptionActive);
    let rent = Rent::get()?;
    let timed_out = Clock::get()?.unix_timestamp >= c.migration_deadline()?;
    if !timed_out {
        // Before expiry retain the shared underfunding rule. After expiry recovery
        // is independent of Raydium's availability, owner and configuration layout.
        let reserved = migration_budget(&a.amm_config, &rent)?
            .checked_add(rent.minimum_balance(0))
            .ok_or(HoldNowCode::MathOverflow)?;
        let (_, _, funded) = price_matched_migration_liquidity(
            a.curve_tokens.amount,
            a.vault.lamports(),
            reserved,
            a.mint.decimals,
            c.minimum_migration_sol,
            c.virtual_token_reserves,
            c.virtual_sol_reserves,
        )?;
        require!(!funded, HoldNowCode::MigrationFunded);
    }
    c.redemption_mode = true;
    emit!(RedemptionActivated {
        mint: a.mint.key(),
        remaining_sol: a.vault.lamports().saturating_sub(rent.minimum_balance(0))
    });
    Ok(())
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(mut)]
    pub mint: Account<'info, Mint>,
    #[account(mut, seeds = [BONDING_CURVE, mint.key().as_ref()], bump)]
    pub bonding_curve: Box<Account<'info, BondingCurve>>,
    #[account(mut, seeds = [SOL_VAULT_SEED, mint.key().as_ref()], bump)]
    pub vault: SystemAccount<'info>,
    #[account(seeds = [VAULT_SEED, mint.key().as_ref()], bump,
        token::mint = mint, token::authority = vault)]
    pub curve_tokens: Account<'info, TokenAccount>,
    #[account(mut, token::mint = mint, token::authority = user)]
    pub user_tokens: Account<'info, TokenAccount>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn redemption_output(available: u64, amount: u64, supply: u64) -> Result<u64> {
    require!(
        amount > 0 && amount <= supply,
        HoldNowCode::InvalidClaimAmount
    );
    let value = (available as u128)
        .checked_mul(amount as u128)
        .ok_or(HoldNowCode::MathOverflow)?
        .checked_div(supply as u128)
        .ok_or(HoldNowCode::MathOverflow)?;
    u64::try_from(value).map_err(|_| error!(HoldNowCode::MathOverflow))
}

pub fn redeem(ctx: Context<Redeem>, token_amount: u64, min_sol: u64) -> Result<()> {
    let a = ctx.accounts;
    let c = &mut a.bonding_curve;
    c.assert_current()?;
    require!(c.complete, HoldNowCode::BondingCurveNotComplete);
    require!(!c.migrated, HoldNowCode::AlreadyMigrated);
    require!(c.redemption_mode, HoldNowCode::RedemptionNotActive);
    // Include every outstanding reward and vesting token in the denominator.
    // Claiming moves tokens between accounts but cannot dilute another holder.
    let supply = a
        .mint
        .supply
        .checked_sub(a.curve_tokens.amount)
        .ok_or(HoldNowCode::MathOverflow)?;
    let available = a
        .vault
        .lamports()
        .saturating_sub(Rent::get()?.minimum_balance(0));
    let output = redemption_output(available, token_amount, supply)?;
    require!(output > 0, HoldNowCode::ZeroAmount);
    require!(output >= min_sol, HoldNowCode::TooLittleSolReceived);
    burn(
        CpiContext::new(
            a.token_program.key(),
            Burn {
                mint: a.mint.to_account_info(),
                from: a.user_tokens.to_account_info(),
                authority: a.user.to_account_info(),
            },
        ),
        token_amount,
    )?;
    let mint = a.mint.key();
    let seeds: &[&[u8]] = &[SOL_VAULT_SEED, mint.as_ref(), &[ctx.bumps.vault]];
    invoke_signed(
        &system_instruction::transfer(&a.vault.key(), &a.user.key(), output),
        &[
            a.vault.to_account_info(),
            a.user.to_account_info(),
            a.system_program.to_account_info(),
        ],
        &[seeds],
    )?;
    c.real_sol_reserves = c.real_sol_reserves.saturating_sub(output);
    emit!(TokensRedeemed {
        mint,
        user: a.user.key(),
        token_amount,
        sol_amount: output
    });
    Ok(())
}

#[event]
pub struct RedemptionActivated {
    pub mint: Pubkey,
    pub remaining_sol: u64,
}
#[event]
pub struct TokensRedeemed {
    pub mint: Pubkey,
    pub user: Pubkey,
    pub token_amount: u64,
    pub sol_amount: u64,
}

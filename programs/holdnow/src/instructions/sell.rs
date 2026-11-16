use anchor_lang::prelude::*;
use anchor_spl::token::{transfer, Mint, Token, TokenAccount, Transfer};

use crate::{
    constants::{
        BONDING_CURVE, GLOBAL_STATE_SEED, REWARD_STATE_SEED, REWARD_VAULT_SEED, SOL_VAULT_SEED,
        VAULT_SEED,
    },
    error::*,
    events::*,
    state::{BondingCurve, Global, UserSaleState},
};
use solana_program::program::invoke_signed;
use solana_system_interface::instruction as system_instruction;

#[derive(Accounts)]
pub struct Sell<'info> {
    #[account(
        seeds = [GLOBAL_STATE_SEED],
        bump
    )]
    pub global: Box<Account<'info, Global>>,

    #[account(
        mut,
        seeds = [REWARD_STATE_SEED, mint.key().as_ref()],
        bump
    )]
    /// CHECK:` doc comment explaining why no checks through types are necessary.
    pub reward_recipient: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [REWARD_VAULT_SEED, mint.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = reward_recipient,
    )]
    pub associated_reward_recipient: Box<Account<'info, TokenAccount>>,
    /// CHECK: Exact configured treasury is checked in the handler; SOL recipients may be program-owned.
    #[account(mut)]
    pub fee_recipient: AccountInfo<'info>,

    pub mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [SOL_VAULT_SEED, mint.key().as_ref()],
        bump
    )]
    /// CHECK: this should be set by admin
    pub vault: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [BONDING_CURVE, mint.key().as_ref()],
        bump
    )]
    pub bonding_curve: Box<Account<'info, BondingCurve>>,

    #[account(
        mut,
        seeds = [VAULT_SEED, mint.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = vault,
    )]
    pub associated_bonding_curve: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = mint,
        token::authority = user
    )]
    pub associated_user: Box<Account<'info, TokenAccount>>,

    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub clock: Sysvar<'info, Clock>,
    #[account(init_if_needed, payer = user, space = UserSaleState::SPACE,
        seeds = [crate::USER_SALE_STATE_SEED, mint.key().as_ref(), user.key().as_ref()], bump)]
    pub user_sale_state: Box<Account<'info, UserSaleState>>,
}

pub fn sell(ctx: Context<Sell>, token_amount: u64, min_sol_amount: u64) -> Result<()> {
    let a = ctx.accounts;
    a.global.assert_current()?;
    a.bonding_curve.assert_current()?;
    let stage = a.bonding_curve.assert_trading(a.clock.unix_timestamp)?;
    require_keys_eq!(
        a.fee_recipient.key(),
        a.global.fee_recipient,
        HoldNowCode::InValidFeeRecipient
    );
    require!(token_amount > 0, HoldNowCode::ZeroAmount);
    require!(
        a.global.fee_basis_points <= crate::MAX_FEE_PERCENT
            && a.global.staging_pool_fee <= crate::MAX_FEE_PERCENT,
        HoldNowCode::InvalidFeeBasisPoints
    );
    let staging = crate::utils::percentage(token_amount, a.global.staging_pool_fee)?;
    let after_staging = token_amount
        .checked_sub(staging)
        .ok_or(HoldNowCode::MathOverflow)?;
    let tax = crate::utils::percentage(
        after_staging,
        crate::utils::current_tax(&a.bonding_curve, a.clock.unix_timestamp)?,
    )?;
    let reward = staging.checked_add(tax).ok_or(HoldNowCode::MathOverflow)?;
    let to_curve = token_amount
        .checked_sub(reward)
        .ok_or(HoldNowCode::MathOverflow)?;
    require!(to_curve > 0, HoldNowCode::ZeroAmount);
    let gross = calculate_sol_cost(&a.bonding_curve, to_curve)?;
    require!(
        gross <= a.bonding_curve.real_sol_reserves,
        HoldNowCode::InsufficientVaultFunds
    );
    let fee = crate::utils::percentage(gross, a.global.fee_basis_points)?;
    let user_sol = gross.checked_sub(fee).ok_or(HoldNowCode::MathOverflow)?;
    require!(user_sol > 0, HoldNowCode::ZeroAmount);
    require!(
        user_sol >= min_sol_amount,
        HoldNowCode::TooLittleSolReceived
    );
    let mint = a.mint.key();
    let seeds: &[&[u8]] = &[SOL_VAULT_SEED, mint.as_ref(), &[ctx.bumps.vault]];
    for (to, amount) in [
        (a.fee_recipient.to_account_info(), fee),
        (a.user.to_account_info(), user_sol),
    ] {
        invoke_signed(
            &system_instruction::transfer(&a.vault.key(), to.key, amount),
            &[
                a.vault.to_account_info(),
                to,
                a.system_program.to_account_info(),
            ],
            &[seeds],
        )?;
    }
    for (to, amount) in [
        (a.associated_bonding_curve.to_account_info(), to_curve),
        (a.associated_reward_recipient.to_account_info(), reward),
    ] {
        transfer(
            CpiContext::new(
                a.token_program.key(),
                Transfer {
                    from: a.associated_user.to_account_info(),
                    to,
                    authority: a.user.to_account_info(),
                },
            ),
            amount,
        )?;
    }
    let c = &mut a.bonding_curve;
    c.stage_reward_pools[stage] = c.stage_reward_pools[stage]
        .checked_add(reward)
        .ok_or(HoldNowCode::MathOverflow)?;
    c.real_token_reserves = c
        .real_token_reserves
        .checked_add(to_curve)
        .ok_or(HoldNowCode::MathOverflow)?;
    c.virtual_token_reserves = c
        .virtual_token_reserves
        .checked_add(to_curve)
        .ok_or(HoldNowCode::MathOverflow)?;
    c.real_sol_reserves = c
        .real_sol_reserves
        .checked_sub(gross)
        .ok_or(HoldNowCode::MathOverflow)?;
    c.virtual_sol_reserves = c
        .virtual_sol_reserves
        .checked_sub(gross)
        .ok_or(HoldNowCode::MathOverflow)?;
    a.user_sale_state.token = mint;
    a.user_sale_state.user = a.user.key();
    a.user_sale_state.last_sold_stage =
        u8::try_from(c.current_stage).map_err(|_| error!(HoldNowCode::InvalidStage))?;
    a.user_sale_state.protocol_version = crate::PROTOCOL_VERSION;
    emit!(SwapEvent {
        stage: c.current_stage,
        mint,
        user: a.user.key(),
        bonding_curve: c.key(),
        reward_cap_addr: a.associated_reward_recipient.key(),
        sol_amount: user_sol,
        token_amount,
        reward_amount: reward,
        is_buy: false,
        token_supply: c.token_total_supply,
        timestamp: a.clock.unix_timestamp,
        reserve_lamport: c.virtual_sol_reserves,
        reserve_token: c.virtual_token_reserves,
        fee_amount: fee
    });
    Ok(())
}
fn calculate_sol_cost(c: &Account<BondingCurve>, amount: u64) -> Result<u64> {
    crate::utils::curve_output(amount, c.virtual_token_reserves, c.virtual_sol_reserves)
}

#[cfg(test)]
mod audit_unit_tests {
    include!("../../tests/unit/sell.rs");
}

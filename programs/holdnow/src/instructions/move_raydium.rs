use crate::raydium as amm_instruction;
use crate::{
    constants::{BONDING_CURVE, GLOBAL_STATE_SEED, SOL_VAULT_SEED},
    error::*,
    events::*,
    state::{BondingCurve, Global},
};
use anchor_lang::prelude::*;
use anchor_spl::token::spl_token::state::Account as SplAcc;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{burn, spl_token, transfer, Burn, Mint, Token, Transfer},
};
use solana_program::program::invoke_signed;
use solana_program::program_pack::Pack;
use solana_system_interface::instruction as system_instruction;

// SOLUTION: Remove PDA validations from accounts to reduce stack usage
#[derive(Accounts)]
pub struct MoveRaydium<'info> {
    // Remove seeds validation - validate manually in function
    #[account(mut)]
    pub global: Box<Account<'info, Global>>,

    #[account(mut)]
    /// CHECK: doesn't read data from this account
    pub fee_recipient: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: doesn't read data from this account
    pub raydium_fee_destination: AccountInfo<'info>,

    #[account(mut)]
    pub mint: Account<'info, Mint>,

    // Remove seeds validation
    #[account(mut)]
    /// CHECK: Validated in function
    pub vault: AccountInfo<'info>,

    // Remove seeds validation
    #[account(mut)]
    pub bonding_curve: Box<Account<'info, BondingCurve>>,

    /// CHECK: Safe
    pub amm_program: AccountInfo<'info>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub sysvar_rent: Sysvar<'info, Rent>,

    // Remove all seeds validations from AMM accounts
    #[account(mut)]
    /// CHECK: AMM account
    pub amm: AccountInfo<'info>,

    /// CHECK: AMM authority
    pub amm_authority: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: AMM open orders
    pub amm_open_orders: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: LP mint
    pub lp_mint: AccountInfo<'info>,

    #[account(mut)]
    pub pc_mint: Box<Account<'info, Mint>>,

    #[account(mut)]
    /// CHECK: Coin vault
    pub coin_vault: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: PC vault
    pub pc_vault: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: Target orders
    pub target_orders: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: AMM config
    pub amm_config: AccountInfo<'info>,

    /// CHECK: OpenBook program
    pub market_program: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: OpenBook market
    pub market: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: User coin token
    pub user_token_coin: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: User pc token
    pub user_token_pc: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: User lp token
    pub user_token_lp: AccountInfo<'info>,

    #[account(mut)]
    pub user: Signer<'info>,
    pub clock: Sysvar<'info, Clock>,
}

pub fn move_raydium(ctx: Context<MoveRaydium>, nonce: u8, fee_bps: u16) -> Result<()> {
    require!(fee_bps <= 10_000, HoldNowCode::InvalidFeeRate); // <= 100%

    let accts = ctx.accounts;
    accts.global.assert_current()?;
    accts.bonding_curve.assert_current()?;
    require!(!accts.bonding_curve.migrated, HoldNowCode::AlreadyMigrated);
    require!(
        !accts.bonding_curve.redemption_mode,
        HoldNowCode::RedemptionActive
    );
    // Execution is permissionless. All destinations, fees and liquidity remain
    // constrained; the caller receives no assets or caller-selected reimbursement.
    accts
        .bonding_curve
        .assert_migration_open(Clock::get()?.unix_timestamp)?;
    require_keys_eq!(
        accts.fee_recipient.key(),
        accts.global.fee_recipient,
        HoldNowCode::InValidFeeRecipient
    );
    require!(
        fee_bps == crate::MIGRATION_FEE_BPS,
        HoldNowCode::InvalidFeeRate
    );
    require!(
        accts.amm_program.key() == crate::AMM_PROGRAM
            && accts.amm_program.executable
            && accts.market_program.key() == crate::MARKET_PROGRAM
            && accts.market_program.executable
            && *accts.market.owner == crate::MARKET_PROGRAM
            && accts.raydium_fee_destination.key() == crate::AMM_FEE_DESTINATION,
        HoldNowCode::InvalidMigrationAccount
    );
    require_keys_eq!(
        accts.pc_mint.key(),
        anchor_spl::token::spl_token::native_mint::ID,
        HoldNowCode::InvalidMigrationAccount
    );
    let coin_key = Pubkey::find_program_address(
        &[crate::VAULT_SEED, accts.mint.key().as_ref()],
        ctx.program_id,
    )
    .0;
    require_keys_eq!(
        accts.user_token_coin.key(),
        coin_key,
        HoldNowCode::InvalidMigrationAccount
    );
    for (account, mint) in [
        (&accts.user_token_coin, accts.mint.key()),
        (&accts.user_token_pc, accts.pc_mint.key()),
    ] {
        require_keys_eq!(
            *account.owner,
            spl_token::ID,
            HoldNowCode::InvalidMigrationAccount
        );
        let data = account.try_borrow_data()?;
        let value = SplAcc::unpack(&data)?;
        require!(
            value.owner == accts.vault.key() && value.mint == mint,
            HoldNowCode::InvalidMigrationAccount
        );
    }

    // MANUAL VALIDATION: Validate PDAs manually instead of in the struct

    // Store mint key to avoid temporary value issue
    let mint_key = accts.mint.key();

    // Validate global PDA
    let (expected_global, _) = Pubkey::find_program_address(&[GLOBAL_STATE_SEED], ctx.program_id);
    require!(
        accts.global.key() == expected_global,
        HoldNowCode::NotAuthorized
    );

    // Validate vault PDA
    let (expected_vault, vault_bump) =
        Pubkey::find_program_address(&[SOL_VAULT_SEED, mint_key.as_ref()], ctx.program_id);
    require!(
        accts.vault.key() == expected_vault,
        HoldNowCode::NotAuthorized
    );

    // Validate bonding curve PDA
    let (expected_bonding_curve, _) =
        Pubkey::find_program_address(&[BONDING_CURVE, mint_key.as_ref()], ctx.program_id);
    require!(
        accts.bonding_curve.key() == expected_bonding_curve,
        HoldNowCode::NotAuthorized
    );

    // Validate AMM PDAs (if needed)
    let (expected_amm, _) = Pubkey::find_program_address(
        &[
            accts.amm_program.key.as_ref(),
            accts.market.key.as_ref(),
            b"amm_associated_seed",
        ],
        &accts.amm_program.key(),
    );
    require!(accts.amm.key() == expected_amm, HoldNowCode::NotAuthorized);

    msg!("PDAs validated successfully");

    let rent = Rent::get()?;

    // The same conservative fresh-pool budget gates both migration and redemption.
    // Prefunding individual AMM accounts cannot change which exit is available.
    let creation_budget = super::redeem::migration_budget(&accts.amm_config, &rent)?;
    let keep_cushion: u64 = rent.minimum_balance(0);
    let reserved = creation_budget
        .checked_add(keep_cushion)
        .ok_or(HoldNowCode::MathOverflow)?;

    // Check we have enough to proceed
    let vault_before = accts.vault.to_account_info().lamports();
    require!(vault_before > reserved, HoldNowCode::InsufficientVaultFunds);

    let bonding_curve = &mut accts.bonding_curve;
    require!(bonding_curve.complete, HoldNowCode::BondingCurveNotComplete);
    require!(
        bonding_curve.token_pool_destination == 3,
        HoldNowCode::IncorrectPoolDestination
    );
    let available_coin = SplAcc::unpack(&accts.user_token_coin.try_borrow_data()?)?.amount;
    let mint_supply_before = accts.mint.supply;
    require!(available_coin > 0, HoldNowCode::InsufficientFunds);
    let (init_pc_amount, fee_cap, funded) = super::redeem::price_matched_migration_liquidity(
        available_coin,
        vault_before,
        reserved,
        accts.mint.decimals,
        bonding_curve.minimum_migration_sol,
        bonding_curve.virtual_token_reserves,
        bonding_curve.virtual_sol_reserves,
    )?;
    require!(funded, HoldNowCode::InvalidLiquidity);
    require!(
        bonding_curve.virtual_sol_reserves > 0,
        HoldNowCode::MathOverflow
    );

    msg!("Unsold inventory available: {}", available_coin);

    let opentime = Clock::get()?.unix_timestamp as u64;

    // FIX: Use mint_key variable instead of creating temporary
    let vault_seeds = &[SOL_VAULT_SEED, mint_key.as_ref(), &[vault_bump]];
    let signed_vault_seeds = &[&vault_seeds[..]];

    // Transfer SOL to user_token_pc
    invoke_signed(
        &system_instruction::transfer(
            &accts.vault.key(),
            &accts.user_token_pc.key(),
            init_pc_amount,
        ),
        &[
            accts.vault.to_account_info(),
            accts.user_token_pc.to_account_info(),
            accts.system_program.to_account_info(),
        ],
        signed_vault_seeds,
    )?;

    // Sync native
    let sync_native_ix =
        spl_token::instruction::sync_native(&spl_token::ID, accts.user_token_pc.key)?;

    anchor_lang::solana_program::program::invoke(
        &sync_native_ix,
        &[
            accts.user_token_pc.to_account_info(),
            accts.token_program.to_account_info(),
        ],
    )?;

    // Include any pre-existing wrapped SOL or unsolicited dust, so donations to
    // the vault-owned wrapping account cannot permanently block graduation.
    let actual_pc_amount = SplAcc::unpack(&accts.user_token_pc.try_borrow_data()?)?.amount;
    let seed_coin_amount = super::redeem::migration_pool_tokens(
        available_coin,
        actual_pc_amount,
        bonding_curve.virtual_token_reserves,
        bonding_curve.virtual_sol_reserves,
    )?;
    msg!("Running raydium amm initialize2");

    // Create the Raydium instruction
    let initialize_ix = amm_instruction::initialize2(
        accts.amm_program.key,
        accts.amm.key,
        accts.amm_authority.key,
        accts.amm_open_orders.key,
        accts.lp_mint.key,
        &mint_key, // Use the stored mint_key
        &accts.pc_mint.key(),
        accts.coin_vault.key,
        accts.pc_vault.key,
        accts.target_orders.key,
        accts.amm_config.key,
        &accts.raydium_fee_destination.key,
        accts.market_program.key,
        accts.market.key,
        accts.vault.key,
        accts.user_token_coin.key,
        accts.user_token_pc.key,
        &accts.user_token_lp.key(),
        nonce,
        opentime,
        actual_pc_amount,
        seed_coin_amount,
    )?;

    // Use vec for accounts
    let account_infos = vec![
        accts.amm_program.to_account_info(),
        accts.amm.to_account_info(),
        accts.amm_authority.to_account_info(),
        accts.amm_open_orders.to_account_info(),
        accts.lp_mint.to_account_info(),
        accts.mint.to_account_info(),
        accts.pc_mint.to_account_info(),
        accts.coin_vault.to_account_info(),
        accts.pc_vault.to_account_info(),
        accts.target_orders.to_account_info(),
        accts.amm_config.to_account_info(),
        accts.raydium_fee_destination.to_account_info(),
        accts.market_program.to_account_info(),
        accts.market.to_account_info(),
        accts.vault.to_account_info(),
        accts.user_token_coin.to_account_info(),
        accts.user_token_pc.to_account_info(),
        accts.user_token_lp.to_account_info(),
        accts.token_program.to_account_info(),
        accts.system_program.to_account_info(),
        accts.associated_token_program.to_account_info(),
        accts.sysvar_rent.to_account_info(),
    ];

    invoke_signed(&initialize_ix, &account_infos, signed_vault_seeds)?;

    // Update state
    //bonding_curve.virtual_token_reserves = 0;
    //bonding_curve.real_token_reserves = 0;
    //bonding_curve.real_sol_reserves = 0;

    require_keys_eq!(
        *accts.user_token_lp.owner,
        spl_token::ID,
        HoldNowCode::InvalidLiquidity
    );
    let lp = SplAcc::unpack(&accts.user_token_lp.try_borrow_data()?)?;
    require!(
        lp.mint == accts.lp_mint.key() && lp.owner == accts.vault.key() && lp.amount > 0,
        HoldNowCode::InvalidLiquidity
    );
    let coin_remaining = SplAcc::unpack(&accts.user_token_coin.try_borrow_data()?)?.amount;
    let pc_remaining = SplAcc::unpack(&accts.user_token_pc.try_borrow_data()?)?.amount;
    require!(
        coin_remaining
            == available_coin
                .checked_sub(seed_coin_amount)
                .ok_or(HoldNowCode::MathOverflow)?
            && pc_remaining == 0,
        HoldNowCode::InvalidLiquidity
    );
    burn(
        CpiContext::new(
            accts.token_program.key(),
            Burn {
                mint: accts.lp_mint.to_account_info(),
                from: accts.user_token_lp.to_account_info(),
                authority: accts.vault.to_account_info(),
            },
        )
        .with_signer(signed_vault_seeds),
        lp.amount,
    )?;
    // Pay only the disclosed fee, then put every unused creation lamport into
    // this pool's WSOL reserve. This includes savings from prefunded PDAs.
    // A future incompatible AMM cost increase fails atomically, never silently
    // consumes the fee or leaves an inconsistent migration-complete record.
    let after_vault = accts.vault.lamports();
    let surplus = after_vault
        .checked_sub(keep_cushion)
        .and_then(|n| n.checked_sub(fee_cap))
        .ok_or(HoldNowCode::InsufficientVaultFunds)?;
    let expected_pc = Pubkey::find_program_address(
        &[
            accts.amm_program.key.as_ref(),
            accts.market.key.as_ref(),
            b"pc_vault_associated_seed",
        ],
        accts.amm_program.key,
    )
    .0;
    require_keys_eq!(
        accts.pc_vault.key(),
        expected_pc,
        HoldNowCode::InvalidMigrationAccount
    );
    require_keys_eq!(
        *accts.pc_vault.owner,
        spl_token::ID,
        HoldNowCode::InvalidMigrationAccount
    );
    let pool_pc = SplAcc::unpack(&accts.pc_vault.try_borrow_data()?)?;
    require!(
        pool_pc.mint == spl_token::native_mint::ID
            && pool_pc.owner == accts.amm_authority.key()
            && pool_pc.is_native.is_some(),
        HoldNowCode::InvalidMigrationAccount
    );
    for (destination, amount) in [
        (accts.fee_recipient.to_account_info(), fee_cap),
        (accts.pc_vault.to_account_info(), surplus),
    ] {
        if amount > 0 {
            invoke_signed(
                &system_instruction::transfer(&accts.vault.key(), destination.key, amount),
                &[
                    accts.vault.to_account_info(),
                    destination,
                    accts.system_program.to_account_info(),
                ],
                signed_vault_seeds,
            )?;
        }
    }
    // A configured treasury can itself be the pool vault; synchronize that fee
    // too, preserving the existing permission to use a program-owned treasury.
    let pool_topup = surplus
        .checked_add(if accts.fee_recipient.key() == accts.pc_vault.key() {
            fee_cap
        } else {
            0
        })
        .ok_or(HoldNowCode::MathOverflow)?;
    {
        let sync = spl_token::instruction::sync_native(&spl_token::ID, accts.pc_vault.key)?;
        anchor_lang::solana_program::program::invoke(
            &sync,
            &[
                accts.pc_vault.to_account_info(),
                accts.token_program.to_account_info(),
            ],
        )?;
        let funded = SplAcc::unpack(&accts.pc_vault.try_borrow_data()?)?;
        require!(
            funded.amount
                >= pool_pc
                    .amount
                    .checked_add(pool_topup)
                    .ok_or(HoldNowCode::MathOverflow)?,
            HoldNowCode::InvalidLiquidity
        );
    }
    require!(
        accts.vault.lamports() == keep_cushion,
        HoldNowCode::InvalidLiquidity
    );
    let total_pool_sol = SplAcc::unpack(&accts.pc_vault.try_borrow_data()?)?.amount;
    require!(
        total_pool_sol
            >= actual_pc_amount
                .checked_add(pool_topup)
                .ok_or(HoldNowCode::MathOverflow)?,
        HoldNowCode::InvalidLiquidity
    );
    // Creation savings and wrapped-SOL donations are now known exactly. Top up
    // the token side to the same price, then burn only the unsold remainder.
    let coin_amount = super::redeem::migration_pool_tokens(
        available_coin,
        total_pool_sol,
        bonding_curve.virtual_token_reserves,
        bonding_curve.virtual_sol_reserves,
    )?;
    let expected_coin = Pubkey::find_program_address(
        &[
            accts.amm_program.key.as_ref(),
            accts.market.key.as_ref(),
            b"coin_vault_associated_seed",
        ],
        accts.amm_program.key,
    )
    .0;
    require_keys_eq!(
        accts.coin_vault.key(),
        expected_coin,
        HoldNowCode::InvalidMigrationAccount
    );
    require_keys_eq!(
        *accts.coin_vault.owner,
        spl_token::ID,
        HoldNowCode::InvalidMigrationAccount
    );
    let pool_coin = SplAcc::unpack(&accts.coin_vault.try_borrow_data()?)?;
    require!(
        pool_coin.mint == mint_key
            && pool_coin.owner == accts.amm_authority.key()
            && pool_coin.amount == seed_coin_amount,
        HoldNowCode::InvalidLiquidity
    );
    let additional_coin = coin_amount
        .checked_sub(seed_coin_amount)
        .ok_or(HoldNowCode::MathOverflow)?;
    if additional_coin > 0 {
        transfer(
            CpiContext::new(
                accts.token_program.key(),
                Transfer {
                    from: accts.user_token_coin.to_account_info(),
                    to: accts.coin_vault.to_account_info(),
                    authority: accts.vault.to_account_info(),
                },
            )
            .with_signer(signed_vault_seeds),
            additional_coin,
        )?;
    }
    let burned_unsold = available_coin
        .checked_sub(coin_amount)
        .ok_or(HoldNowCode::MathOverflow)?;
    if burned_unsold > 0 {
        burn(
            CpiContext::new(
                accts.token_program.key(),
                Burn {
                    mint: accts.mint.to_account_info(),
                    from: accts.user_token_coin.to_account_info(),
                    authority: accts.vault.to_account_info(),
                },
            )
            .with_signer(signed_vault_seeds),
            burned_unsold,
        )?;
    }
    accts.mint.reload()?;
    let final_coin = SplAcc::unpack(&accts.coin_vault.try_borrow_data()?)?.amount;
    let final_sol = SplAcc::unpack(&accts.pc_vault.try_borrow_data()?)?.amount;
    require!(
        final_coin == coin_amount
            && final_sol == total_pool_sol
            && SplAcc::unpack(&accts.user_token_coin.try_borrow_data()?)?.amount == 0
            && accts.mint.supply
                == mint_supply_before
                    .checked_sub(burned_unsold)
                    .ok_or(HoldNowCode::MathOverflow)?,
        HoldNowCode::InvalidLiquidity
    );
    require!(
        (final_sol as u128) * (bonding_curve.virtual_token_reserves as u128)
            >= (final_coin as u128) * (bonding_curve.virtual_sol_reserves as u128),
        HoldNowCode::InvalidLiquidity
    );
    emit!(MigrationInventoryBurned {
        mint: mint_key,
        burned_unsold,
        remaining_supply: accts.mint.supply,
    });
    bonding_curve.migrated = true;
    bonding_curve.real_sol_reserves = 0;
    bonding_curve.real_token_reserves = 0;

    emit! {
        MovedRaydiumEvent {
            bonding_curve: bonding_curve.key(),
            mint: accts.mint.key(),
            token: coin_amount,
            sol: total_pool_sol,
        }
    }

    Ok(())
}

/// Separate event preserves the historical MovedRaydiumEvent ABI for replay.
#[event]
pub struct MigrationInventoryBurned {
    pub mint: Pubkey,
    pub burned_unsold: u64,
    pub remaining_supply: u64,
}

use crate::{
    constants::{BONDING_CURVE, GLOBAL_STATE_SEED, REWARD_STATE_SEED, REWARD_VAULT_SEED},
    error::*,
    events::*,
    state::{BondingCurve, Global},
};
use anchor_lang::prelude::*;
use anchor_spl::token::{transfer, Mint, Token, TokenAccount, Transfer};
use solana_program::program::invoke;
use solana_system_interface::instruction as system_instruction;

#[derive(Accounts)]
pub struct Create<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        seeds = [GLOBAL_STATE_SEED],
        bump
    )]
    pub global: Box<Account<'info, Global>>,

    #[account(mut)]
    pub mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [BONDING_CURVE, mint.key().as_ref()],
        bump,
        constraint = bonding_curve.creator == user.key() @ HoldNowCode::NotAuthorized
    )]
    pub bonding_curve: Box<Account<'info, BondingCurve>>,

    #[account(mut, seeds = [crate::VAULT_SEED, mint.key().as_ref()], bump,
        token::mint = mint)]
    pub associated_bonding_curve: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = mint,
        constraint = associated_user_account.owner == user.key() @ HoldNowCode::NotAuthorized
    )]
    pub associated_user_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: Address checked against global state
    #[account(mut)]
    pub fee_recipient: AccountInfo<'info>,

    #[account(
        init,
        payer = user,
        seeds = [REWARD_STATE_SEED, mint.key().as_ref()],
        bump,
        space = 8
    )]
    /// CHECK: PDA signer, no data read.
    pub reward_recipient: AccountInfo<'info>,

    #[account(
        init,
        payer = user,
        seeds = [REWARD_VAULT_SEED, mint.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = reward_recipient,
    )]
    pub associated_reward_recipient: Box<Account<'info, TokenAccount>>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

pub fn create(
    ctx: Context<Create>,
    number_stage: u64, // 0 means infinite stages
    stage_duration: u64,
    claim_duration: u64,
    sell_tax_min: u8,
    sell_tax_max: u8,
    sell_tax_decay: u8,
    token_pool_destination: u8,
    amount: u64,
    airdrop_amount: u64,
    merkle_root: Pubkey,
    minimum_migration_sol: u64,
    migration_grace_seconds: u64,
) -> Result<()> {
    let global = &ctx.accounts.global;
    let bonding_curve = &mut ctx.accounts.bonding_curve;
    let mint = &ctx.accounts.mint;
    let clock = Clock::get()?;
    global.assert_current()?;
    bonding_curve.assert_current()?;
    require!(
        minimum_migration_sol == global.minimum_migration_sol
            && minimum_migration_sol >= crate::NEW_LAUNCH_MIN_MIGRATION_SOL,
        HoldNowCode::InvalidConfiguration
    );
    require!(
        global.stage_unit_seconds == crate::STAGE_UNIT_SECONDS
            && global.migration_grace_seconds == crate::MIGRATION_GRACE_SECONDS
            && migration_grace_seconds == global.migration_grace_seconds,
        HoldNowCode::InvalidConfiguration
    );

    require!(
        bonding_curve.token_total_supply == 0,
        HoldNowCode::AlreadyInitialized
    );
    require!(
        mint.mint_authority.is_none(),
        HoldNowCode::MitAuthorityEnabled
    );
    require!(
        mint.freeze_authority.is_none(),
        HoldNowCode::FreezeAuthorityEnabled
    );
    require!(
        mint.decimals == crate::TOKEN_DECIMALS && amount == global.token_total_supply,
        HoldNowCode::InvalidSupply
    );
    require!(
        ctx.accounts
            .associated_bonding_curve
            .to_account_info()
            .data_len()
            >= 165,
        HoldNowCode::InvalidAmount
    );
    let needed = BondingCurve::space(number_stage)?;
    require!(
        bonding_curve.to_account_info().data_len() >= needed,
        HoldNowCode::InvalidStageCount
    );
    let total_stage_duration = crate::utils::stage_duration_seconds(stage_duration)?;
    require!(
        sell_tax_min <= sell_tax_max
            && sell_tax_max <= crate::MAX_SELL_TAX_PERCENT
            && sell_tax_decay <= 100
            && token_pool_destination == 3,
        HoldNowCode::InvalidConfiguration
    );
    require!(
        (airdrop_amount == 0) == (merkle_root == Pubkey::default()),
        HoldNowCode::InvalidAirdropAmount
    );
    require!(airdrop_amount < amount, HoldNowCode::InvalidAirdropAmount);
    if bonding_curve.initial_allocation_merkle_root != Pubkey::default() {
        require!(
            bonding_curve.initial_allocation_merkle_root == merkle_root
                && bonding_curve.total_stages as u64 == number_stage,
            HoldNowCode::InvalidConfiguration
        );
    }
    // All the require checks from your original function
    require!(global.initialized == true, HoldNowCode::NotInitialized);
    require!(
        ctx.accounts.fee_recipient.key() == global.fee_recipient,
        HoldNowCode::InValidFeeRecipient
    );
    require!(airdrop_amount <= amount, HoldNowCode::InvalidAirdropAmount);
    require!(mint.supply == amount, HoldNowCode::InvalidAmount);
    require!(
        ctx.accounts.associated_user_account.amount >= amount,
        HoldNowCode::InsufficientFunds
    );

    // Add validation for claim window duration
    const MIN_CLAIM_WINDOW: u64 = 60; // 1 min minimum
    const MAX_CLAIM_WINDOW: u64 = 604800; // 7 days maximum

    require!(
        claim_duration >= MIN_CLAIM_WINDOW,
        HoldNowCode::ClaimWindowTooShort
    );

    require!(
        claim_duration <= MAX_CLAIM_WINDOW,
        HoldNowCode::ClaimWindowTooLong
    );

    // Transfer tokens
    let cpi_ctx = CpiContext::new(
        ctx.accounts.token_program.key(),
        Transfer {
            from: ctx
                .accounts
                .associated_user_account
                .to_account_info()
                .clone(),
            to: ctx
                .accounts
                .associated_bonding_curve
                .to_account_info()
                .clone(),
            authority: ctx.accounts.user.to_account_info().clone(),
        },
    );
    transfer(cpi_ctx, amount - airdrop_amount)?;

    if airdrop_amount > 0 && merkle_root != Pubkey::default() {
        let cpi_ctx_airdrop = CpiContext::new(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx
                    .accounts
                    .associated_user_account
                    .to_account_info()
                    .clone(),
                to: ctx
                    .accounts
                    .associated_reward_recipient
                    .to_account_info()
                    .clone(),
                authority: ctx.accounts.user.to_account_info().clone(),
            },
        );
        transfer(cpi_ctx_airdrop, airdrop_amount)?;

        bonding_curve.initial_allocation_merkle_root = merkle_root;
        bonding_curve.initial_allocation_set_at = clock.unix_timestamp;
        bonding_curve.total_stages = number_stage as u8;
    }

    // Transfer fee
    invoke(
        &system_instruction::transfer(
            &ctx.accounts.user.key(),
            &ctx.accounts.fee_recipient.key(),
            global.create_fee,
        ),
        &[
            ctx.accounts.user.to_account_info().clone(),
            ctx.accounts.fee_recipient.to_account_info().clone(),
            ctx.accounts.system_program.to_account_info().clone(),
        ],
    )?;

    // Configure the bonding curve with all parameters
    bonding_curve.minimum_migration_sol = global.minimum_migration_sol;
    bonding_curve.migration_grace_seconds = global.migration_grace_seconds;

    let (remaining_tokens, adjusted_sol_reserves) = crate::utils::launch_reserves(
        global.initial_virtual_token_reserves,
        global.initial_virtual_sol_reserves,
        airdrop_amount,
    )?;

    // Set adjusted values
    bonding_curve.virtual_token_reserves = remaining_tokens;
    bonding_curve.virtual_sol_reserves = adjusted_sol_reserves;
    bonding_curve.real_token_reserves = amount - airdrop_amount;
    bonding_curve.real_sol_reserves = 0;
    bonding_curve.token_total_supply = mint.supply;
    bonding_curve.complete = false;
    bonding_curve.bonding_curve_start = clock.unix_timestamp;
    bonding_curve.airdrop_amount = airdrop_amount;
    bonding_curve.current_stage = 1;
    bonding_curve.current_stage_complete = false;
    bonding_curve.stage_duration = total_stage_duration as i64;
    bonding_curve.claim_duration = claim_duration as i64;
    bonding_curve.sell_tax_max = sell_tax_max;
    bonding_curve.sell_tax_min = sell_tax_min;
    bonding_curve.sell_tax_decay = sell_tax_decay;
    bonding_curve.token_pool_destination = token_pool_destination;
    bonding_curve.at_start_stage = clock.unix_timestamp;
    bonding_curve.number_stage = number_stage;

    bonding_curve.infinite_mode = false;
    bonding_curve.migrated = false;
    bonding_curve.total_regular_claimed = 0;
    bonding_curve.total_initial_claimed = 0;
    bonding_curve.total_stages = number_stage as u8;
    let vector_size = number_stage as usize + 1;
    bonding_curve.cumulative_merkle_roots = vec![Pubkey::default(); vector_size];
    bonding_curve.stage_completion_times = vec![0; vector_size];
    bonding_curve.stage_reward_pools = vec![0; vector_size];
    bonding_curve.stage_eligible_supply = vec![0; vector_size];
    bonding_curve.total_claimed_per_stage = vec![0; vector_size];
    bonding_curve.stage_settled = vec![false; vector_size];
    bonding_curve.stage_reward_rollovers = vec![0; vector_size];

    msg!(
        "LaunchEvent - Mint: {}, BondingCurve: {}, User: {}",
        ctx.accounts.mint.key(),
        bonding_curve.key(),
        ctx.accounts.user.key()
    );

    // Emit event
    emit! {
        LaunchEvent {
            mint: ctx.accounts.mint.key(),
            user: ctx.accounts.user.key(),
            bonding_curve: bonding_curve.key(),
            token_supply: bonding_curve.token_total_supply,
            reserve_token: bonding_curve.virtual_token_reserves,
            reserve_lamport: bonding_curve.virtual_sol_reserves,
            stages_number: bonding_curve.number_stage,
            infinite_mode: bonding_curve.infinite_mode,
            stage_duration: stage_duration,
            claim_duration: claim_duration,
            sell_tax_min: sell_tax_min ,
            sell_tax_max: sell_tax_max ,
            sell_tax_decay: sell_tax_decay,
            token_pool_destination: token_pool_destination,
            complete : bonding_curve.complete,
            stage_duration_seconds: total_stage_duration,
            minimum_migration_sol: bonding_curve.minimum_migration_sol,
            timestamp: clock.unix_timestamp,
            protocol_version: crate::PROTOCOL_VERSION,
            migration_grace_seconds,
        }
    }

    Ok(())
}

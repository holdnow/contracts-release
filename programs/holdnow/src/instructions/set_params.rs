use crate::{constants::GLOBAL_STATE_SEED, error::*, events::*, state::Global};
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct SetParams<'info> {
    #[account(
        mut,
        seeds = [GLOBAL_STATE_SEED],
        bump
    )]
    pub global: Box<Account<'info, Global>>,

    #[account(mut)]
    pub user: Signer<'info>,
}

pub fn set_params(
    ctx: Context<SetParams>,
    fee_recipient: Pubkey,
    owner_wallet: Pubkey,
    initial_virtual_token_reserves: u64,
    initial_virtual_sol_reserves: u64,
    initial_real_token_reserves: u64,
    token_total_supply: u64,
    fee_basis_points: u64,
    create_fee: u64,
    staging_pool_fee: u64,
    minimum_migration_sol: u64,
) -> Result<()> {
    let global: &mut Box<Account<'_, Global>> = &mut ctx.accounts.global;

    global.assert_current()?;
    require!(
        global.authority == ctx.accounts.user.key(),
        HoldNowCode::NotAuthorized
    );
    require!(
        fee_basis_points <= crate::MAX_FEE_PERCENT,
        HoldNowCode::InvalidFeeBasisPoints
    );
    require!(
        initial_virtual_token_reserves > 0,
        HoldNowCode::InvalidAmount
    );
    require!(initial_virtual_sol_reserves > 0, HoldNowCode::InvalidAmount);
    require!(
        token_total_supply > 0 && token_total_supply <= u64::MAX / crate::MAX_STAGES,
        HoldNowCode::InvalidSupply
    );

    require!(
        staging_pool_fee <= crate::MAX_FEE_PERCENT,
        HoldNowCode::InvalidFeeBasisPoints
    );
    require!(
        fee_recipient != Pubkey::default() && owner_wallet != Pubkey::default(),
        HoldNowCode::InvalidConfiguration
    );
    require!(
        initial_real_token_reserves == token_total_supply
            && initial_virtual_token_reserves >= token_total_supply,
        HoldNowCode::InvalidSupply
    );
    require!(
        minimum_migration_sol >= crate::NEW_LAUNCH_MIN_MIGRATION_SOL,
        HoldNowCode::InvalidConfiguration
    );
    global.minimum_migration_sol = minimum_migration_sol;
    global.fee_recipient = fee_recipient;
    global.owner_wallet = owner_wallet;
    global.initial_virtual_token_reserves = initial_virtual_token_reserves;
    global.initial_virtual_sol_reserves = initial_virtual_sol_reserves;
    global.initial_real_token_reserves = initial_real_token_reserves;
    global.token_total_supply = token_total_supply;
    global.fee_basis_points = fee_basis_points;
    global.create_fee = create_fee;
    global.staging_pool_fee = staging_pool_fee;
    // Log the event details
    msg!(
        "SetParamsEvent  - FeeRecipient: {}, OwnerWallet: {}, TokenReserves: {}, SolReserves: {}, RealTokenReserves: {}, TokenTotalSupply: {}, FeeBasisPoints: {}, CreateFee:{}, StagingPoolFee:{}",
        global.fee_recipient,
        global.owner_wallet ,
        global.initial_virtual_token_reserves,
        global.initial_virtual_sol_reserves ,
        global.initial_real_token_reserves,
        global.token_total_supply,
        global.fee_basis_points,
        global.create_fee,
        global.staging_pool_fee
    );

    emit! {
        SetParamsEvent {
            fee_recipient:  global.fee_recipient,
            owner_wallet: global.owner_wallet,
            initial_virtual_token_reserves: global.initial_virtual_token_reserves,
            initial_virtual_sol_reserves: global.initial_virtual_sol_reserves,
            initial_real_token_reserves: global.initial_real_token_reserves,
            token_total_supply: global.token_total_supply,
            fee_basis_points: global.fee_basis_points,
            create_fee: global.create_fee,
            staging_pool_fee: global.staging_pool_fee,
            minimum_migration_sol: global.minimum_migration_sol,
        }
    }
    Ok(())
}

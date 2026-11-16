use crate::{
    constants::{BONDING_CURVE, GLOBAL_STATE_SEED},
    error::*,
    events::*,
    state::{BondingCurve, Global},
};
use anchor_lang::prelude::*;
use anchor_spl::token::Mint;

#[derive(Accounts)]
pub struct SetInitialAllocations<'info> {
    #[account(
        mut,
        seeds = [GLOBAL_STATE_SEED],
        bump
    )]
    pub global: Box<Account<'info, Global>>,

    #[account(
        mut,
        seeds = [BONDING_CURVE, mint.key().as_ref()],
        bump
    )]
    pub bonding_curve: Account<'info, BondingCurve>,

    pub mint: Box<Account<'info, Mint>>,

    #[account(mut)]
    pub creator: Signer<'info>,

    #[account(
        constraint = backend_wallet.key() == global.owner_wallet.key() @HoldNowCode:: NotAuthorized
    )]
    pub backend_wallet: Signer<'info>,

    pub clock: Sysvar<'info, Clock>,
}

pub fn set_initial_allocations(
    ctx: Context<SetInitialAllocations>,
    merkle_root: Pubkey,
    total_stages: u8,
) -> Result<()> {
    ctx.accounts.global.assert_current()?;
    ctx.accounts.bonding_curve.assert_current()?;
    let a = ctx.accounts;
    let c = &mut a.bonding_curve;
    require_keys_eq!(c.creator, a.creator.key(), HoldNowCode::NotAuthorized);
    require!(
        c.token_total_supply == 0 && c.initial_allocation_merkle_root == Pubkey::default(),
        HoldNowCode::InitialAllocationsAlreadySet
    );
    require!(
        merkle_root != Pubkey::default(),
        HoldNowCode::MerkleRootNotSet
    );
    require!(
        (1..=crate::MAX_STAGES).contains(&(total_stages as u64)),
        HoldNowCode::InvalidStageCount
    );
    c.initial_allocation_merkle_root = merkle_root;
    c.initial_allocation_set_at = a.clock.unix_timestamp;
    c.total_stages = total_stages;
    emit!(InitialAllocationsSet {
        token: a.mint.key(),
        merkle_root,
        total_stages,
        timestamp: a.clock.unix_timestamp
    });
    Ok(())
}

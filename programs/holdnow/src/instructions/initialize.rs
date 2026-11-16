use anchor_lang::prelude::*;

use crate::{constants::GLOBAL_STATE_SEED, error::*, state::Global};
use std::mem::size_of;

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = owner,
        seeds = [GLOBAL_STATE_SEED],
        bump,
        space = 8 + size_of::<Global>()
    )]
    pub global: Box<Account<'info, Global>>,

    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
    initialize_global(&mut ctx.accounts.global, ctx.accounts.owner.key())
}

fn initialize_global(global: &mut Global, owner: Pubkey) -> Result<()> {
    // Build-time authority is generated from the reviewed deployment manifest.
    let expected_owner = crate::deployment::BOOTSTRAP_AUTHORITY;

    require_keys_eq!(owner, expected_owner.key(), HoldNowCode::NotAuthorized);

    require!(global.initialized == false, HoldNowCode::AlreadyInitialized);

    global.authority = owner;
    global.initialized = true;
    global.protocol_version = crate::PROTOCOL_VERSION;
    global.stage_unit_seconds = crate::STAGE_UNIT_SECONDS;
    global.minimum_migration_sol = crate::NEW_LAUNCH_MIN_MIGRATION_SOL;
    global.migration_grace_seconds = crate::MIGRATION_GRACE_SECONDS;

    Ok(())
}

#[cfg(test)]
mod audit_unit_tests {
    include!("../../tests/unit/initialize.rs");
}

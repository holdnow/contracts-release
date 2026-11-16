use crate::{
    constants::{BONDING_CURVE, GLOBAL_STATE_SEED, SOL_VAULT_SEED, VAULT_SEED},
    state::{BondingCurve, Global},
};
use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

#[derive(Accounts)]
#[instruction(number_stages: u64)]
pub struct InitializeCurve<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    pub mint: Box<Account<'info, Mint>>,

    #[account(
        init,
        payer = user,
        seeds = [BONDING_CURVE, mint.key().as_ref()],
        bump,
        space = BondingCurve::space(number_stages)?
    )]
    pub bonding_curve: Box<Account<'info, BondingCurve>>,

    #[account(mut, seeds = [SOL_VAULT_SEED, mint.key().as_ref()], bump)]
    pub vault: SystemAccount<'info>,

    #[account(
        init,
        payer = user,
        seeds = [VAULT_SEED, mint.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = vault,
    )]
    pub associated_bonding_curve: Box<Account<'info, TokenAccount>>,

    #[account(
        seeds = [GLOBAL_STATE_SEED],
        bump
    )]
    pub global: Box<Account<'info, Global>>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,

    // Reserving a launch requires possession of the entire immutable supply.
    #[account(token::mint = mint, token::authority = user)]
    pub creator_token_account: Box<Account<'info, TokenAccount>>,
}

pub fn initialize_curve(
    ctx: Context<InitializeCurve>,
    number_stages: u64, // Add parameter
) -> Result<()> {
    ctx.accounts.global.assert_current()?;
    BondingCurve::space(number_stages)?;
    require!(
        ctx.accounts.global.initialized,
        crate::error::HoldNowCode::NotInitialized
    );
    require!(
        ctx.accounts.mint.mint_authority.is_none(),
        crate::error::HoldNowCode::MitAuthorityEnabled
    );
    require!(
        ctx.accounts.mint.freeze_authority.is_none(),
        crate::error::HoldNowCode::FreezeAuthorityEnabled
    );
    require!(
        ctx.accounts.mint.decimals == crate::TOKEN_DECIMALS
            && ctx.accounts.mint.supply == ctx.accounts.global.token_total_supply
            && ctx.accounts.mint.supply > 0,
        crate::error::HoldNowCode::InvalidSupply
    );
    require!(
        ctx.accounts.creator_token_account.amount == ctx.accounts.mint.supply,
        crate::error::HoldNowCode::InsufficientFunds
    );
    // The rent cushion is launch funding, never a trading deposit. Provision only
    // the shortfall so direct clients and prefunded PDAs have identical behavior.
    require!(
        ctx.accounts.vault.data_is_empty(),
        crate::error::HoldNowCode::NotAuthorized
    );
    let shortfall = ctx
        .accounts
        .rent
        .minimum_balance(0)
        .saturating_sub(ctx.accounts.vault.lamports());
    if shortfall > 0 {
        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.key(),
                anchor_lang::system_program::Transfer {
                    from: ctx.accounts.user.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                },
            ),
            shortfall,
        )?;
    }
    // Basic initialization. The main logic will be in the 'create' instruction.
    let bonding_curve = &mut ctx.accounts.bonding_curve;
    bonding_curve.token_mint = ctx.accounts.mint.key();
    bonding_curve.creator = ctx.accounts.user.key();
    bonding_curve.protocol_version = crate::PROTOCOL_VERSION;

    // Initialize vectors as empty - they'll be properly sized in create()
    bonding_curve.cumulative_merkle_roots = Vec::new();
    bonding_curve.stage_completion_times = Vec::new();
    bonding_curve.stage_reward_pools = Vec::new();
    bonding_curve.stage_eligible_supply = Vec::new();
    bonding_curve.total_claimed_per_stage = Vec::new();

    Ok(())
}

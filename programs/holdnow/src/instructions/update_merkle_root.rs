use crate::{
    constants::{BONDING_CURVE, GLOBAL_STATE_SEED, REWARD_STATE_SEED, REWARD_VAULT_SEED},
    error::*,
    state::{BondingCurve, Global},
};
use anchor_lang::prelude::*;
use anchor_spl::token::{burn, Burn, Mint, Token, TokenAccount};

#[derive(Accounts)]
pub struct UpdateMerkleRoot<'info> {
    #[account(
        mut,
        seeds = [BONDING_CURVE, mint.key().as_ref()],
        bump
    )]
    pub bonding_curve: Account<'info, BondingCurve>,

    #[account(mut)]
    pub mint: Account<'info, Mint>,

    #[account(
        seeds = [GLOBAL_STATE_SEED],
        bump,
        has_one = owner_wallet
    )]
    pub global: Account<'info, Global>,

    pub owner_wallet: Signer<'info>,

    pub clock: Sysvar<'info, Clock>,
    /// CHECK: Canonical reward-vault signing authority.
    #[account(seeds = [REWARD_STATE_SEED, mint.key().as_ref()], bump)]
    pub reward_recipient: AccountInfo<'info>,
    #[account(mut, seeds = [REWARD_VAULT_SEED, mint.key().as_ref()], bump,
        token::mint = mint, token::authority = reward_recipient)]
    pub reward_vault: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

pub fn update_merkle_root(
    ctx: Context<UpdateMerkleRoot>,
    stage: u8,
    merkle_root: Pubkey,
    total_eligible_supply: u64,
) -> Result<()> {
    ctx.accounts.global.assert_current()?;
    ctx.accounts.bonding_curve.assert_current()?;
    let c = &mut ctx.accounts.bonding_curve;
    let i = c.stage_index(stage)?;
    require!(
        c.stage_completion_times[i] > 0,
        HoldNowCode::StageNotComplete
    );
    require!(!c.stage_settled[i], HoldNowCode::MerkleRootAlreadySet);
    require!(
        c.current_stage_complete && c.current_stage == stage as u64 && !c.complete && !c.migrated,
        HoldNowCode::InvalidStage
    );
    require!(
        total_eligible_supply <= c.token_total_supply,
        HoldNowCode::InvalidSupply
    );
    if total_eligible_supply == 0 {
        let unallocated = c.stage_reward_pools[i - 1];
        if unallocated > 0 {
            if (stage as u64) < c.number_stage {
                c.stage_reward_pools[i] = c.stage_reward_pools[i]
                    .checked_add(unallocated)
                    .ok_or(HoldNowCode::MathOverflow)?;
                c.stage_reward_rollovers[i] = unallocated;
            } else {
                // Burn only this stage's unallocated budget, never earlier regular
                // entitlements or initial vesting. Claims never expire.
                let regular_budget = c.stage_reward_pools.iter().try_fold(0u64, |sum, n| {
                    sum.checked_add(*n).ok_or(HoldNowCode::MathOverflow)
                })?;
                let outstanding_regular = regular_budget
                    .checked_sub(c.total_regular_claimed)
                    .ok_or(HoldNowCode::RewardBudgetExceeded)?;
                let outstanding_initial = c
                    .airdrop_amount
                    .checked_sub(c.total_initial_claimed)
                    .ok_or(HoldNowCode::RewardBudgetExceeded)?;
                require!(
                    outstanding_regular >= unallocated,
                    HoldNowCode::RewardBudgetExceeded
                );
                require!(
                    ctx.accounts.reward_vault.amount
                        >= outstanding_regular
                            .checked_add(outstanding_initial)
                            .ok_or(HoldNowCode::MathOverflow)?,
                    HoldNowCode::InsufficientRewardPool
                );
                let mint = ctx.accounts.mint.key();
                let seeds: &[&[u8]] = &[
                    REWARD_STATE_SEED,
                    mint.as_ref(),
                    &[ctx.bumps.reward_recipient],
                ];
                burn(
                    CpiContext::new_with_signer(
                        ctx.accounts.token_program.key(),
                        Burn {
                            mint: ctx.accounts.mint.to_account_info(),
                            from: ctx.accounts.reward_vault.to_account_info(),
                            authority: ctx.accounts.reward_recipient.to_account_info(),
                        },
                        &[seeds],
                    ),
                    unallocated,
                )?;
                c.total_unallocated_burned = c
                    .total_unallocated_burned
                    .checked_add(unallocated)
                    .ok_or(HoldNowCode::MathOverflow)?;
            }
            c.stage_reward_pools[i - 1] = 0;
        }
    } else {
        require!(
            merkle_root != Pubkey::default(),
            HoldNowCode::MerkleRootNotSet
        );
    }
    c.cumulative_merkle_roots[i] = merkle_root;
    c.stage_eligible_supply[i] = total_eligible_supply;
    c.stage_settled[i] = true;
    emit!(crate::events::StageRewardsSettled {
        mint: ctx.accounts.mint.key(),
        stage,
        merkle_root,
        eligible_supply: total_eligible_supply,
        rolled_forward: c.stage_reward_rollovers[i],
        total_unallocated_burned: c.total_unallocated_burned,
    });
    Ok(())
}

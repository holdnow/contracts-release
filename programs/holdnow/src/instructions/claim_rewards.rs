use anchor_lang::prelude::*;
use anchor_spl::token::{transfer, Mint, Token, TokenAccount, Transfer};
use solana_program::keccak;

use crate::{
    constants::{
        BONDING_CURVE, REWARD_STATE_SEED, REWARD_VAULT_SEED, USER_CLAIM_STATE_SEED,
        USER_VESTING_STATE_SEED,
    },
    error::*,
    events::*,
    state::{BondingCurve, ClaimProof, UserClaimState, UserVestingState},
};

#[derive(Accounts)]
#[instruction(stage: u8)]
pub struct ClaimRewards<'info> {
    pub mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [BONDING_CURVE, mint.key().as_ref()],
        bump
    )]
    pub bonding_curve: Box<Account<'info, BondingCurve>>,

    #[account(
        init_if_needed,
        payer = user,
        space = 8 + std::mem::size_of::<UserClaimState>(),
        seeds = [
            USER_CLAIM_STATE_SEED,
            mint.key().as_ref(),
            user.key().as_ref(),
        ],
        bump
    )]
    pub user_claim_state: Box<Account<'info, UserClaimState>>,

    #[account(
        init_if_needed,
        payer = user,
        space = 8 + std::mem::size_of::<UserVestingState>(),
        seeds = [
            USER_VESTING_STATE_SEED,
            mint.key().as_ref(),
            user.key().as_ref(),
        ],
        bump
    )]
    pub user_vesting_state: Box<Account<'info, UserVestingState>>,

    #[account(
        seeds = [REWARD_STATE_SEED, mint.key().as_ref()],
        bump
    )]
    /// CHECK: PDA for reward state
    pub reward_recipient: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [REWARD_VAULT_SEED, mint.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = reward_recipient,
    )]
    pub reward_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = mint,
        token::authority = user
    )]
    pub user_token_account: Box<Account<'info, TokenAccount>>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub clock: Sysvar<'info, Clock>,
    #[account(mut, close = user,
        seeds = [super::claim_proof::CLAIM_PROOF_SEED, mint.key().as_ref(), user.key().as_ref(), &[proof_buffer.category]], bump)]
    pub proof_buffer: Option<Box<Account<'info, ClaimProof>>>,
}

pub fn claim_rewards(
    ctx: Context<ClaimRewards>,
    stage: u8,
    // Regular rewards
    cumulative_regular_allocation: u64,
    mut regular_proof: Vec<[u8; 32]>,
    // Initial allocation (only if user has one)
    initial_allocation_amount: Option<u64>,
    mut initial_proof: Option<Vec<[u8; 32]>>,
) -> Result<()> {
    let accts: &mut ClaimRewards<'_> = ctx.accounts;
    let clock = &accts.clock;
    if let Some(p) = &accts.proof_buffer {
        require!(
            p.token == accts.mint.key()
                && p.user == accts.user.key()
                && p.stage == stage
                && p.length > 0
                && p.length <= 32
                && p.category <= 1,
            HoldNowCode::InvalidMerkleProof
        );
        let expected_mask = ((1u64 << p.length) - 1) as u32;
        require!(p.written == expected_mask, HoldNowCode::InvalidMerkleProof);
        let index = accts.bonding_curve.stage_index(stage)?;
        if p.category == 0 {
            require!(
                regular_proof.is_empty()
                    && cumulative_regular_allocation == p.amount
                    && p.amount > 0
                    && accts.bonding_curve.cumulative_merkle_roots[index] == p.root,
                HoldNowCode::InvalidMerkleProof
            );
            regular_proof = p.nodes[..p.length as usize].to_vec();
        } else {
            require!(
                initial_proof.as_ref().map_or(false, |v| v.is_empty())
                    && initial_allocation_amount == Some(p.amount)
                    && p.amount > 0
                    && accts.bonding_curve.initial_allocation_merkle_root == p.root,
                HoldNowCode::InvalidMerkleProof
            );
            initial_proof = Some(p.nodes[..p.length as usize].to_vec());
        }
    }

    require!(
        regular_proof.len() <= 32 && initial_proof.as_ref().map_or(true, |p| p.len() <= 32),
        HoldNowCode::InvalidMerkleProof
    );
    let stage_index = accts.bonding_curve.stage_index(stage)?;

    require!(
        accts.bonding_curve.stage_completion_times[stage_index] > 0,
        HoldNowCode::StageNotComplete
    );

    let mut total_to_claim = 0u64;
    let mut initial_claim = 0u64;
    let mut regular_claim = 0u64;

    // === Part 1: Handle Initial Allocation Vesting ===
    if let (Some(initial_amount), Some(proof)) = (initial_allocation_amount, initial_proof) {
        // First time: verify and store initial allocation
        if accts.user_vesting_state.initial_allocation == 0 {
            let initial_root = accts.bonding_curve.initial_allocation_merkle_root;
            require!(
                initial_root != Pubkey::default(),
                HoldNowCode::MerkleRootNotSet
            );

            // Verify initial allocation proof
            let leaf = compute_initial_leaf(&accts.user.key(), &accts.mint.key(), initial_amount);

            require!(
                verify_proof(&proof, initial_root, leaf),
                HoldNowCode::InvalidMerkleProof
            );

            accts.user_vesting_state.user = accts.user.key();
            accts.user_vesting_state.token = accts.mint.key();
            accts.user_vesting_state.initial_allocation = initial_amount;
        }

        // Calculate vested amount (proportional to completed stages)
        let total_stages = accts.bonding_curve.total_stages;
        let completed_stages = stage as u64;
        require!(
            total_stages > 0 && total_stages as u64 == accts.bonding_curve.number_stage,
            HoldNowCode::InvalidStageCount
        );
        let vested_amount = u64::try_from(
            (accts.user_vesting_state.initial_allocation as u128)
                .checked_mul(completed_stages.min(total_stages as u64) as u128)
                .ok_or(HoldNowCode::MathOverflow)?
                / total_stages as u128,
        )
        .map_err(|_| error!(HoldNowCode::MathOverflow))?;

        let vested_claimable =
            vested_amount.saturating_sub(accts.user_vesting_state.claimed_from_initial);
        total_to_claim = total_to_claim
            .checked_add(vested_claimable)
            .ok_or(HoldNowCode::MathOverflow)?;

        // Update claimed amount
        accts.user_vesting_state.claimed_from_initial += vested_claimable;
        initial_claim = vested_claimable;
    }

    // === Part 2: Handle Regular Rewards ===
    let regular_root = accts.bonding_curve.cumulative_merkle_roots[stage_index];
    if cumulative_regular_allocation > 0 || !regular_proof.is_empty() {
        require!(
            regular_root != Pubkey::default(),
            HoldNowCode::MerkleRootNotSet
        );
        // Verify regular rewards proof
        let leaf = compute_cumulative_leaf(
            &accts.user.key(),
            &accts.mint.key(),
            stage,
            cumulative_regular_allocation,
        );

        require!(
            verify_proof(&regular_proof, regular_root, leaf),
            HoldNowCode::InvalidMerkleProof
        );

        let regular_claimable = cumulative_regular_allocation
            .saturating_sub(accts.user_claim_state.total_claimed_regular);
        total_to_claim = total_to_claim
            .checked_add(regular_claimable)
            .ok_or(HoldNowCode::MathOverflow)?;

        // Update claimed amount
        accts.user_claim_state.total_claimed_regular += regular_claimable;
        regular_claim = regular_claimable;
    }

    require!(total_to_claim > 0, HoldNowCode::NothingToClaim);

    require!(
        accts.reward_vault.amount >= total_to_claim,
        HoldNowCode::InsufficientRewardPool
    );

    let c = &mut accts.bonding_curve;
    let regular_budget =
        c.stage_reward_pools
            .iter()
            .enumerate()
            .try_fold(0u64, |sum, (index, amount)| {
                if c.stage_completion_times
                    .get(index + 1)
                    .copied()
                    .unwrap_or(0)
                    > 0
                {
                    sum.checked_add(*amount)
                        .ok_or_else(|| error!(HoldNowCode::MathOverflow))
                } else {
                    Ok(sum)
                }
            })?;
    c.total_regular_claimed = c
        .total_regular_claimed
        .checked_add(regular_claim)
        .ok_or(HoldNowCode::MathOverflow)?;
    c.total_initial_claimed = c
        .total_initial_claimed
        .checked_add(initial_claim)
        .ok_or(HoldNowCode::MathOverflow)?;
    require!(
        c.total_regular_claimed <= regular_budget && c.total_initial_claimed <= c.airdrop_amount,
        HoldNowCode::RewardBudgetExceeded
    );
    // === Part 3: Transfer Tokens ===
    let mint_key = accts.mint.key();
    let (_, bump) =
        Pubkey::find_program_address(&[REWARD_STATE_SEED, mint_key.as_ref()], ctx.program_id);

    let signer_seeds = &[REWARD_STATE_SEED, mint_key.as_ref(), &[bump]];

    let cpi_ctx = CpiContext::new(
        accts.token_program.key(),
        Transfer {
            from: accts.reward_vault.to_account_info(),
            to: accts.user_token_account.to_account_info(),
            authority: accts.reward_recipient.to_account_info(),
        },
    );

    transfer(cpi_ctx.with_signer(&[signer_seeds]), total_to_claim)?;

    // === Part 4: Update State ===
    accts.user_claim_state.user = accts.user.key();
    accts.user_claim_state.token = accts.mint.key();
    accts.user_claim_state.last_claimed_stage = stage;
    accts.user_claim_state.claimed_at = clock.unix_timestamp;
    accts.user_claim_state.total_claimed_vested = accts.user_vesting_state.claimed_from_initial;

    accts.bonding_curve.total_claimed_per_stage[stage_index] =
        accts.bonding_curve.total_claimed_per_stage[stage_index]
            .checked_add(total_to_claim)
            .ok_or(HoldNowCode::MathOverflow)?;

    emit!(RewardsClaimed {
        user: accts.user.key(),
        token: accts.mint.key(),
        stage,
        amount: total_to_claim,
        initial_claim,
        regular_claim,
        timestamp: clock.unix_timestamp,
    });

    Ok(())
}

// Helper functions
fn compute_cumulative_leaf(user: &Pubkey, token: &Pubkey, stage: u8, amount: u64) -> [u8; 32] {
    let mut data = Vec::new();
    data.extend_from_slice(&user.to_bytes());
    data.extend_from_slice(&token.to_bytes());
    data.push(stage);
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(b"CUMULATIVE");
    keccak::hash(&data).to_bytes()
}

fn compute_initial_leaf(user: &Pubkey, token: &Pubkey, amount: u64) -> [u8; 32] {
    let mut data = Vec::new();
    data.extend_from_slice(&user.to_bytes());
    data.extend_from_slice(&token.to_bytes());
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(b"INITIAL");
    keccak::hash(&data).to_bytes()
}

fn verify_proof(proof: &[[u8; 32]], root: Pubkey, leaf: [u8; 32]) -> bool {
    let mut computed_hash = leaf;

    for proof_element in proof.iter() {
        computed_hash = if computed_hash <= *proof_element {
            keccak::hash(&[&computed_hash[..], &proof_element[..]].concat()).to_bytes()
        } else {
            keccak::hash(&[&proof_element[..], &computed_hash[..]].concat()).to_bytes()
        };
    }

    computed_hash == root.to_bytes()
}
#[cfg(test)]
mod audit_unit_tests {
    include!("../../tests/unit/claim.rs");
}

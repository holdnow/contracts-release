use crate::{error::HoldNowCode, BondingCurve, ClaimProof, BONDING_CURVE};
use anchor_lang::prelude::*;
use anchor_spl::token::Mint;
pub const CLAIM_PROOF_SEED: &[u8] = b"claim_proof";

#[derive(Accounts)]
#[instruction(stage: u8, category: u8)]
pub struct WriteClaimProof<'info> {
    pub mint: Account<'info, Mint>,
    #[account(seeds = [BONDING_CURVE, mint.key().as_ref()], bump)]
    pub bonding_curve: Box<Account<'info, BondingCurve>>,
    #[account(init_if_needed, payer = user, space = ClaimProof::SPACE,
        seeds = [CLAIM_PROOF_SEED, mint.key().as_ref(), user.key().as_ref(), &[category]], bump)]
    pub proof_buffer: Box<Account<'info, ClaimProof>>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

pub fn write_claim_proof(
    ctx: Context<WriteClaimProof>,
    stage: u8,
    category: u8,
    amount: u64,
    length: u8,
    offset: u8,
    nodes: Vec<[u8; 32]>,
) -> Result<()> {
    let a = ctx.accounts;
    let index = a.bonding_curve.stage_index(stage)?;
    require!(
        category <= 1
            && amount > 0
            && length > 0
            && length <= 32
            && !nodes.is_empty()
            && nodes.len() <= 16
            && (offset as usize + nodes.len()) <= length as usize,
        HoldNowCode::InvalidMerkleProof
    );
    let root = if category == 0 {
        require!(
            a.bonding_curve.stage_completion_times[index] > 0,
            HoldNowCode::StageNotComplete
        );
        a.bonding_curve.cumulative_merkle_roots[index]
    } else {
        a.bonding_curve.initial_allocation_merkle_root
    };
    require!(root != Pubkey::default(), HoldNowCode::MerkleRootNotSet);
    let p = &mut a.proof_buffer;
    if offset == 0 {
        // Only the beneficiary can replace an incomplete proof. Old tails are invalidated.
        p.token = a.mint.key();
        p.user = a.user.key();
        p.stage = stage;
        p.category = category;
        p.amount = amount;
        p.root = root;
        p.length = length;
        p.written = 0;
        p.nodes = [[0; 32]; 32];
    }
    require!(
        p.token == a.mint.key()
            && p.user == a.user.key()
            && p.stage == stage
            && p.category == category
            && p.amount == amount
            && p.root == root
            && p.length == length,
        HoldNowCode::InvalidMerkleProof
    );
    for (i, node) in nodes.into_iter().enumerate() {
        let index = offset as usize + i;
        p.nodes[index] = node;
        p.written |= 1u32 << index;
    }
    Ok(())
}

#[derive(Accounts)]
#[instruction(category: u8)]
pub struct CloseClaimProof<'info> {
    pub mint: Account<'info, Mint>,
    #[account(mut, close = user,
        seeds = [CLAIM_PROOF_SEED, mint.key().as_ref(), user.key().as_ref(), &[category]], bump)]
    pub proof_buffer: Box<Account<'info, ClaimProof>>,
    #[account(mut)]
    pub user: Signer<'info>,
}

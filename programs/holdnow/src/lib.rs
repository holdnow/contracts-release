pub mod constants;
pub mod deployment;
pub mod error;
pub mod events;
pub mod instructions;
mod raydium;
pub mod state;
pub mod utils;

use anchor_lang::prelude::*;

pub use constants::*;
pub use events::*;
use instructions::*;
pub use state::*;

declare_id!("BenqJ8z7KVZ3Y5JvrUAh7Z7koa5c91GsoqbT8kPjV7Xr");

// Stable links keep contact and policy updates independent of program upgrades.
#[cfg(not(feature = "no-entrypoint"))]
solana_security_txt::security_txt! {
    name: "Hold.now",
    project_url: "https://hold.now",
    contacts: "link:https://github.com/holdnow/contracts-release/security/advisories/new",
    policy: "https://github.com/holdnow/contracts-release/blob/main/SECURITY.md",
    preferred_languages: "en",
    source_code: "https://github.com/holdnow/contracts-release"
}

#[program]
pub mod holdnow {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        instructions::initialize(ctx)
    }

    pub fn initialize_curve(ctx: Context<InitializeCurve>, number_stages: u64) -> Result<()> {
        instructions::initialize_curve(ctx, number_stages)
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
        instructions::set_params(
            ctx,
            fee_recipient,
            owner_wallet,
            initial_virtual_token_reserves,
            initial_virtual_sol_reserves,
            initial_real_token_reserves,
            token_total_supply,
            fee_basis_points,
            create_fee,
            staging_pool_fee,
            minimum_migration_sol,
        )
    }

    pub fn create(
        ctx: Context<Create>,
        number_stage: u64,
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
        instructions::create(
            ctx,
            number_stage,
            stage_duration,
            claim_duration,
            sell_tax_min,
            sell_tax_max,
            sell_tax_decay,
            token_pool_destination,
            amount,
            airdrop_amount,
            merkle_root,
            minimum_migration_sol,
            migration_grace_seconds,
        )
    }

    pub fn buy(ctx: Context<Buy>, sol_amount: u64, min_tokens: u64) -> Result<()> {
        instructions::buy(ctx, sol_amount, min_tokens)
    }

    pub fn sell(ctx: Context<Sell>, token_amount: u64, min_sol_amount: u64) -> Result<()> {
        instructions::sell(ctx, token_amount, min_sol_amount)
    }

    pub fn stage_completed(ctx: Context<StageCompleted>) -> Result<()> {
        instructions::stage_completed(ctx)
    }

    pub fn activate_redemption(ctx: Context<ActivateRedemption>) -> Result<()> {
        instructions::activate_redemption(ctx)
    }

    pub fn redeem(ctx: Context<Redeem>, token_amount: u64, min_sol: u64) -> Result<()> {
        instructions::redeem(ctx, token_amount, min_sol)
    }

    pub fn move_raydium(ctx: Context<MoveRaydium>, nonce: u8, fee_bps: u16) -> Result<()> {
        instructions::move_raydium(ctx, nonce, fee_bps)
    }

    // ============================================
    // NEW MERKLE REWARD INSTRUCTIONS
    // ============================================

    /// Set initial allocations for vested rewards
    /// This is called once after token creation to set up initial allocation merkle root
    pub fn set_initial_allocations(
        ctx: Context<SetInitialAllocations>,
        merkle_root: Pubkey,
        total_stages: u8,
    ) -> Result<()> {
        instructions::set_initial_allocations::set_initial_allocations(
            ctx,
            merkle_root,
            total_stages,
        )
    }

    /// Update merkle root for a completed stage
    /// This is called after calculating rewards when a stage completes
    pub fn update_merkle_root(
        ctx: Context<UpdateMerkleRoot>,
        stage: u8,
        merkle_root: Pubkey,
        total_eligible_supply: u64,
    ) -> Result<()> {
        instructions::update_merkle_root::update_merkle_root(
            ctx,
            stage,
            merkle_root,
            total_eligible_supply,
        )
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
        instructions::write_claim_proof(ctx, stage, category, amount, length, offset, nodes)
    }

    pub fn close_claim_proof(_ctx: Context<CloseClaimProof>, _category: u8) -> Result<()> {
        Ok(())
    }

    /// Claim rewards using merkle proof
    /// Users call this to claim their regular and vested rewards
    pub fn claim_rewards(
        ctx: Context<ClaimRewards>,
        stage: u8,
        // Regular rewards
        cumulative_regular_allocation: u64,
        regular_proof: Vec<[u8; 32]>,
        // Initial allocation (vested rewards) - optional
        initial_allocation_amount: Option<u64>,
        initial_proof: Option<Vec<[u8; 32]>>,
    ) -> Result<()> {
        instructions::claim_rewards::claim_rewards(
            ctx,
            stage,
            cumulative_regular_allocation,
            regular_proof,
            initial_allocation_amount,
            initial_proof,
        )
    }
}

#[cfg(test)]
mod test_support;

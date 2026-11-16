use anchor_lang::prelude::*;

#[event]
pub struct LaunchEvent {
    pub mint: Pubkey,
    pub user: Pubkey,
    pub bonding_curve: Pubkey,
    pub token_supply: u64,
    pub reserve_token: u64,
    pub reserve_lamport: u64,
    pub stages_number: u64,
    pub infinite_mode: bool,
    pub stage_duration: u64,
    pub claim_duration: u64,
    pub sell_tax_min: u8,
    pub sell_tax_max: u8,
    pub sell_tax_decay: u8,
    pub token_pool_destination: u8,
    pub complete: bool,
    pub stage_duration_seconds: u64,
    pub minimum_migration_sol: u64,
    pub timestamp: i64,
    pub protocol_version: u8,
    pub migration_grace_seconds: u64,
}

#[event]
pub struct SwapEvent {
    pub stage: u64,
    pub mint: Pubkey,
    pub user: Pubkey,
    pub bonding_curve: Pubkey,
    pub reward_cap_addr: Pubkey,
    pub sol_amount: u64,
    pub token_amount: u64,
    pub reward_amount: u64,
    pub is_buy: bool,
    pub token_supply: u64,
    pub timestamp: i64,
    pub reserve_lamport: u64,
    pub reserve_token: u64,
    pub fee_amount: u64,
}

#[event]
pub struct CompleteEvent {
    pub user: Pubkey,
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct SetParamsEvent {
    pub fee_recipient: Pubkey,
    pub owner_wallet: Pubkey,
    pub initial_virtual_token_reserves: u64,
    pub initial_virtual_sol_reserves: u64,
    pub initial_real_token_reserves: u64,
    pub token_total_supply: u64,
    pub fee_basis_points: u64,
    pub create_fee: u64,
    pub staging_pool_fee: u64,
    pub minimum_migration_sol: u64,
}

#[event]
pub struct MoveStageEvent {
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub user: Pubkey,
    pub current_stage: u64,
    pub reserve_token: u64,
    pub reserve_lamport: u64,
    pub timestamp: i64,
}

#[event]
pub struct CompleteStageEvent {
    pub user: Pubkey,
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub current_stage: u64,
    pub timestamp: i64,
}

#[event]
pub struct MovedRaydiumEvent {
    pub bonding_curve: Pubkey,
    pub mint: Pubkey,
    pub token: u64,
    pub sol: u64,
}

#[event]
pub struct ClaimStageEvent {
    pub mint: Pubkey,
    pub user: Pubkey,
    pub bonding_curve: Pubkey,
    pub reward_amount: u64,
    pub current_stage: u64,
    pub reserve_token: u64,
    pub reserve_lamport: u64,
    pub event_type: String,
}

#[event]
pub struct InitialAllocationsSet {
    pub token: Pubkey,
    pub merkle_root: Pubkey,
    pub total_stages: u8,
    pub timestamp: i64,
}

#[event]
pub struct RewardsClaimed {
    pub user: Pubkey,
    pub token: Pubkey,
    pub stage: u8,
    pub amount: u64,
    pub initial_claim: u64,
    pub regular_claim: u64,
    pub timestamp: i64,
}

#[event]
pub struct StageRewardsSettled {
    pub mint: Pubkey,
    pub stage: u8,
    pub merkle_root: Pubkey,
    pub eligible_supply: u64,
    pub rolled_forward: u64,
    pub total_unallocated_burned: u64,
}

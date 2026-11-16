use anchor_lang::prelude::*;

#[account]
pub struct Global {
    pub initialized: bool,
    pub authority: Pubkey,
    pub fee_recipient: Pubkey,
    pub owner_wallet: Pubkey,
    pub initial_virtual_token_reserves: u64,
    pub initial_virtual_sol_reserves: u64,
    pub initial_real_token_reserves: u64,
    pub token_total_supply: u64,
    pub fee_basis_points: u64,
    pub create_fee: u64,
    pub staging_pool_fee: u64,
    pub backend_wallet: Pubkey,
    pub protocol_version: u8,
    /// Deployment profile, fixed at bootstrap; clients verify this before launch.
    pub stage_unit_seconds: u64,
    /// Default for future launches only; copied into each BondingCurve at create.
    pub minimum_migration_sol: u64,
    /// Fixed deployment recovery window; never changed by set_params.
    pub migration_grace_seconds: u64,
}

impl Global {
    pub fn assert_current(&self) -> Result<()> {
        require!(
            self.protocol_version == crate::PROTOCOL_VERSION,
            crate::error::HoldNowCode::UnsupportedVersion
        );
        Ok(())
    }
}
impl BondingCurve {
    pub fn assert_current(&self) -> Result<()> {
        require!(
            self.protocol_version == crate::PROTOCOL_VERSION,
            crate::error::HoldNowCode::UnsupportedVersion
        );
        Ok(())
    }

    // Borsh size includes seven Vec prefixes and explicit settlement/rollover history.
    // The maximum supported layout remains below Anchor's 10 KiB init limit.
    pub fn space(number_stages: u64) -> Result<usize> {
        require!(
            (1..=crate::MAX_STAGES).contains(&number_stages),
            crate::error::HoldNowCode::InvalidStageCount
        );
        Ok(286 + 73 * (number_stages as usize + 1) + 256)
    }
    pub fn stage_index(&self, stage: u8) -> Result<usize> {
        self.assert_current()?;
        let i = stage as usize;
        require!(
            i > 0
                && i <= self.number_stage as usize
                && i < self.cumulative_merkle_roots.len()
                && i < self.stage_completion_times.len()
                && i < self.total_claimed_per_stage.len()
                && i < self.stage_settled.len()
                && i < self.stage_reward_rollovers.len()
                && i < self.stage_reward_pools.len()
                && i < self.stage_eligible_supply.len(),
            crate::error::HoldNowCode::InvalidStage
        );
        Ok(i)
    }
    /// The final completion timestamp is written once, before publication. Neither
    /// publication, delayed advancement, retries nor donations can extend this window.
    pub fn migration_deadline(&self) -> Result<i64> {
        use crate::error::HoldNowCode;
        let stage =
            u8::try_from(self.number_stage).map_err(|_| error!(HoldNowCode::InvalidStage))?;
        let index = self.stage_index(stage)?;
        let ended = self.stage_completion_times[index];
        require!(ended > 0, HoldNowCode::BondingCurveNotComplete);
        require!(
            self.claim_duration > 0 && self.migration_grace_seconds > 0,
            HoldNowCode::InvalidConfiguration
        );
        let grace = i64::try_from(self.migration_grace_seconds)
            .map_err(|_| error!(HoldNowCode::MathOverflow))?;
        ended
            .checked_add(self.claim_duration)
            .and_then(|n| n.checked_add(grace))
            .ok_or_else(|| error!(HoldNowCode::MathOverflow))
    }

    pub fn assert_migration_open(&self, now: i64) -> Result<()> {
        use crate::error::HoldNowCode;
        self.assert_current()?;
        require!(!self.migrated, HoldNowCode::AlreadyMigrated);
        require!(!self.redemption_mode, HoldNowCode::RedemptionActive);
        require!(self.complete, HoldNowCode::BondingCurveNotComplete);
        require!(
            now < self.migration_deadline()?,
            HoldNowCode::MigrationExpired
        );
        Ok(())
    }

    pub fn assert_trading(&self, now: i64) -> Result<usize> {
        use crate::error::HoldNowCode;
        require!(self.token_total_supply > 0, HoldNowCode::LaunchNotCreated);
        require!(
            !self.complete && !self.migrated,
            HoldNowCode::BondingCurveComplete
        );
        require!(
            !self.current_stage_complete,
            HoldNowCode::CurrentStageComplete
        );
        let i = self.stage_index(
            u8::try_from(self.current_stage).map_err(|_| error!(HoldNowCode::InvalidStage))?,
        )?;
        let end = self
            .at_start_stage
            .checked_add(self.stage_duration)
            .ok_or(HoldNowCode::MathOverflow)?;
        require!(
            now >= self.at_start_stage && now < end,
            HoldNowCode::StageExpired
        );
        require!(
            i - 1 < self.stage_reward_pools.len(),
            HoldNowCode::InvalidStage
        );
        Ok(i - 1)
    }
}

#[account]
pub struct BondingCurve {
    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub real_sol_reserves: u64,
    pub token_total_supply: u64,
    pub token_mint: Pubkey,
    pub creator: Pubkey,
    pub complete: bool,
    pub bonding_curve_start: i64,

    pub airdrop_amount: u64,
    pub current_stage: u64,
    pub current_stage_complete: bool,
    pub number_stage: u64,
    pub stage_duration: i64,
    pub claim_duration: i64,
    pub sell_tax_max: u8,
    pub sell_tax_min: u8,
    pub sell_tax_decay: u8,
    pub token_pool_destination: u8,
    pub at_start_stage: i64,

    // Merkle roots for cumulative allocations (regular rewards)
    pub cumulative_merkle_roots: Vec<Pubkey>, // Was: [Pubkey; 5]
    pub stage_completion_times: Vec<i64>,     // Was: [i64; 5] 40 bytes

    // NEW: Track rewards collected per stage
    pub stage_reward_pools: Vec<u64>, // Was: [u64; 5] Rewards collected in each stage
    pub stage_eligible_supply: Vec<u64>, // Was: [u64; 5] Total eligible supply at stage end

    // Tracking
    pub total_claimed_per_stage: Vec<u64>, // Was: [u64; 5] 40 bytes

    // Merkle roots for initial allocations (vested rewards)
    pub initial_allocation_merkle_root: Pubkey,
    pub initial_allocation_set_at: i64,
    pub total_stages: u8, // Number of stages for this token

    // ADD: New field to indicate infinite mode
    pub infinite_mode: bool, // true = never completes
    // Appended fields. Existing Borsh field offsets are preserved; legacy live
    // launches still need a reviewed migration because old accounting differs.
    pub total_regular_claimed: u64,
    pub total_initial_claimed: u64,
    pub migrated: bool,
    pub protocol_version: u8,
    // One-based settlement markers distinguish an explicit empty publication from no publication.
    pub stage_settled: Vec<bool>,
    // One-based amounts moved from an empty-recipient stage into its successor's budget.
    pub stage_reward_rollovers: Vec<u64>,
    pub total_unallocated_burned: u64,
    /// Irreversible pro-rata exit; mutually exclusive with AMM migration.
    pub redemption_mode: bool,
    /// Immutable net quote-reserve requirement accepted at launch.
    pub minimum_migration_sol: u64,
    /// Fixed deployment recovery window; never changed by set_params.
    pub migration_grace_seconds: u64,
}

// User's claim tracking
#[account]
pub struct UserClaimState {
    pub user: Pubkey,               // 32 bytes
    pub token: Pubkey,              // 32 bytes
    pub total_claimed_regular: u64, // 8 bytes
    pub total_claimed_vested: u64,  // 8 bytes
    pub last_claimed_stage: u8,     // 1 byte
    pub claimed_at: i64,            // 8 bytes
    pub bump: u8,                   // 1 byte
}

// User's vesting tracking (for initial allocations)
#[account]
pub struct UserVestingState {
    pub user: Pubkey,              // 32 bytes
    pub token: Pubkey,             // 32 bytes
    pub initial_allocation: u64,   // 8 bytes
    pub claimed_from_initial: u64, // 8 bytes
    pub bump: u8,                  // 1 byte
}
#[cfg(test)]
mod audit_unit_tests {
    include!("../../tests/unit/state.rs");
}

/// Authoritative seller exclusion. No off-chain event delivery is needed to
/// determine whether this wallet sold during the currently paused stage.
#[account]
pub struct UserSaleState {
    pub token: Pubkey,
    pub user: Pubkey,
    pub last_sold_stage: u8,
    pub protocol_version: u8,
}
impl UserSaleState {
    pub const SPACE: usize = 8 + 32 + 32 + 1 + 1;
}

#[account]
pub struct ClaimProof {
    pub token: Pubkey,
    pub user: Pubkey,
    pub root: Pubkey,
    pub stage: u8,
    pub category: u8,
    pub length: u8,
    pub amount: u64,
    pub written: u32,
    pub nodes: [[u8; 32]; 32],
}
impl ClaimProof {
    pub const SPACE: usize = 8 + 96 + 3 + 8 + 4 + 1024;
}

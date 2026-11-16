use anchor_lang::prelude::*;

#[constant]
pub const GLOBAL_STATE_SEED: &[u8] = b"GLOBAL-STATE-SEED";
pub const MINT_SEED: &[u8] = b"mint";
pub const BONDING_CURVE: &[u8] = b"BONDING-CURVE";
pub const VAULT_SEED: &[u8] = b"VAULT-SEED";
pub const SOL_VAULT_SEED: &[u8] = b"SOL-VAULT-SEED";
pub const REWARD_STATE_SEED: &[u8] = b"REWARD-STATE-SEED";
pub const REWARD_VAULT_SEED: &[u8] = b"REWARD-VAULT-SEED";
pub const METADATA: &[u8] = b"config";

pub const PRECISION: u128 = 1_000_000_000; // 1e9 for price calculations
pub const BASIS_POINTS_DIVISOR: u64 = 10_000; // 100% = 10,000 basis points
pub const PER_MILLE_DIVISOR: u64 = 1_000; // 100% = 1,000 per mille

pub const USER_CLAIM_STATE_SEED: &[u8] = b"user_claim_state";
pub const USER_VESTING_STATE_SEED: &[u8] = b"user_vesting_state";
pub const CLAIM_RECORD_SEED: &[u8] = b"claim_record";
// Public API fee fields retain their legacy percentage units (1 = 1%).
pub const PERCENT_DIVISOR: u64 = 100;
pub const MAX_FEE_PERCENT: u64 = 10;
pub const MAX_SELL_TAX_PERCENT: u8 = 98;
pub const MAX_STAGES: u64 = 100;
pub const MAX_STAGE_DAYS: u64 = 365;
pub const TOKEN_DECIMALS: u8 = 6;
pub const MIGRATION_FEE_BPS: u16 = 100;
#[cfg(not(feature = "devnet"))]
pub const AMM_PROGRAM: Pubkey =
    solana_program::pubkey!("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");
#[cfg(feature = "devnet")]
pub const AMM_PROGRAM: Pubkey =
    solana_program::pubkey!("DRaya7Kj3aMWQSy19kSjvmuwq9docCHofyP9kanQGaav");
#[cfg(not(feature = "devnet"))]
pub const MARKET_PROGRAM: Pubkey =
    solana_program::pubkey!("srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX");
#[cfg(feature = "devnet")]
pub const MARKET_PROGRAM: Pubkey =
    solana_program::pubkey!("EoTcMgcDRTJVZDMZWBoU6rhYHZfkNTVEAfz3uUJRcYGj");
#[cfg(not(feature = "devnet"))]
pub const AMM_FEE_DESTINATION: Pubkey =
    solana_program::pubkey!("7YttLkHDoNj9wyDur5pM1ejNaAvT9X4eqaYcHQqtj2G5");
#[cfg(feature = "devnet")]
pub const AMM_FEE_DESTINATION: Pubkey =
    solana_program::pubkey!("9y8ENuuZ3b19quffx9hQvRVygG5ky6snHfRvGpuSfeJy");

pub const PROTOCOL_VERSION: u8 = 5;
/// Immutable recovery term copied into every new launch.
#[cfg(not(feature = "devnet"))]
pub const MIGRATION_GRACE_SECONDS: u64 = 21_600;
#[cfg(feature = "devnet")]
pub const MIGRATION_GRACE_SECONDS: u64 = 600;
/// A production stage unit remains one day; the devnet build compresses it to two minutes.
#[cfg(not(feature = "devnet"))]
pub const STAGE_UNIT_SECONDS: u64 = 86_400;
#[cfg(feature = "devnet")]
pub const STAGE_UNIT_SECONDS: u64 = 120;
/// Bound actual elapsed time equally in both profiles, while preserving the
/// existing create argument's profile-specific unit semantics.
pub const MAX_STAGE_UNITS: u64 = MAX_STAGE_DAYS * 86_400 / STAGE_UNIT_SECONDS;
/// Economic quote-reserve floor, independent of Raydium's technical LP minimum.
// Legacy launches retain their recorded minimum when the program is upgraded.
pub const MIN_MIGRATION_SOL: u64 = 1_000_000_000;
// These floors apply only to initialization, parameter updates and NEW launches.
#[cfg(not(feature = "devnet"))]
pub const NEW_LAUNCH_MIN_MIGRATION_SOL: u64 = 30_000_000_000;
#[cfg(feature = "devnet")]
pub const NEW_LAUNCH_MIN_MIGRATION_SOL: u64 = 1_000_000_000;
pub const USER_SALE_STATE_SEED: &[u8] = b"user_sale_state";

// Fresh AMM v4 creation: LP mint, three token accounts, pool state, target orders.
// OpenOrders is not created by the supported Raydium Initialize2 deployments.
pub const MIGRATION_ACCOUNT_SIZES: [usize; 6] = [82, 165, 165, 165, 752, 2208];

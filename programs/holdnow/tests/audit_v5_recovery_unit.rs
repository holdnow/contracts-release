use anchor_lang::AccountDeserialize;
use holdnow::{BondingCurve, PROTOCOL_VERSION};
use proptest::prelude::*;
fn curve() -> BondingCurve {
    let bytes = vec![0u8; 2048];
    let mut c = BondingCurve::try_deserialize_unchecked(&mut bytes.as_slice()).unwrap();
    c.protocol_version = PROTOCOL_VERSION;
    c.number_stage = 1;
    c.complete = true;
    c.cumulative_merkle_roots = vec![Default::default(); 2];
    c.stage_completion_times = vec![0, 1000];
    c.total_claimed_per_stage = vec![0; 2];
    c.stage_settled = vec![false, true];
    c.stage_reward_rollovers = vec![0; 2];
    c.stage_reward_pools = vec![0; 2];
    c.stage_eligible_supply = vec![0; 2];
    c.claim_duration = 60;
    c.migration_grace_seconds = 600;
    c
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]
    #[test]
    fn deadline_is_fixed_and_exits_are_exclusive(ended in 1i64..1_000_000_000_000i64, pause in 60i64..604801i64, grace in 1u64..1_000_000u64) {
        let mut c=curve();c.stage_completion_times[1]=ended;c.claim_duration=pause;c.migration_grace_seconds=grace;
        let deadline=ended+pause+grace as i64;
        prop_assert_eq!(c.migration_deadline().unwrap(),deadline);
        prop_assert!(c.assert_migration_open(deadline-1).is_ok());
        prop_assert!(c.assert_migration_open(deadline).is_err());
        c.at_start_stage=deadline+999; c.real_sol_reserves=u64::MAX;
        prop_assert_eq!(c.migration_deadline().unwrap(),deadline);
        c.redemption_mode=true;prop_assert!(c.assert_migration_open(deadline-1).is_err());
    }
}
#[test]
fn invalid_or_overflowing_deadlines_fail_closed() {
    let mut c = curve();
    c.stage_completion_times[1] = i64::MAX;
    assert!(c.migration_deadline().is_err());
    c.stage_completion_times[1] = 0;
    assert!(c.migration_deadline().is_err());
    c.stage_completion_times[1] = 1000;
    c.migration_grace_seconds = u64::MAX;
    assert!(c.migration_deadline().is_err());
    c.migration_grace_seconds = 0;
    assert!(c.migration_deadline().is_err());
    c = curve();
    c.number_stage = u64::MAX;
    assert!(c.migration_deadline().is_err());
    c = curve();
    c.stage_completion_times.clear();
    assert!(c.migration_deadline().is_err());
    c = curve();
    c.complete = false;
    assert!(c.assert_migration_open(1001).is_err());
    c = curve();
    c.migrated = true;
    assert!(c.assert_migration_open(1001).is_err());
}

#[test]
fn compiled_recovery_window_allows_immediate_migration_and_expires_exactly() {
    #[cfg(feature = "devnet")]
    let expected_grace = 600;
    #[cfg(not(feature = "devnet"))]
    let expected_grace = 21_600;
    assert_eq!(holdnow::MIGRATION_GRACE_SECONDS, expected_grace);
    let mut c = curve();
    c.migration_grace_seconds = holdnow::MIGRATION_GRACE_SECONDS;
    let pause_ends = c.stage_completion_times[1] + c.claim_duration;
    let deadline = pause_ends + expected_grace as i64;
    assert_eq!(c.migration_deadline().unwrap(), deadline);
    assert!(c.assert_migration_open(pause_ends).is_ok());
    assert!(c.assert_migration_open(deadline - 1).is_ok());
    assert!(c.assert_migration_open(deadline).is_err());
    assert_eq!(c.claim_duration, 60);
}

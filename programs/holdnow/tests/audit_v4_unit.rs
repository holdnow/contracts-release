use holdnow::{
    instructions::redeem::migration_liquidity, utils::stage_duration_seconds, MAX_STAGE_UNITS,
    MIN_MIGRATION_SOL, STAGE_UNIT_SECONDS,
};
use proptest::prelude::*;
#[test]
fn stage_units_and_invalid_economic_terms_fail_closed() {
    assert_eq!(stage_duration_seconds(1).unwrap(), STAGE_UNIT_SECONDS);
    assert_eq!(
        stage_duration_seconds(365).unwrap(),
        365 * STAGE_UNIT_SECONDS
    );
    assert_eq!(
        stage_duration_seconds(MAX_STAGE_UNITS).unwrap(),
        365 * 86_400
    );
    for n in [0, MAX_STAGE_UNITS + 1, u64::MAX] {
        assert!(stage_duration_seconds(n).is_err());
    }
    assert!(migration_liquidity(1, 1, 0, 6, MIN_MIGRATION_SOL - 1).is_err());
    assert!(
        !migration_liquidity(1, 2_000_000_000, 0, 6, MIN_MIGRATION_SOL)
            .unwrap()
            .2
    ); // LP minimum still applies
    assert!(
        !migration_liquidity(u64::MAX, u64::MAX - 1, u64::MAX, 6, MIN_MIGRATION_SOL)
            .unwrap()
            .2
    );
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]
    #[test]
    fn fee_rounding_has_an_exact_shared_economic_boundary(minimum in 1_000_000_000u64..=u64::MAX/4, reserve in 0u64..=u64::MAX/4) {
        // Solve ceil(99*g/100) >= minimum independently of production's fee subtraction.
        let gross = (((minimum as u128 - 1) * 100) / 99 + 1) as u64;
        let (below, _, funded) = migration_liquidity(u64::MAX, reserve+gross-1, reserve, 6, minimum).unwrap();
        prop_assert_eq!(below,minimum-1); prop_assert!(!funded);
        let (at, fee, funded) = migration_liquidity(u64::MAX, reserve+gross, reserve, 6, minimum).unwrap();
        prop_assert_eq!(at,minimum); prop_assert!(funded); prop_assert_eq!(at+fee,gross);
        prop_assert!(migration_liquidity(u64::MAX,reserve+gross+1,reserve,6,minimum).unwrap().2);
    }
}

use super::*;
use crate::test_support::{account, curve};
use proptest::prelude::*;

#[test]
fn known_quote_and_zero_input() {
    let c = account(curve(1_000_000, 1_000_000));
    assert_eq!(calculate_sol_cost(&c, 1_000_000).unwrap(), 500_000);
    assert_eq!(calculate_sol_cost(&c, 0).unwrap(), 0);
}

#[test]
fn zero_denominator_returns_error() {
    assert!(calculate_sol_cost(&account(curve(0, 1_000)), 0).is_err());
}

#[test]
fn maximal_reserves_do_not_overflow_intermediate_product() {
    let c = account(curve(u64::MAX, u64::MAX));
    assert_eq!(calculate_sol_cost(&c, u64::MAX).unwrap(), u64::MAX / 2);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn quote_is_bounded_monotone_and_preserves_integer_product_rounding(
        t in 1u64..1_000_000_000_000_000,s in 1u64..1_000_000_000_000,
        a in 0u64..1_000_000_000_000_000,b in 0u64..1_000_000_000_000_000) {
        let c=account(curve(t,s));
        let low=calculate_sol_cost(&c,a.min(b)).unwrap();
        let high=calculate_sol_cost(&c,a.max(b)).unwrap();
        prop_assert!(low<=high && high<=s);
        let after_t=t as u128+a.max(b) as u128;
        let before_k=t as u128*s as u128;
        let after_k=(s-high) as u128*after_t;
        prop_assert!(after_k>=before_k);
        prop_assert!(after_k-before_k<after_t);
    }
}

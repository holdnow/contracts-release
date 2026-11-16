use holdnow::instructions::redeem::{migration_liquidity, redemption_output};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]
    #[test]
    fn migration_gate_matches_an_independent_oracle_across_the_entire_u64_domain(
        tokens in any::<u64>(),balance in any::<u64>(),reserved in any::<u64>(),minimum in 1_000_000_000u64..=u64::MAX
    ) {
        let gross=(balance as u128).saturating_sub(reserved as u128);
        // ceil(99*gross/100), independently expressed from fee subtraction.
        let expected=(99*gross+99)/100;
        let lp_boundary=1_000_002_000_001u128;
        let (net,fee,funded)=migration_liquidity(tokens,balance,reserved,6,minimum).unwrap();
        prop_assert_eq!(net as u128,expected);
        prop_assert_eq!(net as u128+fee as u128,gross);
        prop_assert_eq!(funded,expected>=minimum as u128 && tokens as u128*expected>=lp_boundary);
        if funded { prop_assert!(net>=1_000_000_000); }
    }
}
#[test]
fn unattainable_minima_and_dust_redemption_preserve_integer_boundaries() {
    let (net, fee, funded) = migration_liquidity(u64::MAX, u64::MAX, 0, 6, u64::MAX).unwrap();
    assert!(!funded);
    assert_eq!(net.checked_add(fee), Some(u64::MAX));
    assert!(net < u64::MAX);
    let minimum = net;
    assert!(
        migration_liquidity(u64::MAX, u64::MAX, 0, 6, minimum)
            .unwrap()
            .2
    );
    assert!(
        !migration_liquidity(u64::MAX, u64::MAX - 1, 0, 6, minimum)
            .unwrap()
            .2
    );
    assert_eq!(redemption_output(1, 1, u64::MAX).unwrap(), 0);
    assert_eq!(redemption_output(u64::MAX, 1, u64::MAX).unwrap(), 1);
    assert_eq!(
        redemption_output(u64::MAX, u64::MAX, u64::MAX).unwrap(),
        u64::MAX
    );
}

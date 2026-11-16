use holdnow::instructions::redeem::{migration_pool_tokens, price_matched_migration_liquidity};
use proptest::prelude::*;

#[test]
fn reproduces_browser_migration_without_the_ninety_five_percent_price_reset() {
    let inventory = 951_426_843_246_271;
    let pool = migration_pool_tokens(inventory, 1_494_622_212, inventory, 31_531_588_806).unwrap();
    assert_eq!(pool, 45_098_383_775_013);
    assert_eq!(inventory - pool, 906_328_459_471_258);
}
#[test]
fn donations_cap_at_inventory_and_zero_price_is_rejected() {
    assert_eq!(
        migration_pool_tokens(500, u64::MAX, u64::MAX, 1).unwrap(),
        500
    );
    assert_eq!(migration_pool_tokens(500, 0, 500, 30).unwrap(), 0);
    assert!(migration_pool_tokens(500, 1, 500, 0).is_err());
}
#[test]
fn funded_sol_is_not_sufficient_when_price_matching_cannot_mint_minimum_lp() {
    let funded = |virtual_tokens| {
        price_matched_migration_liquidity(
            1_000_000_000_000_000,
            1_010_101_010,
            0,
            6,
            1_000_000_000,
            virtual_tokens,
            1_000_000_000,
        )
        .unwrap()
        .2
    };
    assert!(!funded(1000));
    assert!(funded(1001));
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]
    #[test]
    fn price_does_not_fall_and_no_additional_base_unit_can_be_deposited(
        inventory in any::<u64>(), sol in any::<u64>(),
        virtual_tokens in any::<u64>(), virtual_sol in 1u64..=u64::MAX
    ) {
        let tokens = migration_pool_tokens(inventory, sol, virtual_tokens, virtual_sol).unwrap();
        prop_assert!(tokens <= inventory);
        let backing = sol as u128 * virtual_tokens as u128;
        prop_assert!(tokens as u128 * virtual_sol as u128 <= backing);
        if tokens < inventory {
            prop_assert!((tokens as u128 + 1) * virtual_sol as u128 > backing);
        }
    }
    #[test]
    fn shared_exit_gate_uses_price_matched_tokens_and_exact_fees(
        inventory in any::<u64>(), balance in any::<u64>(), reserved in any::<u64>(),
        virtual_tokens in any::<u64>(), virtual_sol in 1u64..=u64::MAX
    ) {
        let gross = (balance as u128).saturating_sub(reserved as u128);
        let net = (gross * 99 + 99) / 100;
        let matched = (net * virtual_tokens as u128 / virtual_sol as u128).min(inventory as u128);
        let (sol, fee, funded) = price_matched_migration_liquidity(
            inventory, balance, reserved, 6, 1_000_000_000, virtual_tokens, virtual_sol).unwrap();
        prop_assert_eq!(sol as u128, net);
        prop_assert_eq!(sol as u128 + fee as u128, gross);
        prop_assert_eq!(funded, net >= 1_000_000_000 && matched * net >= 1_000_002_000_001);
    }
}

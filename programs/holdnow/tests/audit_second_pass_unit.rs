//! Stateful invariant checks of the production arithmetic, independent of SBPF fixtures.
use holdnow::utils::curve_output;
use proptest::prelude::*;
proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]
    #[test]
    fn arbitrary_trade_sequences_preserve_product_and_cannot_extract_external_sol(
        initial_sol in 1u64..1_000_000_000_000,
        initial_tokens in 1u64..1_000_000_000_000_000,
        capital in 1u64..1_000_000_000_000,
        choices in prop::collection::vec((any::<bool>(), 1u64..=100), 1..49)
    ) {
        let (mut sol, mut token) = (initial_sol, initial_tokens);
        let (mut wallet_sol, mut wallet_token) = (capital, 0u64);
        for (buy, fraction) in choices {
            let product = sol as u128 * token as u128;
            if buy {
                let amount = wallet_sol * fraction / 100;
                let output = curve_output(amount, sol, token).unwrap();
                // Independent reserve formulation uses ceiling division of k/x.
                let next_sol = sol + amount;
                let oracle_remaining = (product + next_sol as u128 - 1) / next_sol as u128;
                prop_assert_eq!(token as u128 - output as u128, oracle_remaining);
                wallet_sol -= amount; sol += amount; token -= output; wallet_token += output;
            } else {
                let amount = wallet_token * fraction / 100;
                let output = curve_output(amount, token, sol).unwrap();
                wallet_token -= amount; token += amount; sol -= output; wallet_sol += output;
            }
            prop_assert_eq!(sol + wallet_sol, initial_sol + capital);
            prop_assert_eq!(token + wallet_token, initial_tokens);
            prop_assert!(sol as u128 * token as u128 >= product);
        }
        let liquidation = curve_output(wallet_token, token, sol).unwrap();
        prop_assert!(wallet_sol + liquidation <= capital);
    }
}

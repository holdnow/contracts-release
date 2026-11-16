//! Independent numerical oracles for the v3 exit mechanism and account storage.
use anchor_lang::AccountSerialize;
use holdnow::{
    instructions::redeem::{has_minimum_liquidity, redemption_output},
    ClaimProof, UserSaleState, PROTOCOL_VERSION,
};
use proptest::prelude::*;
use solana_sdk::pubkey::Pubkey;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]
    #[test]
    fn sequential_redemption_conserves_every_lamport_for_arbitrary_holder_partitions(
        weights in prop::collection::vec(1u64..1_000_000, 1..32), reserve in any::<u64>(), reverse in any::<bool>()
    ) {
        let mut supply: u64 = weights.iter().sum();
        let initial = reserve.max(supply); // every holder has a nonzero redeemable share
        let mut remaining = initial;
        let mut paid = 0u128;
        let recipients: Vec<_> = if reverse { weights.into_iter().rev().collect() } else { weights };
        for amount in recipients {
            let output = redemption_output(remaining, amount, supply).unwrap();
            prop_assert!(output > 0 && output <= remaining);
            // Ratio changes can favor remaining holders only by integer rounding.
            prop_assert!((output as u128) * (supply as u128) <= (remaining as u128) * amount as u128);
            paid += output as u128; remaining -= output; supply -= amount;
        }
        prop_assert_eq!(remaining, 0); prop_assert_eq!(supply, 0);
        prop_assert_eq!(paid, initial as u128);
    }
    #[test]
    fn exact_raydium_boundary_matches_integer_ceiling_oracle(tokens in 1u64..=u64::MAX) {
        // A different formulation than the production product check.
        let locked_plus_one = 1_000_001u128;
        let product = locked_plus_one * locked_plus_one;
        let minimum_sol = ((product + tokens as u128 - 1) / tokens as u128) as u64;
        prop_assert!(!has_minimum_liquidity(tokens, minimum_sol - 1, 6).unwrap());
        prop_assert!(has_minimum_liquidity(tokens, minimum_sol, 6).unwrap());
    }
    #[test]
    fn adding_backing_or_burning_outside_tokens_cannot_reduce_a_remaining_holders_quote(
        reserve in 1u64..=u64::MAX / 2, donation in 0u64..=u64::MAX / 2,
        supply in 2u64..1_000_000_000_000, fraction in 1u64..1_000_000
    ) {
        let amount = 1 + fraction % (supply - 1);
        let before = redemption_output(reserve, amount, supply).unwrap();
        prop_assert!(redemption_output(reserve + donation, amount, supply).unwrap() >= before);
        let burnable = supply - amount;
        prop_assert!(redemption_output(reserve, amount, supply - burnable).unwrap() >= before);
    }
}

#[test]
fn proof_and_sale_accounts_have_exact_serialized_capacity() {
    let proof = ClaimProof {
        token: Pubkey::new_unique(),
        user: Pubkey::new_unique(),
        root: Pubkey::new_unique(),
        stage: 100,
        category: 1,
        length: 32,
        amount: u64::MAX,
        written: u32::MAX,
        nodes: [[255; 32]; 32],
    };
    let mut bytes = Vec::new();
    proof.try_serialize(&mut bytes).unwrap();
    assert_eq!(bytes.len(), ClaimProof::SPACE);
    assert_eq!(bytes.len(), 1143);
    let sale = UserSaleState {
        token: Pubkey::new_unique(),
        user: Pubkey::new_unique(),
        last_sold_stage: 100,
        protocol_version: PROTOCOL_VERSION,
    };
    bytes.clear();
    sale.try_serialize(&mut bytes).unwrap();
    assert_eq!(bytes.len(), UserSaleState::SPACE);
    assert_eq!(bytes.len(), 74);
}

#[test]
fn exit_math_rejects_zero_overspend_and_unrepresentable_decimal_thresholds() {
    for (amount, supply) in [(0, 0), (0, 1), (2, 1), (u64::MAX, u64::MAX - 1)] {
        assert!(redemption_output(u64::MAX, amount, supply).is_err());
    }
    assert_eq!(redemption_output(0, 1, 1).unwrap(), 0); // handler refuses zero-output burns
    assert_eq!(
        redemption_output(u64::MAX, u64::MAX, u64::MAX).unwrap(),
        u64::MAX
    );
    assert!(!has_minimum_liquidity(0, u64::MAX, 6).unwrap());
    assert!(!has_minimum_liquidity(u64::MAX, 0, 6).unwrap());
    assert!(has_minimum_liquidity(u64::MAX, u64::MAX, 255).is_err());
}

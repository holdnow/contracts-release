//! Additional pure invariant/property coverage of public production helpers.
use anchor_lang::AccountDeserialize;
use holdnow::{
    utils::{current_tax, curve_output, percentage},
    BondingCurve, PROTOCOL_VERSION,
};
use proptest::prelude::*;
fn curve() -> BondingCurve {
    let data = vec![0; 1024];
    let mut c = BondingCurve::try_deserialize_unchecked(&mut data.as_slice()).unwrap();
    c.protocol_version = PROTOCOL_VERSION;
    c.number_stage = 3;
    c.token_total_supply = 100;
    c.current_stage = 1;
    c.stage_duration = 86400;
    c.at_start_stage = 1000;
    c.sell_tax_min = 2;
    c.sell_tax_max = 98;
    c.sell_tax_decay = 50;
    c.cumulative_merkle_roots = vec![Default::default(); 4];
    c.stage_completion_times = vec![0; 4];
    c.total_claimed_per_stage = vec![0; 4];
    c.stage_reward_pools = vec![0; 4];
    c.stage_eligible_supply = vec![0; 4];
    c.stage_settled = vec![false; 4];
    c.stage_reward_rollovers = vec![0; 4];
    c
}
#[test]
fn tax_at_every_boundary_and_instant_decay() {
    let mut c = curve();
    for (now, expected) in [
        (i64::MIN, 98),
        (999, 98),
        (1000, 98),
        (22600, 50),
        (44200, 2),
        (i64::MAX, 2),
    ] {
        assert_eq!(current_tax(&c, now).unwrap(), expected);
    }
    c.sell_tax_decay = 0;
    assert_eq!(current_tax(&c, 1000).unwrap(), 2);
    c.sell_tax_min = 98;
    assert_eq!(current_tax(&c, 1000).unwrap(), 98);
}
#[test]
fn malformed_tax_and_percentage_inputs_fail_without_panics() {
    for (min, max, decay, duration) in [
        (20, 10, 1, 86400),
        (0, 101, 1, 86400),
        (0, 98, 101, 86400),
        (0, 98, 100, 0),
        (0, 98, 100, -1),
    ] {
        let mut c = curve();
        c.sell_tax_min = min;
        c.sell_tax_max = max;
        c.sell_tax_decay = decay;
        c.stage_duration = duration;
        assert!(current_tax(&c, 1000).is_err());
    }
    assert!(percentage(1, 101).is_err());
    assert!(percentage(u64::MAX, u64::MAX).is_err());
    for amount in [0, 1, 99, 100, u64::MAX] {
        assert_eq!(percentage(amount, 0).unwrap(), 0);
        assert_eq!(percentage(amount, 100).unwrap(), amount);
    }
}
#[test]
fn stage_validation_covers_truncation_clock_overflow_and_all_versions() {
    let mut c = curve();
    for version in 0..=u8::MAX {
        c.protocol_version = version;
        assert_eq!(c.assert_current().is_ok(), version == PROTOCOL_VERSION);
    }
    c.protocol_version = PROTOCOL_VERSION;
    assert!(c.stage_index(0).is_err());
    assert!(c.stage_index(4).is_err());
    c.stage_completion_times.truncate(1);
    assert!(c.stage_index(1).is_err());
    c.stage_completion_times = vec![0; 4];
    c.total_claimed_per_stage.clear();
    assert!(c.stage_index(1).is_err());
    c.total_claimed_per_stage = vec![0; 4];
    c.stage_reward_pools.clear();
    assert!(c.assert_trading(1000).is_err());
    c.stage_reward_pools = vec![0; 4];
    c.stage_eligible_supply = vec![0; 4];
    c.stage_settled = vec![false; 4];
    c.stage_reward_rollovers = vec![0; 4];
    c.at_start_stage = i64::MAX;
    assert!(c.assert_trading(i64::MAX).is_err());
    c.at_start_stage = 1000;
    c.current_stage = u64::MAX;
    assert!(c.assert_trading(1000).is_err());
    c.current_stage = 1;
    assert!(c.assert_trading(999).is_err());
    assert!(c.assert_trading(87400).is_err());
    assert_eq!(c.assert_trading(1000).unwrap(), 0);
    assert_eq!(c.assert_trading(87399).unwrap(), 0);
}
#[test]
fn quote_extremes_and_zero_reserves_are_defined() {
    for input in [0, 1, u64::MAX] {
        assert!(curve_output(input, 0, 1).is_err());
        assert!(curve_output(input, 1, 0).is_err());
        assert!(curve_output(input, u64::MAX, u64::MAX).unwrap() < u64::MAX);
    }
    assert_eq!(curve_output(u64::MAX, 1, u64::MAX).unwrap(), u64::MAX - 1);
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]
    #[test]
    fn percentages_are_bounded_monotone_and_round_by_less_than_one_unit(amount in any::<u64>(),low in 0u64..=100,high in 0u64..=100) {
        let a=percentage(amount,low.min(high)).unwrap();let b=percentage(amount,low.max(high)).unwrap();
        prop_assert!(a<=b && b<=amount);
        let exact=amount as u128*low.max(high) as u128;
        prop_assert!(b as u128*100<=exact && exact<(b as u128+1)*100);
    }
    #[test]
    fn tax_is_monotone_and_bounded(min in 0u8..=98,max in 0u8..=98,decay in 0u8..=100,days in 1i64..=365,t1 in 0i64..40_000_000,t2 in 0i64..40_000_000) {
        let mut c=curve();c.sell_tax_min=min.min(max);c.sell_tax_max=min.max(max);c.sell_tax_decay=decay;c.stage_duration=days*86400;
        let a=current_tax(&c,1000+t1.min(t2)).unwrap();let b=current_tax(&c,1000+t1.max(t2)).unwrap();
        prop_assert!(a>=b && a<=c.sell_tax_max as u64 && b>=c.sell_tax_min as u64);
    }
    #[test]
    fn a_fee_free_round_trip_cannot_extract_more_than_the_input(sol in 1u64..1_000_000_000_000,tokens in 1u64..1_000_000_000_000_000,input in 1u64..1_000_000_000_000) {
        let bought=curve_output(input,sol,tokens).unwrap();
        let returned=curve_output(bought,tokens-bought,sol+input).unwrap();
        prop_assert!(returned<=input);
    }
}

#[test]
fn launch_reserves_reject_overflow_and_preserve_u64_boundaries() {
    use holdnow::utils::launch_reserves;
    assert_eq!(
        launch_reserves(u64::MAX, u64::MAX, 0).unwrap(),
        (u64::MAX, u64::MAX)
    );
    assert_eq!(
        launch_reserves(100, u64::MAX / 2, 50).unwrap(),
        (50, u64::MAX - 1)
    );
    for args in [
        (100, 1u64 << 63, 50),
        (100, 1, 100),
        (100, 1, 101),
        (100, 0, 0),
        (0, 1, 0),
    ] {
        assert!(launch_reserves(args.0, args.1, args.2).is_err());
    }
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]
    #[test]
    fn launch_adjustment_never_truncates(tokens in 1u64..=u64::MAX, sol in 1u64..=u64::MAX, airdrop in any::<u64>()) {
        let result = holdnow::utils::launch_reserves(tokens, sol, airdrop);
        if airdrop >= tokens { prop_assert!(result.is_err()); }
        else {
            let exact = sol as u128 * tokens as u128 / (tokens - airdrop) as u128;
            if exact > u64::MAX as u128 { prop_assert!(result.is_err()); }
            else { prop_assert_eq!(result.unwrap(), (tokens - airdrop, exact as u64)); }
        }
    }
}

#[test]
fn every_settlement_vector_is_independently_required_for_a_claim_stage() {
    for field in 0..7 {
        let mut c = curve();
        match field {
            0 => c.cumulative_merkle_roots.truncate(3),
            1 => c.stage_completion_times.truncate(3),
            2 => c.total_claimed_per_stage.truncate(3),
            3 => c.stage_settled.truncate(3),
            4 => c.stage_reward_rollovers.truncate(3),
            5 => c.stage_reward_pools.truncate(3),
            _ => c.stage_eligible_supply.truncate(3),
        }
        assert!(c.stage_index(3).is_err(), "vector {field}");
        assert_eq!(c.stage_index(2).unwrap(), 2);
    }
}

#[test]
fn tax_decay_restarts_at_each_stage_start_and_handles_subsecond_rounding() {
    let mut c = curve();
    c.sell_tax_min = 0;
    c.sell_tax_max = 98;
    c.sell_tax_decay = 1;
    assert_eq!(current_tax(&c, c.at_start_stage).unwrap(), 98);
    assert_eq!(current_tax(&c, c.at_start_stage + 864).unwrap(), 0);
    c.at_start_stage += c.stage_duration + c.claim_duration;
    assert_eq!(current_tax(&c, c.at_start_stage).unwrap(), 98);
    assert_eq!(current_tax(&c, c.at_start_stage + 863).unwrap(), 1);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]
    #[test]
    fn taxed_round_trip_never_creates_sol_or_tokens(
        sol in 1u64..1_000_000_000_000,
        token in 1u64..1_000_000_000_000_000,
        input in 1u64..1_000_000_000_000,
        fee in 0u64..=10, staging in 0u64..=10, tax in 0u64..=98,
    ) {
        let buy_fee = percentage(input, fee).unwrap();
        let net = input - buy_fee;
        let gross = curve_output(net, sol, token).unwrap();
        let buy_rewards = percentage(gross, staging).unwrap();
        let held = gross - buy_rewards;
        let stage_rewards = percentage(held, staging).unwrap();
        let tax_rewards = percentage(held - stage_rewards, tax).unwrap();
        let returned = held - stage_rewards - tax_rewards;
        let output = curve_output(returned, token - gross, sol + net).unwrap();
        let sell_fee = percentage(output, fee).unwrap();
        prop_assert!(output - sell_fee <= input);
        prop_assert_eq!(token - gross + returned + buy_rewards + stage_rewards + tax_rewards, token);
        prop_assert!((token - gross + returned) as u128 * (sol + net - output) as u128
            >= token as u128 * sol as u128);
    }
}

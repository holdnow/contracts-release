use super::*;
use crate::test_support::curve;
use anchor_lang::AccountSerialize;

#[test]
fn allocated_space_covers_actual_serialized_layout_for_supported_stage_counts() {
    for count in [1, 3, 10, 100] {
        let n = if count == 0 { 10 } else { count as usize + 1 };
        let mut c = curve(1, 1);
        c.cumulative_merkle_roots = vec![Pubkey::default(); n];
        c.stage_completion_times = vec![0; n];
        c.stage_reward_pools = vec![0; n];
        c.stage_eligible_supply = vec![0; n];
        c.total_claimed_per_stage = vec![0; n];
        c.stage_settled = vec![false; n];
        c.stage_reward_rollovers = vec![0; n];
        let mut bytes = vec![];
        c.try_serialize(&mut bytes).unwrap();
        assert!(BondingCurve::space(count).unwrap() >= bytes.len());
    }
}

#[test]
fn excessive_stage_counts_exceed_anchor_initial_allocation_limit() {
    assert!(BondingCurve::space(255).is_err());
    assert!(BondingCurve::space(0).is_err());
}

#[test]
fn extreme_stage_count_is_rejected_without_panicking() {
    assert!(BondingCurve::space(u64::MAX).is_err());
}

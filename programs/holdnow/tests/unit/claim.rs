use super::*;
use proptest::prelude::*;

#[test]
fn single_leaf_and_sorted_two_leaf_proofs() {
    let a = compute_initial_leaf(
        &Pubkey::new_from_array([1; 32]),
        &Pubkey::new_from_array([2; 32]),
        123,
    );
    let b = compute_initial_leaf(
        &Pubkey::new_from_array([3; 32]),
        &Pubkey::new_from_array([2; 32]),
        456,
    );
    assert!(verify_proof(&[], Pubkey::new_from_array(a), a));
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let root = Pubkey::new_from_array(keccak::hashv(&[&lo, &hi]).to_bytes());
    assert!(verify_proof(&[b], root, a));
    assert!(verify_proof(&[a], root, b));
    assert!(!verify_proof(&[], root, a));
    assert!(!verify_proof(&[[0; 32]], root, a));
}

#[test]
fn leaf_domains_bind_all_fields_and_use_little_endian_amounts() {
    let user = Pubkey::new_from_array([1; 32]);
    let mint = Pubkey::new_from_array([2; 32]);
    let amount = 0x0102030405060708;
    let regular = compute_cumulative_leaf(&user, &mint, 7, amount);
    let mut bytes = vec![1u8; 32];
    bytes.extend([2u8; 32]);
    bytes.push(7);
    bytes.extend([8, 7, 6, 5, 4, 3, 2, 1]);
    bytes.extend(b"CUMULATIVE");
    assert_eq!(regular, keccak::hash(&bytes).to_bytes());
    assert_ne!(regular, compute_initial_leaf(&user, &mint, amount));
    assert_ne!(regular, compute_cumulative_leaf(&mint, &mint, 7, amount));
    assert_ne!(regular, compute_cumulative_leaf(&user, &user, 7, amount));
    assert_ne!(regular, compute_cumulative_leaf(&user, &mint, 8, amount));
    assert_ne!(
        regular,
        compute_cumulative_leaf(&user, &mint, 7, amount + 1)
    );
}

#[test]
fn multi_level_proof_requires_every_sibling_in_order() {
    let pair = |a: [u8; 32], b: [u8; 32]| {
        if a < b {
            keccak::hashv(&[&a, &b]).to_bytes()
        } else {
            keccak::hashv(&[&b, &a]).to_bytes()
        }
    };
    let leaves = [[1; 32], [2; 32], [3; 32], [4; 32]];
    let left = pair(leaves[0], leaves[1]);
    let right = pair(leaves[2], leaves[3]);
    let root = Pubkey::new_from_array(pair(left, right));
    assert!(verify_proof(&[leaves[1], right], root, leaves[0]));
    assert!(!verify_proof(&[right, leaves[1]], root, leaves[0]));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn changed_allocation_never_verifies_against_original_single_leaf(amount in any::<u64>(),stage in any::<u8>()) {
        let user=Pubkey::new_from_array([10;32]);let mint=Pubkey::new_from_array([11;32]);
        let root=Pubkey::new_from_array(compute_cumulative_leaf(&user,&mint,stage,amount));
        let changed=compute_cumulative_leaf(&user,&mint,stage,amount.wrapping_add(1));
        prop_assert!(!verify_proof(&[],root,changed));
    }
}

#![allow(dead_code, unused_imports)]
use anchor_lang::solana_program::system_program;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_system_interface::instruction as system_instruction;
// Additional adversarial and full lifecycle security regressions.
mod common;
use anchor_lang::InstructionData;
use common::*;
use contract::{error::HoldNowCode, UserClaimState, UserVestingState};
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::Signer, transaction::Transaction,
};
fn code(e: HoldNowCode) -> u32 {
    6000 + e as u32
}
async fn balances(f: &mut Fixture) -> [u64; 6] {
    [
        tokens(&mut f.ctx, f.curve_tokens).await,
        tokens(&mut f.ctx, f.reward_tokens).await,
        tokens(&mut f.ctx, f.trader_tokens).await,
        lamports(&mut f.ctx, f.vault).await,
        lamports(&mut f.ctx, f.trader.pubkey()).await,
        lamports(&mut f.ctx, f.fee).await,
    ]
}
#[tokio::test]
async fn wrong_authority_and_program_accounts_fail_atomically() {
    let mut f = Fixture::launched(Config::default()).await;
    f.buy(SOL).await;
    let amount = tokens(&mut f.ctx, f.trader_tokens).await / 2;
    let buy = f.buy_ix(SOL, 0);
    let sell = f.sell_ix(f.trader.pubkey(), f.trader_tokens, amount, 0);
    let before = balances(&mut f).await;
    for (base, index, replacement) in [
        (&buy, 3, f.trader_tokens),
        (&buy, 7, f.trader_tokens),
        (&buy, 2, f.outsider.pubkey()),
        (&buy, 11, anchor_lang::solana_program::system_program::id()),
        (&sell, 2, f.trader_tokens),
        (&sell, 7, f.trader_tokens),
        (&sell, 5, f.outsider.pubkey()),
        (&sell, 11, anchor_lang::solana_program::system_program::id()),
    ] {
        let mut i = base.clone();
        i.accounts[index].pubkey = replacement;
        assert!(send(&mut f.ctx, &[i], &[&f.trader]).await.is_err());
        assert_eq!(balances(&mut f).await, before);
    }
}
#[tokio::test]
async fn missing_signatures_never_authorize_trade_stage_claim_or_configuration() {
    let mut f = Fixture::launched(Config::default()).await;
    f.buy(SOL).await;
    let amount = tokens(&mut f.ctx, f.trader_tokens).await / 2;
    let instructions = [
        f.buy_ix(SOL, 0),
        f.sell_ix(f.trader.pubkey(), f.trader_tokens, amount, 0),
        f.stage_ix(),
        f.claim_ix(1, 1, vec![], None, None),
        f.params_ix(f.admin.pubkey(), 1, 1, SUPPLY, 30 * SOL, SUPPLY),
    ];
    let before = balances(&mut f).await;
    for mut i in instructions {
        for a in &mut i.accounts {
            a.is_signer = false;
        }
        assert!(send(&mut f.ctx, &[i], &[]).await.is_err());
        assert_eq!(balances(&mut f).await, before);
    }
}
#[tokio::test]
async fn failures_after_sol_transfers_roll_back_all_assets_and_reserves() {
    let mut f = Fixture::launched(Config {
        decay: 0,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    let before = balances(&mut f).await;
    let c = f.curve().await;
    let insolvent = f.buy_ix(2000 * SOL, 0);
    assert!(send(&mut f.ctx, &[insolvent], &[&f.trader]).await.is_err());
    assert_eq!(balances(&mut f).await, before);
    let amount = before[2] + 1;
    let sell = f.sell_ix(f.trader.pubkey(), f.trader_tokens, amount, 0);
    assert!(send(&mut f.ctx, &[sell], &[&f.trader]).await.is_err());
    assert_eq!(balances(&mut f).await, before);
    let after = f.curve().await;
    assert_eq!(c.real_sol_reserves, after.real_sol_reserves);
    assert_eq!(c.real_token_reserves, after.real_token_reserves);
    assert_eq!(c.stage_reward_pools, after.stage_reward_pools);
}
#[tokio::test]
async fn prefunded_launch_pdas_cannot_prevent_atomic_initialization() {
    let mut f = Fixture::minted(Config::default()).await;
    let donations = [f.curve, f.curve_tokens, f.reward_authority, f.reward_tokens].map(|to| {
        system_instruction::transfer(
            &f.outsider.pubkey(),
            &to,
            solana_sdk::rent::Rent::default().minimum_balance(0),
        )
    });
    send(&mut f.ctx, &donations, &[&f.outsider]).await.unwrap();
    let init = f.initialize_curve_ix(f.creator.pubkey(), 3);
    let create = f.create_ix();
    send(&mut f.ctx, &[init, create], &[&f.creator])
        .await
        .unwrap();
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, SUPPLY);
    assert_eq!(f.curve().await.token_total_supply, SUPPLY);
}
#[tokio::test]
async fn roots_reject_zero_and_invalid_eligible_supply_without_state_changes() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    f.end_stage().await;
    for (root, eligible) in [
        (Pubkey::default(), SUPPLY),
        (Pubkey::new_unique(), SUPPLY + 1),
    ] {
        let mut i = f.root_ix(f.admin.pubkey(), 1, root);
        i.data = contract::instruction::UpdateMerkleRoot {
            stage: 1,
            merkle_root: root,
            total_eligible_supply: eligible,
        }
        .data();
        assert!(send(&mut f.ctx, &[i], &[&f.admin]).await.is_err());
        assert_eq!(
            f.curve().await.cumulative_merkle_roots[1],
            Pubkey::default()
        );
    }
}
#[tokio::test]
async fn vested_amount_cannot_be_increased_by_changing_cached_claim_arguments() {
    let amount = 1_000_000;
    let mut f = Fixture::initialized(Config {
        airdrop: amount,
        stages: 2,
        ..Config::default()
    })
    .await;
    f.config.root =
        Pubkey::new_from_array(initial_leaf(f.trader.pubkey(), f.mint.pubkey(), amount));
    let create = f.create_ix();
    send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
    f.end_stage().await;
    let first = f.claim_ix(1, 0, vec![], Some(amount), Some(vec![]));
    send(&mut f.ctx, &[first], &[&f.trader]).await.unwrap();
    f.next_stage().await;
    f.end_stage().await;
    let second = f.claim_ix(2, 0, vec![], Some(u64::MAX), Some(vec![[7; 32]]));
    send(&mut f.ctx, &[second], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, amount);
    assert_eq!(f.curve().await.total_initial_claimed, amount);
    let repeated = f.claim_ix(1, 0, vec![], Some(amount), Some(vec![]));
    assert!(send(&mut f.ctx, &[repeated], &[&f.trader]).await.is_err());
}
fn fold_proof(mut leaf: [u8; 32], proof: &[[u8; 32]]) -> [u8; 32] {
    for sibling in proof {
        leaf = if leaf < *sibling {
            solana_sdk::keccak::hashv(&[&leaf, sibling]).to_bytes()
        } else {
            solana_sdk::keccak::hashv(&[sibling, &leaf]).to_bytes()
        };
    }
    leaf
}
#[tokio::test]
async fn a_fourteen_sibling_claim_fits_a_real_packet_and_executes_with_only_its_beneficiary() {
    let mut f = Fixture::launched(Config::default()).await;
    f.buy(SOL).await;
    f.end_stage().await;
    let amount = f.curve().await.stage_reward_pools[0];
    let proof: Vec<_> = (0..14).map(|n| [n + 1; 32]).collect();
    let root = fold_proof(
        regular_leaf(f.trader.pubkey(), f.mint.pubkey(), 1, amount),
        &proof,
    );
    let publish = f.root_ix(f.admin.pubkey(), 1, Pubkey::new_from_array(root));
    send(&mut f.ctx, &[publish], &[&f.admin]).await.unwrap();
    let claim = f.claim_ix(1, amount, proof, None, None);
    let hash = f.ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[
            ComputeBudgetInstruction::set_compute_unit_limit(300_000),
            claim,
        ],
        Some(&f.trader.pubkey()),
        &[&f.trader],
        hash,
    );
    let wire_size = 1 + 64 * tx.signatures.len() + tx.message_data().len();
    assert!(wire_size <= 1232, "packet size {wire_size}");
    assert_eq!(tx.signatures.len(), 1);
    f.ctx.banks_client.process_transaction(tx).await.unwrap();
    assert_eq!(f.curve().await.total_regular_claimed, amount);
}
#[tokio::test]
async fn all_one_hundred_stages_progress_and_pay_cumulative_and_vested_claims_without_state_injection(
) {
    let allocation = 1_000_003;
    let mut f = Fixture::initialized(Config {
        stages: 100,
        airdrop: allocation,
        decay: 0,
        ..Config::default()
    })
    .await;
    f.config.root =
        Pubkey::new_from_array(initial_leaf(f.trader.pubkey(), f.mint.pubkey(), allocation));
    let create = f.create_ix();
    send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
    let mut cumulative = 0u64;
    for stage in 1..=100u8 {
        f.buy(10_000_000).await;
        if stage % 2 == 0 {
            let balance = tokens(&mut f.ctx, f.trader_tokens).await;
            let sale = f.sell_ix(f.trader.pubkey(), f.trader_tokens, balance / 10, 0);
            send(&mut f.ctx, &[sale], &[&f.trader]).await.unwrap();
        }
        f.end_stage().await;
        cumulative += f.curve().await.stage_reward_pools[stage as usize - 1];
        let root = f.root_ix(
            f.admin.pubkey(),
            stage,
            Pubkey::new_from_array(regular_leaf(
                f.trader.pubkey(),
                f.mint.pubkey(),
                stage,
                cumulative,
            )),
        );
        send(&mut f.ctx, &[root], &[&f.admin]).await.unwrap();
        let claim = f.claim_ix(stage, cumulative, vec![], Some(allocation), Some(vec![]));
        send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
        let c = f.curve().await;
        assert_eq!(c.total_regular_claimed, cumulative);
        assert_eq!(c.total_initial_claimed, allocation * stage as u64 / 100);
        assert_eq!(
            tokens(&mut f.ctx, f.curve_tokens).await,
            c.real_token_reserves
        );
        assert_eq!(
            tokens(&mut f.ctx, f.reward_tokens).await,
            allocation - c.total_initial_claimed
        );
        assert_eq!(
            tokens(&mut f.ctx, f.curve_tokens).await
                + tokens(&mut f.ctx, f.reward_tokens).await
                + tokens(&mut f.ctx, f.trader_tokens).await,
            SUPPLY
        );
        f.next_stage().await;
    }
    let c = f.curve().await;
    assert!(c.complete);
    assert_eq!(c.current_stage, 101);
    assert!(c.stage_completion_times[1..].iter().all(|t| *t > 0));
    assert_eq!(c.total_initial_claimed, allocation);
}

fn merkle_four(leaves: [[u8; 32]; 4]) -> ([u8; 32], Vec<Vec<[u8; 32]>>) {
    let left = fold_proof(leaves[0], &[leaves[1]]);
    let right = fold_proof(leaves[2], &[leaves[3]]);
    (
        fold_proof(left, &[right]),
        vec![
            vec![leaves[1], right],
            vec![leaves[0], right],
            vec![leaves[3], left],
            vec![leaves[2], left],
        ],
    )
}
#[tokio::test]
async fn multiple_beneficiaries_claim_in_any_order_and_claim_late_without_double_payment() {
    let mut f = Fixture::launched(Config {
        stages: 2,
        ..Config::default()
    })
    .await;
    let users = [
        f.trader.pubkey(),
        f.creator.pubkey(),
        f.outsider.pubkey(),
        f.admin.pubkey(),
    ];
    let atas = users.map(|u| {
        spl_associated_token_account::address::get_associated_token_address(&u, &f.mint.pubkey())
    });
    let instructions: Vec<_> = users[2..]
        .iter()
        .map(|u| {
            spl_associated_token_account::instruction::create_associated_token_account(
                &f.outsider.pubkey(),
                u,
                &f.mint.pubkey(),
                &spl_token::id(),
            )
        })
        .collect();
    send(&mut f.ctx, &instructions, &[&f.outsider])
        .await
        .unwrap();
    let mut cumulative = [0u64; 4];
    let mut earned_total = 0;
    for stage in 1..=2u8 {
        f.buy(SOL).await;
        f.end_stage().await;
        let pool = f.curve().await.stage_reward_pools[stage as usize - 1];
        earned_total += pool;
        for (index, amount) in cumulative.iter_mut().enumerate() {
            *amount += if index == 3 {
                pool - 3 * (pool / 4)
            } else {
                pool / 4
            };
        }
        let leaves = std::array::from_fn(|index| {
            regular_leaf(users[index], f.mint.pubkey(), stage, cumulative[index])
        });
        let (root, proofs) = merkle_four(leaves);
        let publish = f.root_ix(f.admin.pubkey(), stage, Pubkey::new_from_array(root));
        send(&mut f.ctx, &[publish], &[&f.admin]).await.unwrap();
        if stage == 2 {
            f.next_stage().await;
            assert!(f.curve().await.complete);
        }
        for index in if stage == 1 {
            [3, 1, 0, 2]
        } else {
            [2, 0, 1, 3]
        } {
            let mut claim = f.claim_ix(stage, cumulative[index], proofs[index].clone(), None, None);
            claim.accounts[2].pubkey = Pubkey::find_program_address(
                &[
                    contract::USER_CLAIM_STATE_SEED,
                    f.mint.pubkey().as_ref(),
                    users[index].as_ref(),
                ],
                &contract::ID,
            )
            .0;
            claim.accounts[3].pubkey = Pubkey::find_program_address(
                &[
                    contract::USER_VESTING_STATE_SEED,
                    f.mint.pubkey().as_ref(),
                    users[index].as_ref(),
                ],
                &contract::ID,
            )
            .0;
            claim.accounts[6].pubkey = atas[index];
            claim.accounts[7].pubkey = users[index];
            let signer = match index {
                0 => &f.trader,
                1 => &f.creator,
                2 => &f.outsider,
                _ => &f.admin,
            };
            send(&mut f.ctx, &[claim.clone()], &[signer]).await.unwrap();
            custom_error(
                send(&mut f.ctx, &[claim], &[signer]).await,
                code(HoldNowCode::NothingToClaim),
            );
        }
        assert_eq!(f.curve().await.total_regular_claimed, earned_total);
        assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
        let mut circulating = 0;
        for account in atas {
            circulating += tokens(&mut f.ctx, account).await;
        }
        assert_eq!(
            circulating + tokens(&mut f.ctx, f.curve_tokens).await,
            SUPPLY
        );
        if stage == 1 {
            f.next_stage().await;
        }
    }
}

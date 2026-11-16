#![allow(dead_code, unused_imports)]
use anchor_lang::solana_program::system_program;
use solana_compute_budget_interface::ComputeBudgetInstruction;
// Fresh review of revision 8b9d993. Adversarial state-machine/accounting tests.
mod common;
use anchor_lang::{AccountSerialize, InstructionData};
use common::*;
use contract::error::HoldNowCode;
use solana_sdk::{program_pack::Pack, pubkey::Pubkey, signature::Signer, transaction::Transaction};
use spl_token::state::Mint;

fn empty(f: &Fixture, stage: u8) -> solana_sdk::instruction::Instruction {
    let mut i = f.root_ix(f.admin.pubkey(), stage, Pubkey::default());
    i.data = contract::instruction::UpdateMerkleRoot {
        stage,
        merkle_root: Pubkey::default(),
        total_eligible_supply: 0,
    }
    .data();
    i
}
async fn snapshot(f: &mut Fixture) -> (Vec<u8>, [u64; 4]) {
    let mut data = vec![];
    f.curve().await.try_serialize(&mut data).unwrap();
    (
        data,
        [
            tokens(&mut f.ctx, f.curve_tokens).await,
            tokens(&mut f.ctx, f.reward_tokens).await,
            tokens(&mut f.ctx, f.trader_tokens).await,
            lamports(&mut f.ctx, f.vault).await,
        ],
    )
}

#[tokio::test]
async fn repeated_rollovers_final_burn_preserve_partial_vesting_old_claims_and_donations() {
    let initial = 1_000_003;
    let mut f = Fixture::initialized(Config {
        stages: 4,
        airdrop: initial,
        ..Config::default()
    })
    .await;
    f.config.root =
        Pubkey::new_from_array(initial_leaf(f.trader.pubkey(), f.mint.pubkey(), initial));
    let create = f.create_ix();
    send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
    f.buy(SOL).await;
    f.end_stage().await;
    let old = f.curve().await.stage_reward_pools[0];
    let root = f.root_ix(
        f.admin.pubkey(),
        1,
        Pubkey::new_from_array(regular_leaf(f.trader.pubkey(), f.mint.pubkey(), 1, old)),
    );
    send(&mut f.ctx, &[root], &[&f.admin]).await.unwrap();
    let vesting = f.claim_ix(1, 0, vec![], Some(initial), Some(vec![]));
    send(&mut f.ctx, &[vesting], &[&f.trader]).await.unwrap();
    f.next_stage().await;
    let donation = 137;
    let donate = spl_token::instruction::transfer(
        &spl_token::id(),
        &f.trader_tokens,
        &f.reward_tokens,
        &f.trader.pubkey(),
        &[],
        donation,
    )
    .unwrap();
    send(&mut f.ctx, &[donate], &[&f.trader]).await.unwrap();
    let mut carried = 0;
    for stage in 2..=4 {
        let before_pool = f.curve().await.stage_reward_pools[stage as usize - 1];
        assert_eq!(before_pool, carried);
        f.buy(SOL).await;
        let budget = f.curve().await.stage_reward_pools[stage as usize - 1];
        assert!(budget > carried);
        f.end_stage().await;
        let settle = empty(&f, stage);
        let before_sol = lamports(&mut f.ctx, f.vault).await;
        send(&mut f.ctx, &[settle.clone()], &[&f.admin])
            .await
            .unwrap();
        let c = f.curve().await;
        assert_eq!(c.stage_reward_pools[stage as usize - 1], 0);
        if stage < 4 {
            assert_eq!(c.stage_reward_rollovers[stage as usize], budget);
            assert_eq!(c.stage_reward_pools[stage as usize], budget);
        } else {
            assert_eq!(c.total_unallocated_burned, budget);
            let mint = f
                .ctx
                .banks_client
                .get_account(f.mint.pubkey())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(Mint::unpack(&mint.data).unwrap().supply, SUPPLY - budget);
            assert_eq!(
                tokens(&mut f.ctx, f.reward_tokens).await,
                old + initial - initial / 4 + donation
            );
        }
        assert_eq!(lamports(&mut f.ctx, f.vault).await, before_sol);
        let before = snapshot(&mut f).await;
        custom_error(
            send(&mut f.ctx, &[settle], &[&f.admin]).await,
            6000 + HoldNowCode::MerkleRootAlreadySet as u32,
        );
        assert_eq!(snapshot(&mut f).await, before);
        carried = budget;
        f.next_stage().await;
    }
    let claim = f.claim_ix(1, old, vec![], None, None);
    send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
    let vesting = f.claim_ix(4, 0, vec![], Some(initial), Some(vec![]));
    send(&mut f.ctx, &[vesting], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, donation);
    assert_eq!(f.curve().await.total_regular_claimed, old);
    assert_eq!(f.curve().await.total_initial_claimed, initial);
}

#[tokio::test]
async fn burn_rejects_every_substituted_signer_vault_mint_and_program_atomically() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    f.end_stage().await;
    let before = snapshot(&mut f).await;
    // Different signer, reward authority, canonical token account, mint, and token program.
    for (index, key) in [
        (3, f.outsider.pubkey()),
        (5, f.vault),
        (6, f.trader_tokens),
        (1, f.trader_tokens),
        (7, anchor_lang::solana_program::system_program::id()),
    ] {
        let mut i = empty(&f, 1);
        i.accounts[index].pubkey = key;
        let signer = if index == 3 { &f.outsider } else { &f.admin };
        assert!(send(&mut f.ctx, &[i], &[signer]).await.is_err());
        assert_eq!(snapshot(&mut f).await, before);
    }
    let i = empty(&f, 1);
    send(&mut f.ctx, &[i], &[&f.admin]).await.unwrap();
    assert!(f.curve().await.total_unallocated_burned > 0);
}

#[tokio::test]
async fn initial_vesting_is_claimable_during_operator_outage_before_settlement() {
    let initial = 1_000_003;
    let mut f = Fixture::initialized(Config {
        stages: 1,
        airdrop: initial,
        ..Config::default()
    })
    .await;
    f.config.root =
        Pubkey::new_from_array(initial_leaf(f.trader.pubkey(), f.mint.pubkey(), initial));
    let create = f.create_ix();
    send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
    f.end_stage().await;
    clock(&mut f.ctx, START + 365 * 86400).await;
    let claim = f.claim_ix(1, 0, vec![], Some(initial), Some(vec![]));
    send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
    assert_eq!(f.curve().await.total_initial_claimed, initial);
    assert!(!f.curve().await.stage_settled[1]);
}

#[tokio::test]
async fn invalid_regular_proof_rolls_back_valid_vesting_and_new_claim_accounts() {
    let initial = 999;
    let mut f = Fixture::initialized(Config {
        airdrop: initial,
        ..Config::default()
    })
    .await;
    f.config.root =
        Pubkey::new_from_array(initial_leaf(f.trader.pubkey(), f.mint.pubkey(), initial));
    let create = f.create_ix();
    send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
    f.buy(SOL).await;
    f.end_stage().await;
    f.settle_stage().await;
    let before = snapshot(&mut f).await;
    let claim = f.claim_ix(1, u64::MAX, vec![], Some(initial), Some(vec![]));
    let user_state = claim.accounts[2].pubkey;
    let vesting_state = claim.accounts[3].pubkey;
    custom_error(
        send(&mut f.ctx, &[claim], &[&f.trader]).await,
        6000 + HoldNowCode::InvalidMerkleProof as u32,
    );
    assert_eq!(snapshot(&mut f).await, before);
    assert!(f
        .ctx
        .banks_client
        .get_account(user_state)
        .await
        .unwrap()
        .is_none());
    assert!(f
        .ctx
        .banks_client
        .get_account(vesting_state)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn direct_token_burn_and_unsolicited_curve_donations_do_not_corrupt_trading_accounts() {
    let mut f = Fixture::launched(Config {
        decay: 0,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    let burn = spl_token::instruction::burn(
        &spl_token::id(),
        &f.trader_tokens,
        &f.mint.pubkey(),
        &f.trader.pubkey(),
        &[],
        123,
    )
    .unwrap();
    let donate = spl_token::instruction::transfer(
        &spl_token::id(),
        &f.trader_tokens,
        &f.curve_tokens,
        &f.trader.pubkey(),
        &[],
        456,
    )
    .unwrap();
    send(&mut f.ctx, &[burn, donate], &[&f.trader])
        .await
        .unwrap();
    f.buy(SOL).await;
    let sale = f.sell_ix(f.trader.pubkey(), f.trader_tokens, 1_000_000, 0);
    send(&mut f.ctx, &[sale], &[&f.trader]).await.unwrap();
    let c = f.curve().await;
    assert_eq!(
        tokens(&mut f.ctx, f.curve_tokens).await,
        c.real_token_reserves + 456
    );
    assert_eq!(
        tokens(&mut f.ctx, f.curve_tokens).await
            + tokens(&mut f.ctx, f.reward_tokens).await
            + tokens(&mut f.ctx, f.trader_tokens).await,
        SUPPLY - 123
    );
}

#[tokio::test]
async fn multiple_swaps_in_one_signed_packet_are_valid_and_account_for_both_rewards() {
    let mut f = Fixture::launched(Config {
        decay: 0,
        ..Config::default()
    })
    .await;
    let buy = f.buy_ix(SOL, 0);
    let sell = f.sell_ix(f.trader.pubkey(), f.trader_tokens, 1_000_000, 0);
    let blockhash = f.ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[
            ComputeBudgetInstruction::set_compute_unit_limit(400_000),
            buy,
            sell,
        ],
        Some(&f.trader.pubkey()),
        &[&f.trader],
        blockhash,
    );
    assert!(bincode::serialize(&tx).unwrap().len() <= 1232);
    let result = f
        .ctx
        .banks_client
        .process_transaction_with_metadata(tx)
        .await
        .unwrap();
    result.result.unwrap();
    assert_eq!(
        tokens(&mut f.ctx, f.reward_tokens).await,
        f.curve().await.stage_reward_pools[0]
    );
    assert!(tokens(&mut f.ctx, f.trader_tokens).await > 0);
    assert!(f.curve().await.real_sol_reserves < 990_000_000);
}

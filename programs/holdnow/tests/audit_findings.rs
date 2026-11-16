#![allow(dead_code, unused_imports)]
//! Regression tests for the independently reproduced follow-up audit findings.
mod common;
use anchor_lang::{AccountDeserialize, InstructionData};
use common::*;
use contract::error::HoldNowCode;
use solana_sdk::{pubkey::Pubkey, signature::Signer};

async fn alternative_ata(f: &mut Fixture, owner: Pubkey) -> Pubkey {
    let address = spl_associated_token_account::address::get_associated_token_address(
        &owner,
        &f.mint.pubkey(),
    );
    let i = spl_associated_token_account::instruction::create_associated_token_account(
        &f.outsider.pubkey(),
        &owner,
        &f.mint.pubkey(),
        &spl_token::id(),
    );
    send(&mut f.ctx, &[i], &[&f.outsider]).await.unwrap();
    address
}
fn code(e: HoldNowCode) -> u32 {
    6000 + e as u32
}
async fn balances(f: &mut Fixture) -> [u64; 5] {
    [
        tokens(&mut f.ctx, f.curve_tokens).await,
        tokens(&mut f.ctx, f.reward_tokens).await,
        tokens(&mut f.ctx, f.trader_tokens).await,
        f.ctx.banks_client.get_balance(f.vault).await.unwrap(),
        f.ctx.banks_client.get_balance(f.fee).await.unwrap(),
    ]
}
#[tokio::test]
async fn fixed_f01_buy_and_claim_reject_alternative_reward_vault_atomically() {
    let mut f = Fixture::launched(Config::default()).await;
    let owner = f.reward_authority;
    let alternative = alternative_ata(&mut f, owner).await;
    f.buy(SOL).await;
    let before = balances(&mut f).await;
    let earned = f.curve().await.stage_reward_pools[0];
    let mut buy = f.buy_ix(SOL, 0);
    buy.accounts[3].pubkey = alternative;
    custom_error(send(&mut f.ctx, &[buy], &[&f.trader]).await, 2006);
    assert_eq!(balances(&mut f).await, before);
    assert_eq!(f.curve().await.stage_reward_pools[0], earned);
    f.end_stage().await;
    let root = f.root_ix(
        f.admin.pubkey(),
        1,
        Pubkey::new_from_array(regular_leaf(f.trader.pubkey(), f.mint.pubkey(), 1, earned)),
    );
    send(&mut f.ctx, &[root], &[&f.admin]).await.unwrap();
    let mut claim = f.claim_ix(1, earned, vec![], None, None);
    claim.accounts[5].pubkey = alternative;
    custom_error(send(&mut f.ctx, &[claim], &[&f.trader]).await, 2006);
    assert_eq!(f.curve().await.total_regular_claimed, 0);
    assert_eq!(tokens(&mut f.ctx, alternative).await, 0);
    let claim = f.claim_ix(1, earned, vec![], None, None);
    send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
    assert_eq!(f.curve().await.total_regular_claimed, earned);
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
}
#[tokio::test]
async fn fixed_f01_each_sell_vault_and_buy_source_rejects_substitution() {
    let mut f = Fixture::launched(Config {
        decay: 0,
        ..Config::default()
    })
    .await;
    let curve_owner = f.vault;
    let reward_owner = f.reward_authority;
    let alternative_curve = alternative_ata(&mut f, curve_owner).await;
    let alternative_reward = alternative_ata(&mut f, reward_owner).await;
    f.buy(SOL).await;
    let before = balances(&mut f).await;
    let amount = before[2] / 2;
    for (index, vault) in [(7, alternative_curve), (2, alternative_reward)] {
        let mut sell = f.sell_ix(f.trader.pubkey(), f.trader_tokens, amount, 0);
        sell.accounts[index].pubkey = vault;
        custom_error(send(&mut f.ctx, &[sell], &[&f.trader]).await, 2006);
        assert_eq!(balances(&mut f).await, before);
    }
    let mut buy = f.buy_ix(1_000_000, 0);
    buy.accounts[7].pubkey = alternative_curve;
    custom_error(send(&mut f.ctx, &[buy], &[&f.trader]).await, 2006);
    assert_eq!(balances(&mut f).await, before);
    assert_eq!(f.curve().await.real_token_reserves, before[0]);
    let sell = f.sell_ix(f.trader.pubkey(), f.trader_tokens, amount, 0);
    send(&mut f.ctx, &[sell], &[&f.trader]).await.unwrap();
    assert_eq!(
        f.curve().await.real_token_reserves,
        tokens(&mut f.ctx, f.curve_tokens).await
    );
    assert_eq!(tokens(&mut f.ctx, alternative_curve).await, 0);
    assert_eq!(tokens(&mut f.ctx, alternative_reward).await, 0);
}
#[tokio::test]
async fn fixed_f02_adjusted_reserve_overflow_rolls_back_launch_and_escrow() {
    let mut f = Fixture::initialized(Config {
        airdrop: SUPPLY / 2,
        root: Pubkey::new_unique(),
        ..Config::default()
    })
    .await;
    let params = f.params_ix(f.admin.pubkey(), 1, 1, SUPPLY, 1u64 << 63, SUPPLY);
    send(&mut f.ctx, &[params], &[&f.admin]).await.unwrap();
    let before = f.ctx.banks_client.get_balance(f.fee).await.unwrap();
    let create = f.create_ix();
    custom_error(
        send(&mut f.ctx, &[create], &[&f.creator]).await,
        code(HoldNowCode::MathOverflow),
    );
    assert_eq!(f.curve().await.token_total_supply, 0);
    assert_eq!(tokens(&mut f.ctx, f.creator_tokens).await, SUPPLY);
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, 0);
    assert_eq!(f.ctx.banks_client.get_balance(f.fee).await.unwrap(), before);
    assert!(f
        .ctx
        .banks_client
        .get_account(f.reward_tokens)
        .await
        .unwrap()
        .is_none());
    let params = f.params_ix(f.admin.pubkey(), 1, 1, SUPPLY, 30 * SOL, SUPPLY);
    send(&mut f.ctx, &[params], &[&f.admin]).await.unwrap();
    let create = f.create_ix();
    send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
    assert_eq!(f.curve().await.virtual_sol_reserves, 60 * SOL);
    f.buy(SOL).await;
}
#[tokio::test]
async fn fixed_f03_initializer_must_own_the_entire_immutable_supply() {
    let mut f = Fixture::minted(Config::default()).await;
    let init = f.initialize_curve_ix(f.outsider.pubkey(), 3);
    custom_error(send(&mut f.ctx, &[init], &[&f.outsider]).await, 2015);
    assert!(f
        .ctx
        .banks_client
        .get_account(f.curve)
        .await
        .unwrap()
        .is_none());
    let owner = f.outsider.pubkey();
    let empty = alternative_ata(&mut f, owner).await;
    let mut init = f.initialize_curve_ix(owner, 3);
    init.accounts.last_mut().unwrap().pubkey = empty;
    custom_error(
        send(&mut f.ctx, &[init], &[&f.outsider]).await,
        code(HoldNowCode::InsufficientFunds),
    );
    assert!(f
        .ctx
        .banks_client
        .get_account(f.curve)
        .await
        .unwrap()
        .is_none());
    let init = f.initialize_curve_ix(f.creator.pubkey(), 3);
    let create = f.create_ix();
    send(&mut f.ctx, &[init, create], &[&f.creator])
        .await
        .unwrap();
    assert_eq!(f.curve().await.creator, f.creator.pubkey());
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, SUPPLY);
}
#[tokio::test]
async fn fixed_f04_program_owned_configured_treasury_receives_buy_and_sell_fees() {
    let mut f = Fixture::launched(Config {
        decay: 0,
        ..Config::default()
    })
    .await;
    let mut params = f.params_ix(f.admin.pubkey(), 1, 1, SUPPLY, 30 * SOL, SUPPLY);
    params.data = contract::instruction::SetParams {
        fee_recipient: f.global,
        owner_wallet: f.admin.pubkey(),
        initial_virtual_token_reserves: SUPPLY,
        initial_virtual_sol_reserves: 30 * SOL,
        initial_real_token_reserves: SUPPLY,
        token_total_supply: SUPPLY,
        fee_basis_points: 1,
        create_fee: 0,
        staging_pool_fee: 1,
        minimum_migration_sol: contract::NEW_LAUNCH_MIN_MIGRATION_SOL,
    }
    .data();
    send(&mut f.ctx, &[params], &[&f.admin]).await.unwrap();
    f.fee = f.global;
    let initial = f.ctx.banks_client.get_balance(f.fee).await.unwrap();
    f.buy(SOL).await;
    let after_buy = f.ctx.banks_client.get_balance(f.fee).await.unwrap();
    assert_eq!(after_buy - initial, SOL / 100);
    let before = f.curve().await.real_sol_reserves;
    let amount = tokens(&mut f.ctx, f.trader_tokens).await / 2;
    let sell = f.sell_ix(f.trader.pubkey(), f.trader_tokens, amount, 0);
    send(&mut f.ctx, &[sell], &[&f.trader]).await.unwrap();
    assert!(f.curve().await.real_sol_reserves < before);
    assert!(f.ctx.banks_client.get_balance(f.fee).await.unwrap() > after_buy);
}

#[tokio::test]
async fn fixed_f05_outage_cannot_advance_unsettled_stage_and_late_claims_remain_open() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    f.end_stage().await;
    let c = f.curve().await;
    clock(&mut f.ctx, c.at_start_stage + c.claim_duration + 30 * 86400).await;
    let advance = f.stage_ix();
    custom_error(
        send(&mut f.ctx, &[advance], &[&f.outsider]).await,
        code(HoldNowCode::StageNotSettled),
    );
    assert_eq!(f.curve().await.current_stage, 1);
    assert!(!f.curve().await.complete);
    f.settle_stage().await;
    let advance = f.stage_ix();
    send(&mut f.ctx, &[advance], &[&f.outsider]).await.unwrap();
    assert!(f.curve().await.complete);
    clock(&mut f.ctx, c.at_start_stage + 10 * 365 * 86400).await;
    let claim = f.claim_ix(1, c.stage_reward_pools[0], vec![], None, None);
    send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
}
fn empty_settlement(f: &Fixture, stage: u8) -> solana_sdk::instruction::Instruction {
    let mut i = f.root_ix(f.admin.pubkey(), stage, Pubkey::default());
    i.data = contract::instruction::UpdateMerkleRoot {
        stage,
        merkle_root: Pubkey::default(),
        total_eligible_supply: 0,
    }
    .data();
    i
}
#[tokio::test]
async fn fixed_f05_no_recipient_rewards_roll_forward_once_without_double_budgeting() {
    let mut f = Fixture::launched(Config {
        stages: 2,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    f.end_stage().await;
    let earned = f.curve().await.stage_reward_pools[0];
    let settle = empty_settlement(&f, 1);
    send(&mut f.ctx, &[settle.clone()], &[&f.admin])
        .await
        .unwrap();
    let c = f.curve().await;
    assert!(c.stage_settled[1]);
    assert_eq!(c.stage_reward_rollovers[1], earned);
    assert_eq!(c.stage_reward_pools[0], 0);
    assert_eq!(c.stage_reward_pools[1], earned);
    custom_error(
        send(&mut f.ctx, &[settle], &[&f.admin]).await,
        code(HoldNowCode::MerkleRootAlreadySet),
    );
    f.next_stage().await;
    f.buy(SOL).await;
    f.end_stage().await;
    let total = f.curve().await.stage_reward_pools[1];
    assert!(total > earned);
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, total);
    f.next_stage().await;
    let claim = f.claim_ix(2, total, vec![], None, None);
    send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
    assert_eq!(f.curve().await.total_regular_claimed, total);
}
#[tokio::test]
async fn fixed_f05_empty_zero_reward_stage_settles_explicitly_and_immutably() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.end_stage().await;
    let settle = empty_settlement(&f, 1);
    send(&mut f.ctx, &[settle.clone()], &[&f.admin])
        .await
        .unwrap();
    assert!(f.curve().await.stage_settled[1]);
    assert_eq!(
        f.curve().await.cumulative_merkle_roots[1],
        Pubkey::default()
    );
    custom_error(
        send(&mut f.ctx, &[settle], &[&f.admin]).await,
        code(HoldNowCode::MerkleRootAlreadySet),
    );
    f.next_stage().await;
    assert!(f.curve().await.complete);
}

#[tokio::test]
async fn final_empty_rewards_burn_without_touching_old_regular_or_vested_claims() {
    use solana_sdk::program_pack::Pack;
    let initial = 900_000_000;
    let mut f = Fixture::initialized(Config {
        stages: 2,
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
    let old_rewards = f.curve().await.stage_reward_pools[0];
    f.next_stage().await;
    f.buy(SOL).await;
    f.end_stage().await;
    let to_burn = f.curve().await.stage_reward_pools[1];
    let curve_before = tokens(&mut f.ctx, f.curve_tokens).await;
    let sol_before = f.ctx.banks_client.get_balance(f.vault).await.unwrap();
    let settle = empty_settlement(&f, 2);
    send(&mut f.ctx, &[settle.clone()], &[&f.admin])
        .await
        .unwrap();
    assert_eq!(f.curve().await.total_unallocated_burned, to_burn);
    assert_eq!(f.curve().await.stage_reward_pools[1], 0);
    let mint = f
        .ctx
        .banks_client
        .get_account(f.mint.pubkey())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        spl_token::state::Mint::unpack(&mint.data).unwrap().supply,
        SUPPLY - to_burn
    );
    assert_eq!(
        tokens(&mut f.ctx, f.reward_tokens).await,
        initial + old_rewards
    );
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, curve_before);
    assert_eq!(
        f.ctx.banks_client.get_balance(f.vault).await.unwrap(),
        sol_before
    );
    custom_error(
        send(&mut f.ctx, &[settle], &[&f.admin]).await,
        code(HoldNowCode::MerkleRootAlreadySet),
    );
    f.next_stage().await;
    let c = f.curve().await;
    clock(&mut f.ctx, c.at_start_stage + 10 * 365 * 86400).await;
    let old_claim = f.claim_ix(1, old_rewards, vec![], None, None);
    send(&mut f.ctx, &[old_claim], &[&f.trader]).await.unwrap();
    let vesting = f.claim_ix(2, 0, vec![], Some(initial), Some(vec![]));
    send(&mut f.ctx, &[vesting], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
    assert_eq!(f.curve().await.total_regular_claimed, old_rewards);
    assert_eq!(f.curve().await.total_initial_claimed, initial);
}

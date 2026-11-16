mod common;
use anchor_lang::AccountSerialize;
use common::*;
use solana_sdk::{account::AccountSharedData, signature::Signer};

#[test]
fn net_liquidity_boundaries_preserve_legacy_and_new_terms() {
    use contract::instructions::redeem::price_matched_migration_liquidity;
    assert_eq!(contract::MIN_MIGRATION_SOL, SOL);
    assert_eq!(
        contract::NEW_LAUNCH_MIN_MIGRATION_SOL,
        if cfg!(feature = "devnet") {
            SOL
        } else {
            30 * SOL
        }
    );
    for minimum in [SOL, 30 * SOL] {
        let reserved = 600_000_000;
        let gross = ((minimum as u128 - 1) * 10000 / 9900 + 1) as u64;
        let check = |balance| {
            price_matched_migration_liquidity(
                SUPPLY,
                balance,
                reserved,
                6,
                minimum,
                SUPPLY,
                60 * SOL,
            )
            .unwrap()
        };
        assert!(!check(reserved + gross - 1).2);
        let (net, fee, ready) = check(reserved + gross);
        assert!(ready);
        assert_eq!(net, minimum);
        assert_eq!(net + fee, gross);
    }
}

#[tokio::test]
async fn new_launch_floor_rejects_lower_admin_and_stale_global_terms() {
    let floor = contract::NEW_LAUNCH_MIN_MIGRATION_SOL;
    let mut f = Fixture::initialized(Config {
        minimum_migration_sol: floor,
        ..Config::default()
    })
    .await;
    let mut params = f.params_ix(f.admin.pubkey(), 1, 1, SUPPLY, 30 * SOL, SUPPLY);
    let len = params.data.len();
    params.data[len - 8..].copy_from_slice(&(floor - 1).to_le_bytes());
    custom_error(
        send(&mut f.ctx, &[params], &[&f.admin]).await,
        6000 + contract::error::HoldNowCode::InvalidConfiguration as u32,
    );
    let mut g: contract::Global = state(&mut f.ctx, f.global).await;
    assert_eq!(g.minimum_migration_sol, floor);
    // Simulate an existing Global whose former default was not updated with the binary.
    let mut account = f
        .ctx
        .banks_client
        .get_account(f.global)
        .await
        .unwrap()
        .unwrap();
    g.minimum_migration_sol = floor - 1;
    g.try_serialize(&mut account.data.as_mut_slice()).unwrap();
    f.ctx
        .set_account(&f.global, &AccountSharedData::from(account));
    let mut create = f.create_ix();
    let len = create.data.len();
    create.data[len - 16..len - 8].copy_from_slice(&(floor - 1).to_le_bytes());
    custom_error(
        send(&mut f.ctx, &[create], &[&f.creator]).await,
        6000 + contract::error::HoldNowCode::InvalidConfiguration as u32,
    );
    assert_eq!(f.curve().await.token_total_supply, 0);
    let params = f.params_ix(f.admin.pubkey(), 1, 1, SUPPLY, 30 * SOL, SUPPLY);
    send(&mut f.ctx, &[params], &[&f.admin]).await.unwrap();
    let create = f.create_ix();
    send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
    assert_eq!(f.curve().await.minimum_migration_sol, floor);
}

#[tokio::test]
async fn trading_continues_above_target_through_every_scheduled_stage() {
    let floor = contract::NEW_LAUNCH_MIN_MIGRATION_SOL;
    let mut f = Fixture::launched(Config {
        minimum_migration_sol: floor,
        ..Config::default()
    })
    .await;
    f.buy(floor + 2 * SOL).await;
    assert!(f.curve().await.real_sol_reserves > floor);
    for stage in 1..=3 {
        let before = f.curve().await;
        assert_eq!(before.current_stage, stage);
        assert!(!before.complete);
        f.buy(SOL).await;
        let after = f.curve().await;
        assert_eq!(
            after.real_sol_reserves - before.real_sol_reserves,
            SOL - SOL / 100
        );
        let held = tokens(&mut f.ctx, f.trader_tokens).await;
        let sell = f.sell_ix(f.trader.pubkey(), f.trader_tokens, held / 100, 0);
        send(&mut f.ctx, &[sell], &[&f.trader]).await.unwrap();
        assert!(f.curve().await.real_sol_reserves < after.real_sol_reserves);
        f.end_stage().await;
        assert!(!f.curve().await.complete);
        f.next_stage().await;
    }
    let complete = f.curve().await;
    assert!(complete.complete);
    assert_eq!(complete.minimum_migration_sol, floor);
}

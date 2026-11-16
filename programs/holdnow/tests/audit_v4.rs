#![allow(dead_code, unused_imports)]
mod common;
use anchor_lang::InstructionData;
use common::*;
use solana_sdk::signature::Signer;

#[tokio::test]
async fn deployment_duration_is_exact_at_trading_and_pause_boundaries() {
    let mut f = Fixture::launched(Config {
        duration: 2,
        ..Config::default()
    })
    .await;
    let c = f.curve().await;
    let expected_unit = if cfg!(feature = "devnet") {
        120
    } else {
        86_400
    };
    assert_eq!(c.stage_duration, 2 * expected_unit);
    assert_eq!(c.minimum_migration_sol, SOL);
    let g: contract::Global = state(&mut f.ctx, f.global).await;
    assert_eq!(g.stage_unit_seconds as i64, expected_unit);
    clock(&mut f.ctx, c.at_start_stage + c.stage_duration - 1).await;
    f.buy(1_000_000).await;
    clock(&mut f.ctx, c.at_start_stage + c.stage_duration).await;
    let buy = f.buy_ix(1_000_000, 0);
    custom_error(
        send(&mut f.ctx, &[buy], &[&f.trader]).await,
        6000 + contract::error::HoldNowCode::StageExpired as u32,
    );
    f.end_stage().await;
    f.settle_stage().await;
    let paused = f.curve().await;
    clock(&mut f.ctx, paused.at_start_stage + 59).await;
    let advance = f.stage_ix();
    custom_error(
        send(&mut f.ctx, &[advance.clone()], &[&f.outsider]).await,
        6000 + contract::error::HoldNowCode::NotMoveNextStage as u32,
    );
    clock(&mut f.ctx, paused.at_start_stage + 60).await;
    send(&mut f.ctx, &[advance], &[&f.outsider]).await.unwrap();
    let next = f.curve().await;
    assert_eq!(next.current_stage, 2);
    assert_eq!(next.stage_duration, 2 * expected_unit);
    assert_eq!(next.at_start_stage, paused.at_start_stage + 60);
    f.buy(1_000_000).await;
}
fn settings(f: &Fixture, minimum: u64) -> solana_sdk::instruction::Instruction {
    let mut ix = f.params_ix(f.admin.pubkey(), 1, 1, SUPPLY, 30 * SOL, SUPPLY);
    ix.data = contract::instruction::SetParams {
        fee_recipient: f.fee,
        owner_wallet: f.admin.pubkey(),
        initial_virtual_token_reserves: SUPPLY,
        initial_virtual_sol_reserves: 30 * SOL,
        initial_real_token_reserves: SUPPLY,
        token_total_supply: SUPPLY,
        fee_basis_points: 1,
        create_fee: 6_900_000,
        staging_pool_fee: 1,
        minimum_migration_sol: minimum,
    }
    .data();
    ix
}
#[tokio::test]
async fn creator_accepts_immutable_minimum_and_global_changes_apply_only_to_future_launches() {
    let mut f = Fixture::initialized(Config::default()).await;
    let floor = contract::NEW_LAUNCH_MIN_MIGRATION_SOL;
    let invalid = settings(&f, floor - 1);
    assert!(send(&mut f.ctx, &[invalid], &[&f.admin]).await.is_err());
    let g: contract::Global = state(&mut f.ctx, f.global).await;
    assert_eq!(g.minimum_migration_sol, floor);
    let change = settings(&f, 2 * floor);
    send(&mut f.ctx, &[change], &[&f.admin]).await.unwrap();
    let before = tokens(&mut f.ctx, f.creator_tokens).await;
    let stale = f.create_ix();
    custom_error(
        send(&mut f.ctx, &[stale], &[&f.creator]).await,
        6000 + contract::error::HoldNowCode::InvalidConfiguration as u32,
    );
    assert_eq!(tokens(&mut f.ctx, f.creator_tokens).await, before);
    assert_eq!(f.curve().await.token_total_supply, 0);
    f.config.minimum_migration_sol = 2 * floor;
    let accepted = f.create_ix();
    send(&mut f.ctx, &[accepted], &[&f.creator]).await.unwrap();
    assert_eq!(f.curve().await.minimum_migration_sol, 2 * floor);
    let change = settings(&f, 3 * floor);
    send(&mut f.ctx, &[change], &[&f.admin]).await.unwrap();
    assert_eq!(f.curve().await.minimum_migration_sol, 2 * floor);
    assert_eq!(
        f.curve().await.stage_duration,
        contract::STAGE_UNIT_SECONDS as i64
    );
}

#![allow(dead_code, unused_imports)]
mod common;
use common::*;

#[tokio::test]
async fn production_length_stages_trade_until_the_actual_day_boundary_in_both_profiles() {
    for days in [1, 7] {
        let mut f = Fixture::launched(Config {
            duration: days * 86_400 / contract::STAGE_UNIT_SECONDS,
            ..Config::default()
        })
        .await;
        let curve = f.curve().await;
        assert_eq!(curve.stage_duration, days as i64 * 86_400);
        assert_eq!(curve.claim_duration, 60);
        clock(&mut f.ctx, curve.at_start_stage + curve.stage_duration - 1).await;
        f.buy(1_000_000).await;
        clock(&mut f.ctx, curve.at_start_stage + curve.stage_duration).await;
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
        assert_eq!(f.curve().await.stage_duration, days as i64 * 86_400);
        f.buy(1_000_000).await;
    }
}

#[tokio::test]
async fn excessive_duration_fails_before_moving_tokens_in_both_profiles() {
    let mut f = Fixture::initialized(Config {
        duration: contract::MAX_STAGE_UNITS + 1,
        ..Config::default()
    })
    .await;
    let before = tokens(&mut f.ctx, f.creator_tokens).await;
    let create = f.create_ix();
    custom_error(
        send(&mut f.ctx, &[create], &[&f.creator]).await,
        6000 + contract::error::HoldNowCode::InvalidConfiguration as u32,
    );
    assert_eq!(tokens(&mut f.ctx, f.creator_tokens).await, before);
    assert_eq!(f.curve().await.token_total_supply, 0);
}

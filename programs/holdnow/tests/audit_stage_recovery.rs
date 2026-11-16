#![allow(dead_code, unused_imports)]
mod common;
use common::*;
use contract::error::HoldNowCode;
use solana_sdk::signature::Signer;

#[tokio::test]
async fn delayed_settlement_and_repeated_cranks_preserve_every_stage_and_claim() {
    let mut f = Fixture::launched(Config {
        stages: 3,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    let holder_before = tokens(&mut f.ctx, f.trader_tokens).await;
    for stage in 1..=3 {
        let active = f.curve().await;
        assert_eq!(active.current_stage, stage);
        let ended_at = active.at_start_stage + active.stage_duration + 37;
        clock(&mut f.ctx, ended_at).await;
        let crank = f.stage_ix();
        send(&mut f.ctx, &[crank], &[&f.outsider]).await.unwrap();
        let paused = f.curve().await;
        assert!(paused.current_stage_complete);
        assert_eq!(paused.stage_completion_times[stage as usize], ended_at);
        let resumed_at = ended_at + paused.claim_duration + 86_400;
        clock(&mut f.ctx, resumed_at).await;
        let crank = f.stage_ix();
        custom_error(
            send(&mut f.ctx, &[crank], &[&f.outsider]).await,
            6000 + HoldNowCode::StageNotSettled as u32,
        );
        f.settle_stage().await;
        let crank = f.stage_ix();
        send(&mut f.ctx, &[crank], &[&f.outsider]).await.unwrap();
        let resumed = f.curve().await;
        assert_eq!(resumed.current_stage, stage + 1);
        assert_eq!(resumed.at_start_stage, resumed_at);
        assert_eq!(resumed.stage_completion_times[stage as usize], ended_at);
        assert!(resumed.stage_settled[stage as usize]);
        assert_eq!(resumed.complete, stage == 3);
        // A duplicate/replayed crank cannot skip the next trading stage.
        let crank = f.stage_ix();
        let expected = if stage == 3 {
            HoldNowCode::BondingCurveComplete
        } else {
            HoldNowCode::NotStageComplete
        };
        custom_error(
            send(&mut f.ctx, &[crank], &[&f.outsider]).await,
            6000 + expected as u32,
        );
    }
    let complete = f.curve().await;
    let reward: u64 = complete.stage_reward_pools.iter().sum();
    assert!(reward > 0);
    let claim = f.claim_ix(3, reward, vec![], None, None);
    send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
    assert_eq!(
        tokens(&mut f.ctx, f.trader_tokens).await,
        holder_before + reward
    );
    assert_eq!(f.curve().await.total_regular_claimed, reward);
}

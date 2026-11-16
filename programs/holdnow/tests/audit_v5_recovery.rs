#![allow(dead_code, unused_imports)]
mod common;
use anchor_lang::{solana_program::system_program, InstructionData};
use common::*;
use contract::error::HoldNowCode;
use solana_sdk::{
    account::{Account, AccountSharedData},
    instruction::AccountMeta,
    pubkey::Pubkey,
    rent::Rent,
    signature::Signer,
};

async fn install_config(f: &mut Fixture) {
    let key = Pubkey::find_program_address(&[b"amm_config_account_seed"], &contract::AMM_PROGRAM).0;
    f.ctx.set_account(
        &key,
        &AccountSharedData::from(Account {
            lamports: 10_000_000,
            data: vec![0; 544],
            owner: contract::AMM_PROGRAM,
            executable: false,
            rent_epoch: 0,
        }),
    );
}

#[tokio::test]
async fn funded_deadline_exit_ignores_external_config_and_preserves_all_late_claims() {
    for current_cluster_rent in [false, true] {
        let mut f = Fixture::initialized(Config {
            stages: 1,
            airdrop: 9_000_000_000,
            current_cluster_rent,
            ..Config::default()
        })
        .await;
        f.config.root = Pubkey::new_from_array(initial_leaf(
            f.trader.pubkey(),
            f.mint.pubkey(),
            f.config.airdrop,
        ));
        let create = f.create_ix();
        send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
        f.buy(contract::NEW_LAUNCH_MIN_MIGRATION_SOL + 2 * SOL)
            .await;
        f.end_stage().await;
        f.next_stage().await;
        install_config(&mut f).await;
        let c = f.curve().await;
        let deadline = c.migration_deadline().unwrap();
        assert_eq!(
            deadline,
            c.stage_completion_times[1]
                + c.claim_duration
                + contract::MIGRATION_GRACE_SECONDS as i64
        );
        let before = lamports(&mut f.ctx, f.vault).await;
        let activate = f.activate_redemption_ix();
        clock(&mut f.ctx, deadline - 1).await;
        error(
            send(&mut f.ctx, &[activate.clone()], &[&f.trader]).await,
            HoldNowCode::MigrationFunded,
        );
        assert!(!f.curve().await.redemption_mode);
        assert_eq!(lamports(&mut f.ctx, f.vault).await, before);
        // An unrelated existing account demonstrates that no external config/program
        // availability, ownership or layout can gate timed recovery.
        let mut recovery = activate;
        recovery.accounts[4] = AccountMeta::new_readonly(system_program::id(), false);
        clock(&mut f.ctx, deadline).await;
        send(&mut f.ctx, &[recovery.clone()], &[&f.trader])
            .await
            .unwrap();
        assert!(f.curve().await.redemption_mode);
        error(
            send(&mut f.ctx, &[recovery], &[&f.trader]).await,
            HoldNowCode::RedemptionActive,
        );
        let held = tokens(&mut f.ctx, f.trader_tokens).await;
        let redeem = f.redeem_ix(held, 1);
        send(&mut f.ctx, &[redeem], &[&f.trader]).await.unwrap();
        let claim = f.claim_ix(
            1,
            c.stage_reward_pools[0],
            vec![],
            Some(f.config.airdrop),
            Some(vec![]),
        );
        send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
        let held = tokens(&mut f.ctx, f.trader_tokens).await;
        let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
        let remaining = lamports(&mut f.ctx, f.vault).await - rent.minimum_balance(0);
        let redeem = f.redeem_ix(held, remaining);
        send(&mut f.ctx, &[redeem], &[&f.trader]).await.unwrap();
        assert_eq!(lamports(&mut f.ctx, f.vault).await, rent.minimum_balance(0));
        assert_eq!(f.curve().await.total_initial_claimed, f.config.airdrop);
        assert_eq!(
            f.curve().await.total_regular_claimed,
            c.stage_reward_pools[0]
        );
    }
}

#[tokio::test]
async fn delayed_publication_and_advancement_never_reset_deadline_or_bypass_settlement() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(10 * SOL).await;
    f.end_stage().await;
    let deadline = f.curve().await.migration_deadline().unwrap();
    clock(&mut f.ctx, deadline + 1).await;
    let activate = f.activate_redemption_ix();
    error(
        send(&mut f.ctx, &[activate.clone()], &[&f.trader]).await,
        HoldNowCode::BondingCurveNotComplete,
    );
    let advance = f.stage_ix();
    error(
        send(&mut f.ctx, &[advance.clone()], &[&f.outsider]).await,
        HoldNowCode::StageNotSettled,
    );
    f.settle_stage().await;
    let donate = solana_system_interface::instruction::transfer(&f.trader.pubkey(), &f.vault, SOL);
    send(&mut f.ctx, &[donate], &[&f.trader]).await.unwrap();
    // Complete and activate in a single holder transaction even if the backend is offline.
    let mut recovery = activate;
    recovery.accounts[4] = AccountMeta::new_readonly(system_program::id(), false);
    send(&mut f.ctx, &[advance, recovery], &[&f.outsider, &f.trader])
        .await
        .unwrap();
    let c = f.curve().await;
    assert!(c.complete && c.redemption_mode);
    assert_eq!(c.migration_deadline().unwrap(), deadline);
    assert_eq!(c.total_unallocated_burned, 0);
    assert!(c.stage_reward_pools[0] > 0);
}

#[tokio::test]
async fn creator_must_accept_the_exact_immutable_recovery_window() {
    let mut f = Fixture::initialized(Config::default()).await;
    // Reject stale 72-hour clients as well as a one-second terms mismatch.
    for wrong_grace in [contract::MIGRATION_GRACE_SECONDS + 1, 259_200] {
        let mut create = f.create_ix();
        let end = create.data.len();
        create.data[end - 8..].copy_from_slice(&wrong_grace.to_le_bytes());
        error(
            send(&mut f.ctx, &[create], &[&f.creator]).await,
            HoldNowCode::InvalidConfiguration,
        );
        assert_eq!(f.curve().await.token_total_supply, 0);
    }
    let create = f.create_ix();
    send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
    let params = f.params_ix(f.admin.pubkey(), 1, 1, SUPPLY, 30 * SOL, SUPPLY);
    send(&mut f.ctx, &[params], &[&f.admin]).await.unwrap();
    assert_eq!(
        f.curve().await.migration_grace_seconds,
        contract::MIGRATION_GRACE_SECONDS
    );
    assert!(f.curve().await.migration_deadline().is_err());
}

fn error(
    result: Result<(), solana_program_test::BanksClientError>,
    code: contract::error::HoldNowCode,
) {
    custom_error(result, 6000 + code as u32);
}

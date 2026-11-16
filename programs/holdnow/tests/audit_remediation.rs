#![allow(dead_code, unused_imports)]
use anchor_lang::solana_program::system_program;
// Economic exit, authoritative seller state and packet-safe proof regressions.
mod common;
use anchor_lang::{AccountDeserialize, InstructionData};
use common::*;
use contract::{
    error::HoldNowCode,
    instructions::redeem::{has_minimum_liquidity, redemption_output},
};
use solana_sdk::{
    account::{Account, AccountSharedData},
    instruction::Instruction,
    program_pack::Pack,
    pubkey::Pubkey,
    signature::Signer,
};
use spl_token::state::Mint;

fn config(f: &mut Fixture, fee: u64) {
    let mut data = vec![0; 544];
    data[536..].copy_from_slice(&fee.to_le_bytes());
    let key = Pubkey::find_program_address(&[b"amm_config_account_seed"], &contract::AMM_PROGRAM).0;
    f.ctx.set_account(
        &key,
        &AccountSharedData::from(Account {
            lamports: 10_000_000,
            data,
            owner: contract::AMM_PROGRAM,
            executable: false,
            rent_epoch: 0,
        }),
    );
}
async fn finished() -> Fixture {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(2 * SOL).await;
    f.end_stage().await;
    f.next_stage().await;
    f
}
#[test]
fn exact_liquidity_boundary_and_redemption_rounding() {
    assert!(!has_minimum_liquidity(1_000_000, 1_000_000, 6).unwrap());
    assert!(has_minimum_liquidity(1_000_001, 1_000_001, 6).unwrap());
    assert_eq!(
        redemption_output(u64::MAX, u64::MAX, u64::MAX).unwrap(),
        u64::MAX
    );
    assert!(redemption_output(10, 0, 0).is_err());
    assert!(redemption_output(10, 3, 2).is_err());
    for reserve in 1..100u64 {
        for supply in 2..50u64 {
            for first in 1..supply {
                let a = redemption_output(reserve, first, supply).unwrap();
                let b = redemption_output(reserve - a, supply - first, supply - first).unwrap();
                assert_eq!(a + b, reserve);
            }
        }
    }
}
#[tokio::test]
async fn redemption_preserves_late_rewards_and_cannot_be_reversed() {
    let mut f = finished().await;
    config(&mut f, 2 * SOL);
    let reserve =
        lamports(&mut f.ctx, f.vault).await - solana_sdk::rent::Rent::default().minimum_balance(0);
    let amount = tokens(&mut f.ctx, f.trader_tokens).await;
    let rewards = f.curve().await.stage_reward_pools[0];
    let activate = f.activate_redemption_ix();
    send(&mut f.ctx, &[activate], &[&f.trader]).await.unwrap();
    let expected = redemption_output(reserve, amount, amount + rewards).unwrap();
    let bad = f.redeem_ix(amount, expected + 1);
    custom_error(
        send(&mut f.ctx, &[bad], &[&f.trader]).await,
        6000 + HoldNowCode::TooLittleSolReceived as u32,
    );
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, amount);
    let redeem = f.redeem_ix(amount, expected);
    send(&mut f.ctx, &[redeem], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, 0);
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, rewards);
    assert_eq!(
        lamports(&mut f.ctx, f.vault).await,
        reserve - expected + solana_sdk::rent::Rent::default().minimum_balance(0)
    );
    let claim = f.claim_ix(1, rewards, vec![], None, None);
    send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
    let redeem = f.redeem_ix(rewards, reserve - expected);
    send(&mut f.ctx, &[redeem], &[&f.trader]).await.unwrap();
    assert_eq!(
        lamports(&mut f.ctx, f.vault).await,
        solana_sdk::rent::Rent::default().minimum_balance(0)
    );
    config(&mut f, 0);
    let activate = f.activate_redemption_ix();
    custom_error(
        send(&mut f.ctx, &[activate], &[&f.trader]).await,
        6000 + HoldNowCode::RedemptionActive as u32,
    );
}
#[tokio::test]
async fn funded_launch_and_forged_config_cannot_enable_redemption() {
    let mut f = finished().await;
    config(&mut f, 0);
    let activate = f.activate_redemption_ix();
    custom_error(
        send(&mut f.ctx, &[activate.clone()], &[&f.trader]).await,
        6000 + HoldNowCode::MigrationFunded as u32,
    );
    let mut forged = activate;
    forged.accounts[4].pubkey = f.vault;
    custom_error(
        send(&mut f.ctx, &[forged], &[&f.trader]).await,
        6000 + HoldNowCode::InvalidMigrationAccount as u32,
    );
    assert!(!f.curve().await.redemption_mode);
    let redeem = f.redeem_ix(1, 0);
    custom_error(
        send(&mut f.ctx, &[redeem], &[&f.trader]).await,
        6000 + HoldNowCode::RedemptionNotActive as u32,
    );
}
#[tokio::test]
async fn successful_sells_record_executed_stage_and_failed_sells_do_not() {
    let mut f = Fixture::launched(Config::default()).await;
    f.buy(SOL).await;
    let key = Pubkey::find_program_address(
        &[
            contract::USER_SALE_STATE_SEED,
            f.mint.pubkey().as_ref(),
            f.trader.pubkey().as_ref(),
        ],
        &contract::ID,
    )
    .0;
    let sale = f.sell_ix(f.trader.pubkey(), f.trader_tokens, 1_000_000, u64::MAX);
    assert!(send(&mut f.ctx, &[sale], &[&f.trader]).await.is_err());
    assert!(f.ctx.banks_client.get_account(key).await.unwrap().is_none());
    let sale = f.sell_ix(f.trader.pubkey(), f.trader_tokens, 1_000_000, 0);
    send(&mut f.ctx, &[sale], &[&f.trader]).await.unwrap();
    let data = f
        .ctx
        .banks_client
        .get_account(key)
        .await
        .unwrap()
        .unwrap()
        .data;
    assert_eq!(data.len(), contract::UserSaleState::SPACE);
    let record = contract::UserSaleState::try_deserialize(&mut data.as_slice()).unwrap();
    assert_eq!(record.last_sold_stage, 1);
    assert_eq!(record.user, f.trader.pubkey());
    f.end_stage().await;
    f.next_stage().await;
    let sale = f.sell_ix(f.trader.pubkey(), f.trader_tokens, 1_000_000, 0);
    send(&mut f.ctx, &[sale], &[&f.trader]).await.unwrap();
    let data = f
        .ctx
        .banks_client
        .get_account(key)
        .await
        .unwrap()
        .unwrap()
        .data;
    assert_eq!(
        contract::UserSaleState::try_deserialize(&mut data.as_slice())
            .unwrap()
            .last_sold_stage,
        2
    );
}

#[tokio::test]
async fn thirty_two_level_claim_requires_complete_bound_proof_and_refunds_buffer_rent() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    f.end_stage().await;
    let amount = f.curve().await.stage_reward_pools[0];
    let proof: Vec<[u8; 32]> = (0..32).map(|i| [i + 1; 32]).collect();
    let mut hash = regular_leaf(f.trader.pubkey(), f.mint.pubkey(), 1, amount);
    for node in &proof {
        hash = if hash <= *node {
            solana_sdk::keccak::hashv(&[&hash, node]).to_bytes()
        } else {
            solana_sdk::keccak::hashv(&[node, &hash]).to_bytes()
        };
    }
    let root = f.root_ix(f.admin.pubkey(), 1, Pubkey::new_from_array(hash));
    send(&mut f.ctx, &[root], &[&f.admin]).await.unwrap();
    let buffer = Pubkey::find_program_address(
        &[
            b"claim_proof",
            f.mint.pubkey().as_ref(),
            f.trader.pubkey().as_ref(),
            &[0],
        ],
        &contract::ID,
    )
    .0;
    for offset in [0, 16] {
        let write = ix(
            contract::accounts::WriteClaimProof {
                mint: f.mint.pubkey(),
                bonding_curve: f.curve,
                proof_buffer: buffer,
                user: f.trader.pubkey(),
                system_program: system_program::id(),
            },
            contract::instruction::WriteClaimProof {
                stage: 1,
                category: 0,
                amount,
                length: 32,
                offset,
                nodes: proof[offset as usize..offset as usize + 16].to_vec(),
            },
        );
        let packet = solana_sdk::transaction::Transaction::new_with_payer(
            &[write.clone()],
            Some(&f.trader.pubkey()),
        );
        assert!(bincode::serialized_size(&packet).unwrap() <= 1232);
        send(&mut f.ctx, &[write], &[&f.trader]).await.unwrap();
        if offset == 0 {
            let mut claim = f.claim_ix(1, amount, vec![], None, None);
            *claim.accounts.last_mut().unwrap() =
                solana_sdk::instruction::AccountMeta::new(buffer, false);
            custom_error(
                send(&mut f.ctx, &[claim], &[&f.trader]).await,
                6000 + HoldNowCode::InvalidMerkleProof as u32,
            );
        }
    }
    let mut wrong = f.claim_ix(1, amount - 1, vec![], None, None);
    *wrong.accounts.last_mut().unwrap() = solana_sdk::instruction::AccountMeta::new(buffer, false);
    custom_error(
        send(&mut f.ctx, &[wrong], &[&f.trader]).await,
        6000 + HoldNowCode::InvalidMerkleProof as u32,
    );
    let mut claim = f.claim_ix(1, amount, vec![], None, None);
    *claim.accounts.last_mut().unwrap() = solana_sdk::instruction::AccountMeta::new(buffer, false);
    send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
    assert!(f
        .ctx
        .banks_client
        .get_account(buffer)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn redemption_rejects_foreign_tokens_and_vaults_without_mutation() {
    let mut f = finished().await;
    config(&mut f, 2 * SOL);
    let activate = f.activate_redemption_ix();
    send(&mut f.ctx, &[activate], &[&f.trader]).await.unwrap();
    let balance = tokens(&mut f.ctx, f.trader_tokens).await;
    let sol = lamports(&mut f.ctx, f.vault).await;
    let mint_before = f
        .ctx
        .banks_client
        .get_account(f.mint.pubkey())
        .await
        .unwrap()
        .unwrap()
        .data;
    for (index, key) in [
        (4, f.creator_tokens),
        (3, f.reward_tokens),
        (2, f.outsider.pubkey()),
    ] {
        let mut forged = f.redeem_ix(balance, 1);
        forged.accounts[index].pubkey = key;
        assert!(send(&mut f.ctx, &[forged], &[&f.trader]).await.is_err());
        assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, balance);
        assert_eq!(lamports(&mut f.ctx, f.vault).await, sol);
        assert_eq!(
            f.ctx
                .banks_client
                .get_account(f.mint.pubkey())
                .await
                .unwrap()
                .unwrap()
                .data,
            mint_before
        );
    }
}

#[tokio::test]
async fn initial_proof_buffer_is_beneficiary_owned_resettable_and_cancellable() {
    let amount = 10_000_000;
    let mut f = Fixture::initialized(Config {
        stages: 1,
        airdrop: amount,
        ..Config::default()
    })
    .await;
    let proof: Vec<[u8; 32]> = (0..32).map(|i| [i + 1; 32]).collect();
    let mut hash = initial_leaf(f.trader.pubkey(), f.mint.pubkey(), amount);
    for node in &proof {
        hash = if hash <= *node {
            solana_sdk::keccak::hashv(&[&hash, node]).to_bytes()
        } else {
            solana_sdk::keccak::hashv(&[node, &hash]).to_bytes()
        };
    }
    f.config.root = Pubkey::new_from_array(hash);
    let create = f.create_ix();
    send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
    f.end_stage().await;
    let buffer = Pubkey::find_program_address(
        &[
            b"claim_proof",
            f.mint.pubkey().as_ref(),
            f.trader.pubkey().as_ref(),
            &[1],
        ],
        &contract::ID,
    )
    .0;
    let write = |f: &Fixture, offset: u8, length: u8, nodes: Vec<[u8; 32]>| {
        ix(
            contract::accounts::WriteClaimProof {
                mint: f.mint.pubkey(),
                bonding_curve: f.curve,
                proof_buffer: buffer,
                user: f.trader.pubkey(),
                system_program: system_program::id(),
            },
            contract::instruction::WriteClaimProof {
                stage: 1,
                category: 1,
                amount,
                length,
                offset,
                nodes,
            },
        )
    };
    for bad in [
        write(&f, 0, 33, proof[..16].to_vec()),
        write(&f, 31, 32, proof[..2].to_vec()),
        write(&f, 0, 32, proof[..17].to_vec()),
    ] {
        assert!(send(&mut f.ctx, &[bad], &[&f.trader]).await.is_err());
        assert!(f
            .ctx
            .banks_client
            .get_account(buffer)
            .await
            .unwrap()
            .is_none());
    }
    let first = write(&f, 0, 32, proof[..16].to_vec());
    let second = write(&f, 16, 32, proof[16..].to_vec());
    send(&mut f.ctx, &[first.clone()], &[&f.trader])
        .await
        .unwrap();
    send(&mut f.ctx, &[second.clone()], &[&f.trader])
        .await
        .unwrap();
    let mut forged = first.clone();
    forged.accounts[3].pubkey = f.outsider.pubkey();
    assert!(send(&mut f.ctx, &[forged], &[&f.outsider]).await.is_err());
    let close = ix(
        contract::accounts::CloseClaimProof {
            mint: f.mint.pubkey(),
            proof_buffer: buffer,
            user: f.trader.pubkey(),
        },
        contract::instruction::CloseClaimProof { _category: 1 },
    );
    let mut forged = close.clone();
    forged.accounts[2].pubkey = f.outsider.pubkey();
    assert!(send(&mut f.ctx, &[forged], &[&f.outsider]).await.is_err());
    // Replacing the beginning invalidates every old tail, even at the same length.
    send(&mut f.ctx, &[first.clone()], &[&f.trader])
        .await
        .unwrap();
    let mut claim = f.claim_ix(1, 0, vec![], Some(amount), Some(vec![]));
    *claim.accounts.last_mut().unwrap() = solana_sdk::instruction::AccountMeta::new(buffer, false);
    custom_error(
        send(&mut f.ctx, &[claim.clone()], &[&f.trader]).await,
        6000 + HoldNowCode::InvalidMerkleProof as u32,
    );
    let deposit = lamports(&mut f.ctx, buffer).await;
    let before = lamports(&mut f.ctx, f.trader.pubkey()).await;
    send(&mut f.ctx, &[close], &[&f.trader]).await.unwrap();
    assert_eq!(
        lamports(&mut f.ctx, f.trader.pubkey()).await,
        before + deposit
    );
    assert!(f
        .ctx
        .banks_client
        .get_account(buffer)
        .await
        .unwrap()
        .is_none());
    // A cancellation does not consume the entitlement; reprepare and claim it.
    send(&mut f.ctx, &[first], &[&f.trader]).await.unwrap();
    send(&mut f.ctx, &[second], &[&f.trader]).await.unwrap();
    send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, amount);
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
    assert!(f
        .ctx
        .banks_client
        .get_account(buffer)
        .await
        .unwrap()
        .is_none());
}

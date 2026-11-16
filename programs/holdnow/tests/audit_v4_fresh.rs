#![allow(dead_code, unused_imports)]
mod common;
use anchor_lang::solana_program::system_program;
use anchor_lang::{AccountDeserialize, InstructionData};
use common::*;
use contract::error::HoldNowCode;
use solana_program_test::{processor, ProgramTest};
use solana_sdk::{
    account::{Account, AccountSharedData},
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    program_error::ProgramError,
    program_pack::Pack,
    pubkey::Pubkey,
    rent::Rent,
    signature::Signer,
    transaction::{Transaction, TransactionError},
};
use spl_token::state::Mint;

const BOOTSTRAP: Pubkey = contract::deployment::BOOTSTRAP_AUTHORITY;
fn inspect_bootstrap(_id: &Pubkey, accounts: &[AccountInfo], _data: &[u8]) -> ProgramResult {
    assert_eq!(*accounts[0].owner, contract::ID);
    let g =
        contract::Global::try_deserialize(&mut accounts[0].try_borrow_data()?.as_ref()).unwrap();
    assert!(g.initialized);
    assert_eq!(g.authority, BOOTSTRAP);
    assert_eq!(g.protocol_version, contract::PROTOCOL_VERSION);
    assert_eq!(g.migration_grace_seconds, contract::MIGRATION_GRACE_SECONDS);
    assert_eq!(
        g.stage_unit_seconds,
        if cfg!(feature = "devnet") { 120 } else { 86400 }
    );
    assert_eq!(
        g.minimum_migration_sol,
        contract::NEW_LAUNCH_MIN_MIGRATION_SOL
    );
    assert_eq!(g.token_total_supply, 0);
    Ok(())
}

#[tokio::test]
async fn privileged_bootstrap_executes_in_sbf_simulation_without_exposing_a_private_key() {
    // Banks simulation deliberately skips cryptographic signature verification.
    // It exercises Anchor init/allocation and the real compiled handler, NOT live
    // authorization. The ordinary processing path below must reject these signatures.
    assert!(std::env::var_os("HOLDNOW_TEST_SBF").is_some());
    let owner: Pubkey = BOOTSTRAP;
    let global = Pubkey::find_program_address(&[contract::GLOBAL_STATE_SEED], &contract::ID).0;
    let probe = Pubkey::new_unique();
    let mut pt = ProgramTest::default();
    pt.prefer_bpf(true);
    pt.add_program("holdnow", contract::ID, None);
    pt.prefer_bpf(false);
    pt.add_program("inspect_bootstrap", probe, processor!(inspect_bootstrap));
    pt.add_account(owner, Account::new(SOL, 0, &system_program::id()));
    let ctx = pt.start_with_context().await;
    let initialize = ix(
        contract::accounts::Initialize {
            global,
            owner,
            system_program: system_program::id(),
        },
        contract::instruction::Initialize {},
    );
    let inspect = Instruction {
        program_id: probe,
        accounts: vec![AccountMeta::new_readonly(global, false)],
        data: vec![],
    };
    let mut tx = Transaction::new_with_payer(
        &[initialize.clone(), inspect.clone()],
        Some(&ctx.payer.pubkey()),
    );
    tx.message.recent_blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    assert!(tx.verify().is_err());
    let result = ctx
        .banks_client
        .simulate_transaction(tx.clone())
        .await
        .unwrap();
    assert_eq!(result.result, Some(Ok(())), "{result:?}");
    assert!(result.simulation_details.unwrap().units_consumed < 400_000);
    assert!(ctx
        .banks_client
        .get_account(global)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        ctx.banks_client
            .process_transaction(tx)
            .await
            .unwrap_err()
            .unwrap(),
        TransactionError::SignatureFailure
    );
    let mut repeat = Transaction::new_with_payer(
        &[initialize.clone(), inspect, initialize],
        Some(&ctx.payer.pubkey()),
    );
    repeat.message.recent_blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    assert!(ctx
        .banks_client
        .simulate_transaction(repeat)
        .await
        .unwrap()
        .result
        .unwrap()
        .is_err());
}

fn minimum_settings(f: &Fixture, minimum: u64) -> Instruction {
    let mut i = f.params_ix(f.admin.pubkey(), 1, 1, SUPPLY, 30 * SOL, SUPPLY);
    i.data = contract::instruction::SetParams {
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
    i
}
async fn canonical_config(f: &mut Fixture) -> u64 {
    let cluster = if cfg!(feature = "devnet") {
        "devnet"
    } else {
        "mainnet-beta"
    };
    let data = std::fs::read(format!("../../audit/external/{cluster}-config.bin")).unwrap();
    let fee = u64::from_le_bytes(data[536..544].try_into().unwrap());
    let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
    let address =
        Pubkey::find_program_address(&[b"amm_config_account_seed"], &contract::AMM_PROGRAM).0;
    f.ctx.set_account(
        &address,
        &AccountSharedData::from(Account {
            lamports: rent.minimum_balance(data.len()),
            data,
            owner: contract::AMM_PROGRAM,
            executable: false,
            rent_epoch: 0,
        }),
    );
    fee + contract::MIGRATION_ACCOUNT_SIZES
        .iter()
        .map(|&size| rent.minimum_balance(size))
        .sum::<u64>()
        + rent.minimum_balance(0)
}
async fn snapshot(f: &mut Fixture, keys: &[Pubkey]) -> Vec<Option<Account>> {
    let mut values = vec![];
    for key in keys {
        values.push(f.ctx.banks_client.get_account(*key).await.unwrap());
    }
    values
}

#[tokio::test]
async fn lowering_global_default_cannot_remove_an_existing_holders_redemption_option() {
    let floor = contract::NEW_LAUNCH_MIN_MIGRATION_SOL;
    for current_cluster_rent in [false, true] {
        let mut f = Fixture::initialized(Config {
            stages: 1,
            current_cluster_rent,
            ..Config::default()
        })
        .await;
        let terms = minimum_settings(&f, 2 * floor);
        send(&mut f.ctx, &[terms], &[&f.admin]).await.unwrap();
        f.config.minimum_migration_sol = 2 * floor;
        let create = f.create_ix();
        send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
        f.buy(floor + SOL).await;
        f.end_stage().await;
        f.next_stage().await;
        let reserved = canonical_config(&mut f).await;
        let lower = minimum_settings(&f, floor);
        send(&mut f.ctx, &[lower], &[&f.admin]).await.unwrap();
        let balance = lamports(&mut f.ctx, f.vault).await;
        let pool_tokens = tokens(&mut f.ctx, f.curve_tokens).await;
        assert!(
            contract::instructions::redeem::migration_liquidity(
                pool_tokens,
                balance,
                reserved,
                6,
                floor
            )
            .unwrap()
            .2
        );
        assert!(
            !contract::instructions::redeem::migration_liquidity(
                pool_tokens,
                balance,
                reserved,
                6,
                2 * floor
            )
            .unwrap()
            .2
        );
        let activate = f.activate_redemption_ix();
        send(&mut f.ctx, &[activate], &[&f.trader]).await.unwrap();
        assert_eq!(f.curve().await.minimum_migration_sol, 2 * floor);
        let reward = f.curve().await.stage_reward_pools[0];
        let claim = f.claim_ix(1, reward, vec![], None, None);
        send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
        let all = tokens(&mut f.ctx, f.trader_tokens).await;
        let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
        let redeem = f.redeem_ix(all, balance - rent.minimum_balance(0));
        send(&mut f.ctx, &[redeem], &[&f.trader]).await.unwrap();
        assert_eq!(lamports(&mut f.ctx, f.vault).await, rent.minimum_balance(0));
    }
}

#[tokio::test]
async fn redemption_handles_direct_burns_transfers_dust_late_vesting_and_new_sol() {
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
        f.buy(500_000_000).await;
        let held = tokens(&mut f.ctx, f.trader_tokens).await;
        let mint = f.mint.pubkey();
        let outsider_tokens = spl_associated_token_account::address::get_associated_token_address(
            &f.outsider.pubkey(),
            &mint,
        );
        let ata = spl_associated_token_account::instruction::create_associated_token_account(
            &f.ctx.payer.pubkey(),
            &f.outsider.pubkey(),
            &mint,
            &spl_token::id(),
        );
        send(&mut f.ctx, &[ata], &[]).await.unwrap();
        let transfer = |from: Pubkey, to: Pubkey, user: Pubkey, n: u64| {
            spl_token::instruction::transfer(&spl_token::id(), &from, &to, &user, &[], n).unwrap()
        };
        let burn = spl_token::instruction::burn(
            &spl_token::id(),
            &f.trader_tokens,
            &mint,
            &f.trader.pubkey(),
            &[],
            held / 9,
        )
        .unwrap();
        let actions = [
            transfer(
                f.trader_tokens,
                f.creator_tokens,
                f.trader.pubkey(),
                held / 3,
            ),
            transfer(f.trader_tokens, outsider_tokens, f.trader.pubkey(), 1),
            burn,
            transfer(
                f.trader_tokens,
                f.curve_tokens,
                f.trader.pubkey(),
                held / 13,
            ),
        ];
        send(&mut f.ctx, &actions, &[&f.trader]).await.unwrap();
        f.end_stage().await;
        f.next_stage().await;
        canonical_config(&mut f).await;
        let activate = f.activate_redemption_ix();
        send(&mut f.ctx, &[activate], &[&f.trader]).await.unwrap();
        let keys = [
            f.curve,
            f.vault,
            mint,
            f.trader_tokens,
            f.creator_tokens,
            outsider_tokens,
            f.reward_tokens,
        ];
        let before = snapshot(&mut f, &keys).await;
        let mut dust = f.redeem_ix(1, 0);
        dust.accounts[4].pubkey = outsider_tokens;
        dust.accounts[5].pubkey = f.outsider.pubkey();
        custom_error(
            send(&mut f.ctx, &[dust], &[&f.outsider]).await,
            6000 + HoldNowCode::ZeroAmount as u32,
        );
        assert_eq!(snapshot(&mut f, &keys).await, before);
        let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
        let available = lamports(&mut f.ctx, f.vault).await - rent.minimum_balance(0);
        let mint_state = Mint::unpack(
            &f.ctx
                .banks_client
                .get_account(mint)
                .await
                .unwrap()
                .unwrap()
                .data,
        )
        .unwrap();
        let denominator = mint_state.supply - tokens(&mut f.ctx, f.curve_tokens).await;
        let creator_amount = tokens(&mut f.ctx, f.creator_tokens).await;
        let expected = (available as u128 * creator_amount as u128 / denominator as u128) as u64;
        let mut first = f.redeem_ix(creator_amount, expected);
        first.accounts[4].pubkey = f.creator_tokens;
        first.accounts[5].pubkey = f.creator.pubkey();
        send(&mut f.ctx, &[first], &[&f.creator]).await.unwrap();
        assert_eq!(
            lamports(&mut f.ctx, f.vault).await,
            available - expected + rent.minimum_balance(0)
        );
        let donation =
            solana_system_interface::instruction::transfer(&f.creator.pubkey(), &f.vault, 123456);
        send(&mut f.ctx, &[donation], &[&f.creator]).await.unwrap();
        let reward = f.curve().await.stage_reward_pools[0];
        let claim = f.claim_ix(1, reward, vec![], Some(f.config.airdrop), Some(vec![]));
        send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
        let return_dust = transfer(outsider_tokens, f.trader_tokens, f.outsider.pubkey(), 1);
        send(&mut f.ctx, &[return_dust], &[&f.outsider])
            .await
            .unwrap();
        let all = tokens(&mut f.ctx, f.trader_tokens).await;
        let last = f.redeem_ix(all, available - expected + 123456);
        send(&mut f.ctx, &[last], &[&f.trader]).await.unwrap();
        assert_eq!(lamports(&mut f.ctx, f.vault).await, rent.minimum_balance(0));
        assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
        let mint_state = Mint::unpack(
            &f.ctx
                .banks_client
                .get_account(mint)
                .await
                .unwrap()
                .unwrap()
                .data,
        )
        .unwrap();
        assert_eq!(mint_state.supply, tokens(&mut f.ctx, f.curve_tokens).await);
    }
}

#[tokio::test]
async fn malformed_claim_wire_lengths_options_and_truncations_reject_without_mutation() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    f.end_stage().await;
    f.next_stage().await;
    let reward = f.curve().await.stage_reward_pools[0];
    let valid = f.claim_ix(1, reward, vec![], None, None);
    assert_eq!(valid.data.len(), 23);
    let keys = [
        f.curve,
        f.vault,
        f.mint.pubkey(),
        f.trader_tokens,
        f.reward_tokens,
        valid.accounts[2].pubkey,
        valid.accounts[3].pubkey,
    ];
    let before = snapshot(&mut f, &keys).await;
    let mut payloads = vec![];
    for length in 0..valid.data.len() {
        payloads.push(valid.data[..length].to_vec());
    }
    for length in [1u32, 2, 16, 32, 33, 65535, u32::MAX] {
        let mut data = valid.data.clone();
        data[17..21].copy_from_slice(&length.to_le_bytes());
        payloads.push(data);
    }
    for tag in [2u8, 3, 127, 255] {
        for offset in [21usize, 22] {
            let mut data = valid.data.clone();
            data[offset] = tag;
            payloads.push(data);
        }
    }
    for data in payloads {
        let mut malformed = valid.clone();
        malformed.data = data;
        assert!(send(&mut f.ctx, &[malformed], &[&f.trader]).await.is_err());
        assert_eq!(snapshot(&mut f, &keys).await, before);
    }
    send(&mut f.ctx, &[valid], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
}

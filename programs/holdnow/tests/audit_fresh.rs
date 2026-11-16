#![allow(dead_code, unused_imports)]
//! Independent v3 audit: production instructions execute from the compiled SBPF binary.
//! The original finding reproductions are preserved in commit 60d9a7f; these assert the repairs.
mod common;
use anchor_lang::solana_program::system_program;
use anchor_lang::{AccountDeserialize, InstructionData};
use common::*;
use contract::error::HoldNowCode;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::{
    account::{Account, AccountSharedData},
    instruction::{AccountMeta, Instruction},
    program_pack::Pack,
    pubkey::Pubkey,
    rent::Rent,
    signature::{Keypair, Signer},
    transaction::{Transaction, TransactionError},
};
use solana_system_interface::instruction as system_instruction;
use spl_token::state::Mint;

fn config(f: &mut Fixture, creation_fee: u64) {
    let key = Pubkey::find_program_address(&[b"amm_config_account_seed"], &contract::AMM_PROGRAM).0;
    let mut data = vec![0; 544];
    data[536..].copy_from_slice(&creation_fee.to_le_bytes());
    f.ctx.set_account(
        &key,
        &AccountSharedData::from(Account {
            lamports: Rent::default().minimum_balance(544),
            data,
            owner: contract::AMM_PROGRAM,
            executable: false,
            rent_epoch: 0,
        }),
    );
}
fn pair(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    if a < b {
        solana_sdk::keccak::hashv(&[&a, &b]).to_bytes()
    } else {
        solana_sdk::keccak::hashv(&[&b, &a]).to_bytes()
    }
}
async fn account_bytes(f: &mut Fixture, keys: &[Pubkey]) -> Vec<Option<(u64, Vec<u8>)>> {
    let mut result = Vec::new();
    for key in keys {
        result.push(
            f.ctx
                .banks_client
                .get_account(*key)
                .await
                .unwrap()
                .map(|a| (a.lamports, a.data)),
        );
    }
    result
}
fn claim_for(
    f: &Fixture,
    user: Pubkey,
    token_account: Pubkey,
    stage: u8,
    amount: u64,
    proof: Vec<[u8; 32]>,
) -> Instruction {
    let mut claim = f.claim_ix(stage, amount, proof, None, None);
    claim.accounts[2].pubkey = Pubkey::find_program_address(
        &[b"user_claim_state", f.mint.pubkey().as_ref(), user.as_ref()],
        &contract::ID,
    )
    .0;
    claim.accounts[3].pubkey = Pubkey::find_program_address(
        &[
            b"user_vesting_state",
            f.mint.pubkey().as_ref(),
            user.as_ref(),
        ],
        &contract::ID,
    )
    .0;
    claim.accounts[6].pubkey = token_account;
    claim.accounts[7].pubkey = user;
    claim
}
fn redeem_for(
    f: &Fixture,
    user: Pubkey,
    token_account: Pubkey,
    amount: u64,
    minimum: u64,
) -> Instruction {
    let mut ix = f.redeem_ix(amount, minimum);
    ix.accounts[4].pubkey = token_account;
    ix.accounts[5].pubkey = user;
    ix
}

#[tokio::test]
async fn fixed_v3_02_launch_funds_only_missing_rent_and_tiny_trades_round_trip() {
    for current_cluster_rent in [false, true] {
        for funding in 0..4 {
            let mut f = Fixture::minted_with_vault_prefunding(
                Config {
                    tax_max: 0,
                    current_cluster_rent,
                    ..Config::default()
                },
                false,
            )
            .await;
            let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
            let cushion = rent.minimum_balance(0);
            let prefund = match funding {
                0 => 0,
                1 => 1,
                2 => cushion,
                _ => cushion + 123,
            };
            let params = f.params_ix(f.admin.pubkey(), 0, 0, SUPPLY, 30 * SOL, SUPPLY);
            send(&mut f.ctx, &[params], &[&f.admin]).await.unwrap();
            let before = lamports(&mut f.ctx, f.creator.pubkey()).await;
            let mut instructions = vec![];
            if prefund > 0 {
                // A partial prefund is admissible in the same atomic transaction.
                instructions.push(system_instruction::transfer(
                    &f.creator.pubkey(),
                    &f.vault,
                    prefund,
                ));
            }
            instructions.push(f.initialize_curve_ix(f.creator.pubkey(), f.config.stages));
            send(&mut f.ctx, &instructions, &[&f.creator])
                .await
                .unwrap();
            let funded = cushion.max(prefund);
            assert_eq!(lamports(&mut f.ctx, f.vault).await, funded);
            assert_eq!(
                before - lamports(&mut f.ctx, f.creator.pubkey()).await,
                lamports(&mut f.ctx, f.curve).await
                    + lamports(&mut f.ctx, f.curve_tokens).await
                    + funded
            );
            let create = f.create_ix();
            send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
            assert_eq!(f.curve().await.real_sol_reserves, 0);
            f.buy(1).await; // below either rent minimum, now valid even for a direct client
            f.buy(1_000_000).await;
            let held = tokens(&mut f.ctx, f.trader_tokens).await;
            let c = f.curve().await;
            let quote = contract::utils::curve_output(
                held,
                c.virtual_token_reserves,
                c.virtual_sol_reserves,
            )
            .unwrap();
            let sale = f.sell_ix(f.trader.pubkey(), f.trader_tokens, held, quote);
            send(&mut f.ctx, &[sale], &[&f.trader]).await.unwrap();
            assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, 0);
            let c = f.curve().await;
            assert_eq!(
                lamports(&mut f.ctx, f.vault).await,
                funded + c.real_sol_reserves
            );
        }
    }
}

#[tokio::test]
async fn fixed_v3_03_backend_allocation_claims_and_redemption_leave_no_dust() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    let held = tokens(&mut f.ctx, f.trader_tokens).await;
    let split = held / 2;
    let transfer = spl_token::instruction::transfer(
        &spl_token::id(),
        &f.trader_tokens,
        &f.creator_tokens,
        &f.trader.pubkey(),
        &[],
        split,
    )
    .unwrap();
    send(&mut f.ctx, &[transfer], &[&f.trader]).await.unwrap();
    f.end_stage().await;
    let budget = f.curve().await.stage_reward_pools[0];
    // Execute the production backend allocator, including its budget validator.
    let project = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let node = std::env::var_os("HOLDNOW_TEST_NODE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| project.join(".audit-tools/node/bin/node"));
    let input = serde_json::json!({ "pool": budget.to_string(), "balances": [
        [f.trader.pubkey().to_string(), (held - split).to_string()],
        [f.creator.pubkey().to_string(), split.to_string()]], "blacklisted": [] });
    let output = std::process::Command::new(node)
        .arg(project.join("backend/node_modules/tsx/dist/cli.mjs"))
        .arg(project.join("backend/tests/fixtures/buildRewardAllocations.ts"))
        .arg(input.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let award = |key: Pubkey| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|row| row["user"].as_str().unwrap() == key.to_string())
            .unwrap()["stageAmount"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    let a = award(f.trader.pubkey());
    let b = award(f.creator.pubkey());
    assert_eq!(a + b, budget);
    let old_floor_sum = ((budget as u128) * (held - split) as u128 / held as u128)
        + ((budget as u128) * split as u128 / held as u128);
    assert!(old_floor_sum < budget as u128); // this exact fixture stranded dust before the fix
    let leaf_a = regular_leaf(f.trader.pubkey(), f.mint.pubkey(), 1, a);
    let leaf_b = regular_leaf(f.creator.pubkey(), f.mint.pubkey(), 1, b);
    let mut root = f.root_ix(
        f.admin.pubkey(),
        1,
        Pubkey::new_from_array(pair(leaf_a, leaf_b)),
    );
    root.data = contract::instruction::UpdateMerkleRoot {
        stage: 1,
        merkle_root: Pubkey::new_from_array(pair(leaf_a, leaf_b)),
        total_eligible_supply: held,
    }
    .data();
    send(&mut f.ctx, &[root], &[&f.admin]).await.unwrap();
    f.next_stage().await;
    config(&mut f, 2 * SOL);
    let activate = f.activate_redemption_ix();
    send(&mut f.ctx, &[activate], &[&f.trader]).await.unwrap();
    for (user, token, amount, proof, creator) in [
        (f.trader.pubkey(), f.trader_tokens, a, leaf_b, false),
        (f.creator.pubkey(), f.creator_tokens, b, leaf_a, true),
    ] {
        let claim = claim_for(&f, user, token, 1, amount, vec![proof]);
        if creator {
            send(&mut f.ctx, &[claim], &[&f.creator]).await.unwrap();
        } else {
            send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
        }
    }
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
    for (user, token, creator) in [
        (f.trader.pubkey(), f.trader_tokens, false),
        (f.creator.pubkey(), f.creator_tokens, true),
    ] {
        let held = tokens(&mut f.ctx, token).await;
        let redeem = redeem_for(&f, user, token, held, 1);
        if creator {
            send(&mut f.ctx, &[redeem], &[&f.creator]).await.unwrap();
        } else {
            send(&mut f.ctx, &[redeem], &[&f.trader]).await.unwrap();
        }
    }
    let remaining = lamports(&mut f.ctx, f.vault).await - Rent::default().minimum_balance(0);
    assert_eq!(remaining, 0);
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, 0);
    assert_eq!(tokens(&mut f.ctx, f.creator_tokens).await, 0);
    let c = f.curve().await;
    assert_eq!(c.total_regular_claimed, budget);
    assert_eq!(c.total_unallocated_burned, 0);
    let replay = f.claim_ix(1, a, vec![leaf_b], None, None);
    custom_error(
        send(&mut f.ctx, &[replay], &[&f.trader]).await,
        6000 + HoldNowCode::NothingToClaim as u32,
    );
    println!("REWARD_CONSERVATION unassigned_tokens=0 locked_lamports={remaining}");
}

#[tokio::test]
async fn hundred_stage_lifecycle_with_thirty_two_level_vesting_fits_real_client_limits() {
    let amount = 900_000_000;
    let mut f = Fixture::initialized(Config {
        stages: 100,
        airdrop: amount,
        ..Config::default()
    })
    .await;
    let proof: Vec<[u8; 32]> = (0..32).map(|i| [i + 1; 32]).collect();
    let mut root = initial_leaf(f.trader.pubkey(), f.mint.pubkey(), amount);
    for node in &proof {
        root = pair(root, *node);
    }
    f.config.root = Pubkey::new_from_array(root);
    let create = f.create_ix();
    send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
    for _ in 0..100 {
        f.end_stage().await;
        f.next_stage().await;
    }
    let c = f.curve().await;
    assert!(c.complete);
    assert_eq!(c.current_stage, 101);
    assert!(c.stage_settled[1..].iter().all(|v| *v));
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
    let project = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let node = std::env::var_os("HOLDNOW_TEST_NODE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| project.join("../.audit-tools/node/bin/node"));
    let input = serde_json::json!({ "mint": f.mint.pubkey().to_string(), "user": f.trader.pubkey().to_string(),
        "userTokenAccount": f.trader_tokens.to_string(), "programId": contract::ID.to_string(), "stage": 100,
        "amount": amount.to_string(), "proof": proof });
    let output = std::process::Command::new(node)
        .arg(project.join("programs/holdnow/tests/fixtures/build_claim_packets.cjs"))
        .arg(input.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let encoded: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let sdk: Vec<Instruction> = encoded
        .as_array()
        .unwrap()
        .iter()
        .map(|value| Instruction {
            program_id: value["programId"].as_str().unwrap().parse().unwrap(),
            data: value["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| b.as_u64().unwrap() as u8)
                .collect(),
            accounts: value["accounts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|meta| AccountMeta {
                    pubkey: meta["pubkey"].as_str().unwrap().parse().unwrap(),
                    is_signer: meta["isSigner"].as_bool().unwrap(),
                    is_writable: meta["isWritable"].as_bool().unwrap(),
                })
                .collect(),
        })
        .collect();
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
                stage: 100,
                category: 1,
                amount,
                length: 32,
                offset,
                nodes: proof[offset as usize..offset as usize + 16].to_vec(),
            },
        );
        assert_eq!(write, sdk[offset as usize / 16]);
        let hash = f.ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[sdk[offset as usize / 16].clone()],
            Some(&f.trader.pubkey()),
            &[&f.trader],
            hash,
        );
        assert!(bincode::serialize(&tx).unwrap().len() <= 1232);
        f.ctx.banks_client.process_transaction(tx).await.unwrap(); // normal 200k instruction budget
    }
    let mut claim = f.claim_ix(100, 0, vec![], Some(amount), Some(vec![]));
    *claim.accounts.last_mut().unwrap() = AccountMeta::new(buffer, false);
    assert_eq!(claim, sdk[2]);
    let claim = sdk[2].clone();
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
    assert!(bincode::serialize(&tx).unwrap().len() <= 1232);
    let result = f
        .ctx
        .banks_client
        .process_transaction_with_metadata(tx)
        .await
        .unwrap();
    result.result.unwrap();
    println!(
        "MAX_STAGE_PROOF_COMPUTE {}",
        result.metadata.unwrap().compute_units_consumed
    );
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, amount);
    assert!(f
        .ctx
        .banks_client
        .get_account(buffer)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn sale_history_survives_token_account_closure_recreation_and_rebuy() {
    let mut f = Fixture::launched(Config {
        tax_max: 0,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    let amount = tokens(&mut f.ctx, f.trader_tokens).await;
    let sale = f.sell_ix(f.trader.pubkey(), f.trader_tokens, amount, 1);
    send(&mut f.ctx, &[sale], &[&f.trader]).await.unwrap();
    let record = Pubkey::find_program_address(
        &[
            b"user_sale_state",
            f.mint.pubkey().as_ref(),
            f.trader.pubkey().as_ref(),
        ],
        &contract::ID,
    )
    .0;
    let before = f
        .ctx
        .banks_client
        .get_account(record)
        .await
        .unwrap()
        .unwrap();
    let close = spl_token::instruction::close_account(
        &spl_token::id(),
        &f.trader_tokens,
        &f.trader.pubkey(),
        &f.trader.pubkey(),
        &[],
    )
    .unwrap();
    send(&mut f.ctx, &[close], &[&f.trader]).await.unwrap();
    let create = spl_associated_token_account::instruction::create_associated_token_account(
        &f.trader.pubkey(),
        &f.trader.pubkey(),
        &f.mint.pubkey(),
        &spl_token::id(),
    );
    send(&mut f.ctx, &[create], &[&f.trader]).await.unwrap();
    f.buy(SOL).await;
    assert_eq!(
        f.ctx
            .banks_client
            .get_account(record)
            .await
            .unwrap()
            .unwrap(),
        before
    );
    let state = contract::UserSaleState::try_deserialize(&mut before.data.as_slice()).unwrap();
    assert_eq!(state.last_sold_stage, 1);
}

#[tokio::test]
async fn delegated_token_authority_cannot_redirect_redemption_and_failed_burn_is_atomic() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    f.end_stage().await;
    f.next_stage().await;
    config(&mut f, 2 * SOL);
    let activate = f.activate_redemption_ix();
    send(&mut f.ctx, &[activate], &[&f.trader]).await.unwrap();
    let held = tokens(&mut f.ctx, f.trader_tokens).await;
    let approve = spl_token::instruction::approve(
        &spl_token::id(),
        &f.trader_tokens,
        &f.outsider.pubkey(),
        &f.trader.pubkey(),
        &[],
        held,
    )
    .unwrap();
    send(&mut f.ctx, &[approve], &[&f.trader]).await.unwrap();
    let keys = [
        f.curve,
        f.vault,
        f.mint.pubkey(),
        f.trader_tokens,
        f.reward_tokens,
    ];
    let before = account_bytes(&mut f, &keys).await;
    let mut forged = f.redeem_ix(held, 1);
    forged.accounts[5].pubkey = f.outsider.pubkey();
    assert!(send(&mut f.ctx, &[forged], &[&f.outsider]).await.is_err());
    assert_eq!(account_bytes(&mut f, &keys).await, before);
    // This amount fits outstanding total supply, but fails inside SPL burn.
    let overdraw = f.redeem_ix(held + 1, 1);
    assert!(send(&mut f.ctx, &[overdraw], &[&f.trader]).await.is_err());
    assert_eq!(account_bytes(&mut f, &keys).await, before);
    let valid = f.redeem_ix(held, 1);
    send(&mut f.ctx, &[valid], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, 0);
}

#[tokio::test]
async fn claim_and_split_redemption_in_one_packet_conserve_supply_and_backing() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    f.end_stage().await;
    f.next_stage().await;
    config(&mut f, 2 * SOL);
    let activate = f.activate_redemption_ix();
    send(&mut f.ctx, &[activate], &[&f.trader]).await.unwrap();
    let reward = f.curve().await.stage_reward_pools[0];
    let total = tokens(&mut f.ctx, f.trader_tokens).await + reward;
    let claim = f.claim_ix(1, reward, vec![], None, None);
    let keys = [
        f.curve,
        f.vault,
        f.mint.pubkey(),
        f.trader_tokens,
        f.reward_tokens,
        claim.accounts[2].pubkey,
        claim.accounts[3].pubkey,
    ];
    let before = account_bytes(&mut f, &keys).await;
    let bad = f.redeem_ix(total, u64::MAX);
    assert!(send(&mut f.ctx, &[claim.clone(), bad], &[&f.trader])
        .await
        .is_err());
    assert_eq!(account_bytes(&mut f, &keys).await, before);
    let first = f.redeem_ix(total / 3, 1);
    let last = f.redeem_ix(total - total / 3, 1);
    let hash = f.ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[
            ComputeBudgetInstruction::set_compute_unit_limit(400_000),
            claim,
            first,
            last,
        ],
        Some(&f.trader.pubkey()),
        &[&f.trader],
        hash,
    );
    assert!(bincode::serialize(&tx).unwrap().len() <= 1232);
    f.ctx.banks_client.process_transaction(tx).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, 0);
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
    assert_eq!(
        lamports(&mut f.ctx, f.vault).await,
        Rent::default().minimum_balance(0)
    );
    let mint = Mint::unpack(
        &f.ctx
            .banks_client
            .get_account(f.mint.pubkey())
            .await
            .unwrap()
            .unwrap()
            .data,
    )
    .unwrap();
    assert_eq!(mint.supply, tokens(&mut f.ctx, f.curve_tokens).await);
    assert_eq!(f.curve().await.real_sol_reserves, 0);
}

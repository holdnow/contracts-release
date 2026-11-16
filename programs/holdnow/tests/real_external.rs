use anchor_lang::solana_program::system_program;
// Opt-in tests against exact public deployed Raydium/OpenBook ELF snapshots.
// Uses an isolated bank, new market/mint and synthetic balances; no RPC writes.
mod common;
use common::*;
use solana_sdk::{
    account::{Account, AccountSharedData},
    instruction::{AccountMeta, Instruction},
    program_option::COption,
    program_pack::Pack,
    pubkey::Pubkey,
    rent::Rent,
    signature::Signer,
    sysvar,
};
use spl_token::state::{Account as TokenAccount, AccountState, Mint};
async fn put(f: &mut Fixture, key: Pubkey, owner: Pubkey, data: Vec<u8>) {
    let account = Account {
        lamports: Rent::default().minimum_balance(data.len()),
        data,
        owner,
        executable: false,
        rent_epoch: 0,
    };
    put_balanced(f, key, account).await;
}
async fn token(f: &mut Fixture, key: Pubkey, mint: Pubkey, owner: Pubkey, native: bool) {
    let mut data = vec![0; TokenAccount::LEN];
    TokenAccount::pack(
        TokenAccount {
            mint,
            owner,
            amount: 0,
            delegate: COption::None,
            state: AccountState::Initialized,
            is_native: if native {
                COption::Some(Rent::default().minimum_balance(TokenAccount::LEN))
            } else {
                COption::None
            },
            delegated_amount: 0,
            close_authority: COption::None,
        },
        &mut data,
    )
    .unwrap();
    put(f, key, spl_token::id(), data).await;
}
async fn setup() -> (Fixture, Instruction, Pubkey, Pubkey, Pubkey, Pubkey) {
    setup_with_final_burn(false).await
}
async fn setup_with_final_burn(
    burn_final: bool,
) -> (Fixture, Instruction, Pubkey, Pubkey, Pubkey, Pubkey) {
    setup_launch(burn_final, 10 * SOL, 1).await
}

async fn setup_launch(
    burn_final: bool,
    buy_lamports: u64,
    virtual_factor: u64,
) -> (Fixture, Instruction, Pubkey, Pubkey, Pubkey, Pubkey) {
    setup_launch_with_rent(burn_final, buy_lamports, virtual_factor, false).await
}

async fn setup_launch_with_rent(
    burn_final: bool,
    buy_lamports: u64,
    virtual_factor: u64,
    current_cluster_rent: bool,
) -> (Fixture, Instruction, Pubkey, Pubkey, Pubkey, Pubkey) {
    setup_launch_with_shortfall(
        burn_final,
        buy_lamports,
        virtual_factor,
        current_cluster_rent,
        None,
        contract::MIN_MIGRATION_SOL,
    )
    .await
}

async fn setup_launch_with_shortfall(
    burn_final: bool,
    buy_lamports: u64,
    virtual_factor: u64,
    current_cluster_rent: bool,
    migration_shortfall: Option<u64>,
    minimum_migration_sol: u64,
) -> (Fixture, Instruction, Pubkey, Pubkey, Pubkey, Pubkey) {
    assert!(
        std::env::var_os("HOLDNOW_EXTERNAL_PROGRAMS").is_some(),
        "Run scripts/test-external.sh; external binaries are required"
    );
    let mut f = Fixture::initialized(Config {
        current_cluster_rent,
        minimum_migration_sol,
        stages: if burn_final { 2 } else { 1 },
        airdrop: if burn_final { 900_000_000 } else { 0 },
        ..Config::default()
    })
    .await;
    if burn_final {
        f.config.root = Pubkey::new_from_array(initial_leaf(
            f.trader.pubkey(),
            f.mint.pubkey(),
            f.config.airdrop,
        ));
    }
    if virtual_factor != 1 {
        let params = f.params_ix(
            f.admin.pubkey(),
            0,
            1,
            SUPPLY * virtual_factor,
            30 * SOL,
            SUPPLY,
        );
        send(&mut f.ctx, &[params], &[&f.admin]).await.unwrap();
    }
    let create = f.create_ix();
    send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
    f.replay_legacy_minimum().await;
    let actual_buy = if let Some(shortfall) = migration_shortfall {
        assert!(!burn_final && virtual_factor == 1);
        let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
        let cluster = if cfg!(feature = "devnet") {
            "devnet"
        } else {
            "mainnet-beta"
        };
        let dir = std::path::PathBuf::from(std::env::var_os("HOLDNOW_EXTERNAL_PROGRAMS").unwrap());
        let config = std::fs::read(dir.join(format!("{cluster}-config.bin"))).unwrap();
        let fee = u64::from_le_bytes(config[536..544].try_into().unwrap());
        let reserved = contract::MIGRATION_ACCOUNT_SIZES
            .iter()
            .map(|&size| rent.minimum_balance(size))
            .sum::<u64>()
            + fee
            + rent.minimum_balance(0);
        let desired_net = reserved - shortfall - lamports(&mut f.ctx, f.vault).await;
        // The fixture's one-percent fee rounds down. Find an exact real buyer
        // input; no state or vault balance injection funds this launch.
        let mut gross = desired_net * 100 / 99;
        while gross - gross / 100 < desired_net {
            gross += 1;
        }
        assert_eq!(gross - gross / 100, desired_net);
        gross
    } else {
        buy_lamports
    };
    f.buy(actual_buy).await;
    f.end_stage().await;
    if burn_final {
        use anchor_lang::InstructionData;
        f.next_stage().await;
        f.buy(SOL).await;
        f.end_stage().await;
        let mut settle = f.root_ix(f.admin.pubkey(), 2, Pubkey::default());
        settle.data = contract::instruction::UpdateMerkleRoot {
            stage: 2,
            merkle_root: Pubkey::default(),
            total_eligible_supply: 0,
        }
        .data();
        send(&mut f.ctx, &[settle], &[&f.admin]).await.unwrap();
    }
    f.next_stage().await;
    let native = spl_token::native_mint::id();
    let mut data = vec![0; Mint::LEN];
    Mint::pack(
        Mint {
            mint_authority: COption::None,
            supply: 0,
            decimals: 9,
            is_initialized: true,
            freeze_authority: COption::None,
        },
        &mut data,
    )
    .unwrap();
    put(&mut f, native, spl_token::id(), data).await;
    let market = Pubkey::new_unique();
    let req = Pubkey::new_unique();
    let events = Pubkey::new_unique();
    let bids = Pubkey::new_unique();
    let asks = Pubkey::new_unique();
    for (key, size) in [
        (market, 388),
        (req, 5120 + 12),
        (events, 262144 + 12),
        (bids, 65536 + 12),
        (asks, 65536 + 12),
    ] {
        put(&mut f, key, contract::MARKET_PROGRAM, vec![0; size]).await;
    }
    let (signer, nonce) = (0u64..)
        .find_map(|n| {
            Pubkey::create_program_address(
                &[market.as_ref(), &n.to_le_bytes()],
                &contract::MARKET_PROGRAM,
            )
            .ok()
            .map(|p| (p, n))
        })
        .unwrap();
    let coin_market = Pubkey::new_unique();
    let pc_market = Pubkey::new_unique();
    let mint = f.mint.pubkey();
    token(&mut f, coin_market, mint, signer, false).await;
    token(&mut f, pc_market, native, signer, true).await;
    let mut data = vec![0; 5];
    data.extend_from_slice(&100u64.to_le_bytes());
    data.extend_from_slice(&100u64.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&nonce.to_le_bytes());
    data.extend_from_slice(&100u64.to_le_bytes());
    let mut accounts: Vec<_> = [market, req, events, bids, asks, coin_market, pc_market]
        .into_iter()
        .map(|key| AccountMeta::new(key, false))
        .collect();
    accounts.extend(
        [mint, native, sysvar::rent::id()]
            .into_iter()
            .map(|key| AccountMeta::new_readonly(key, false)),
    );
    send(
        &mut f.ctx,
        &[Instruction {
            program_id: contract::MARKET_PROGRAM,
            accounts,
            data,
        }],
        &[],
    )
    .await
    .expect("real OpenBook market initialization");
    let market_data = f
        .ctx
        .banks_client
        .get_account(market)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&market_data.data[..5], b"serum");
    assert_eq!(
        u64::from_le_bytes(market_data.data[5..13].try_into().unwrap()) & 3,
        3
    );
    let pda = |seed: &[u8]| {
        Pubkey::find_program_address(
            &[contract::AMM_PROGRAM.as_ref(), market.as_ref(), seed],
            &contract::AMM_PROGRAM,
        )
        .0
    };
    let amm = pda(b"amm_associated_seed");
    let orders = pda(b"open_order_associated_seed");
    let lp = pda(b"lp_mint_associated_seed");
    let coin = pda(b"coin_vault_associated_seed");
    let pc = pda(b"pc_vault_associated_seed");
    let target = pda(b"target_associated_seed");
    let (authority, bump) =
        Pubkey::find_program_address(&[b"amm authority"], &contract::AMM_PROGRAM);
    let config =
        Pubkey::find_program_address(&[b"amm_config_account_seed"], &contract::AMM_PROGRAM).0;
    let cluster = if cfg!(feature = "devnet") {
        "devnet"
    } else {
        "mainnet-beta"
    };
    let dir = std::path::PathBuf::from(std::env::var_os("HOLDNOW_EXTERNAL_PROGRAMS").unwrap());
    put(
        &mut f,
        config,
        contract::AMM_PROGRAM,
        std::fs::read(dir.join(format!("{cluster}-config.bin"))).unwrap(),
    )
    .await;
    put(
        &mut f,
        contract::AMM_FEE_DESTINATION,
        spl_token::id(),
        std::fs::read(dir.join(format!("{cluster}-fee.bin"))).unwrap(),
    )
    .await;
    // Preserve valid native-token backing in the snapshotted fee account.
    let mut fee = f
        .ctx
        .banks_client
        .get_account(contract::AMM_FEE_DESTINATION)
        .await
        .unwrap()
        .unwrap();
    let state = TokenAccount::unpack(&fee.data).unwrap();
    fee.lamports = state.amount + state.is_native.unwrap();
    put_balanced(&mut f, contract::AMM_FEE_DESTINATION, fee).await;
    let user_pc =
        spl_associated_token_account::address::get_associated_token_address(&f.vault, &native);
    let vault = f.vault;
    token(&mut f, user_pc, native, vault, true).await;
    let user_lp =
        spl_associated_token_account::address::get_associated_token_address(&f.vault, &lp);
    let i = ix(
        contract::accounts::MoveRaydium {
            global: f.global,
            fee_recipient: f.fee,
            raydium_fee_destination: contract::AMM_FEE_DESTINATION,
            mint,
            vault: f.vault,
            bonding_curve: f.curve,
            amm_program: contract::AMM_PROGRAM,
            token_program: spl_token::id(),
            associated_token_program: spl_associated_token_account::program::id(),
            system_program: system_program::id(),
            sysvar_rent: sysvar::rent::id(),
            amm,
            amm_authority: authority,
            amm_open_orders: orders,
            lp_mint: lp,
            pc_mint: native,
            coin_vault: coin,
            pc_vault: pc,
            target_orders: target,
            amm_config: config,
            market_program: contract::MARKET_PROGRAM,
            market,
            user_token_coin: f.curve_tokens,
            user_token_pc: user_pc,
            user_token_lp: user_lp,
            user: f.admin.pubkey(),
            clock: sysvar::clock::id(),
        },
        contract::instruction::MoveRaydium {
            nonce: bump,
            fee_bps: 100,
        },
    );
    (f, i, coin, pc, user_lp, lp)
}
struct MigrationSnapshot {
    inventory: u64,
    supply: u64,
    held: u64,
    rewards: u64,
    virtual_tokens: u64,
    virtual_sol: u64,
}
async fn snapshot(f: &mut Fixture) -> MigrationSnapshot {
    let curve = f.curve().await;
    let mint = f
        .ctx
        .banks_client
        .get_account(f.mint.pubkey())
        .await
        .unwrap()
        .unwrap();
    MigrationSnapshot {
        inventory: tokens(&mut f.ctx, f.curve_tokens).await,
        supply: Mint::unpack(&mint.data).unwrap().supply,
        held: tokens(&mut f.ctx, f.trader_tokens).await,
        rewards: tokens(&mut f.ctx, f.reward_tokens).await,
        virtual_tokens: curve.virtual_token_reserves,
        virtual_sol: curve.virtual_sol_reserves,
    }
}
async fn assert_price_and_supply(
    f: &mut Fixture,
    before: &MigrationSnapshot,
    coin: Pubkey,
    pc: Pubkey,
) {
    let sol = tokens(&mut f.ctx, pc).await as u128;
    let actual = tokens(&mut f.ctx, coin).await as u128;
    let expected = (sol * before.virtual_tokens as u128 / before.virtual_sol as u128)
        .min(before.inventory as u128);
    assert_eq!(
        actual, expected,
        "pool deposit must match final spot price to one base unit"
    );
    assert!(sol * before.virtual_tokens as u128 >= actual * before.virtual_sol as u128);
    let mint = f
        .ctx
        .banks_client
        .get_account(f.mint.pubkey())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Mint::unpack(&mint.data).unwrap().supply,
        before.supply - (before.inventory - actual as u64)
    );
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, 0);
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, before.held);
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, before.rewards);
}
#[tokio::test]
#[ignore = "requires downloaded deployed programs; scripts/test-external.sh"]
async fn real_openbook_market_and_raydium_migration_burn_lp() {
    let (mut f, i, coin, pc, user_lp, lp) = setup().await;
    let before = snapshot(&mut f).await;
    let fee_before = lamports(&mut f.ctx, contract::AMM_FEE_DESTINATION).await;
    send(&mut f.ctx, &[i.clone()], &[&f.admin])
        .await
        .expect("real Raydium initialize2 and atomic LP burn");
    assert_price_and_supply(&mut f, &before, coin, pc).await;
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, 0);
    assert!(tokens(&mut f.ctx, pc).await > 9 * SOL);
    assert_eq!(tokens(&mut f.ctx, user_lp).await, 0);
    let mint = f.ctx.banks_client.get_account(lp).await.unwrap().unwrap();
    assert_eq!(Mint::unpack(&mint.data).unwrap().supply, 0);
    assert!(f.curve().await.migrated);
    let config = f
        .ctx
        .banks_client
        .get_account(i.accounts[19].pubkey)
        .await
        .unwrap()
        .unwrap();
    let creation_fee = u64::from_le_bytes(config.data[536..544].try_into().unwrap());
    assert_eq!(
        lamports(&mut f.ctx, contract::AMM_FEE_DESTINATION).await - fee_before,
        creation_fee
    );
    assert_pool_can_swap(&mut f, &i, coin, pc).await;

    assert!(send(&mut f.ctx, &[i], &[&f.admin]).await.is_err());
}
async fn assert_pool_can_swap(f: &mut Fixture, i: &Instruction, coin: Pubkey, pc: Pubkey) {
    // Exercise actual pool usability after all issued LP is burned.
    let market = i.accounts[21].pubkey;
    let market_data = f
        .ctx
        .banks_client
        .get_account(market)
        .await
        .unwrap()
        .unwrap()
        .data;
    let key_at =
        |offset| Pubkey::new_from_array(market_data[offset..offset + 32].try_into().unwrap());
    let nonce = u64::from_le_bytes(market_data[45..53].try_into().unwrap());
    let signer = Pubkey::create_program_address(
        &[market.as_ref(), &nonce.to_le_bytes()],
        &contract::MARKET_PROGRAM,
    )
    .unwrap();
    let trader_pc = Pubkey::new_unique();
    let trader = f.trader.pubkey();
    token(f, trader_pc, spl_token::native_mint::id(), trader, true).await;
    let mut accounts = vec![
        AccountMeta::new_readonly(spl_token::id(), false),
        AccountMeta::new(i.accounts[11].pubkey, false),
        AccountMeta::new_readonly(i.accounts[12].pubkey, false),
        AccountMeta::new(i.accounts[13].pubkey, false),
        AccountMeta::new(coin, false),
        AccountMeta::new(pc, false),
        AccountMeta::new_readonly(contract::MARKET_PROGRAM, false),
        AccountMeta::new(market, false),
    ];
    accounts.extend(
        [
            key_at(285),
            key_at(317),
            key_at(253),
            key_at(117),
            key_at(165),
        ]
        .into_iter()
        .map(|k| AccountMeta::new(k, false)),
    );
    accounts.extend([
        AccountMeta::new_readonly(signer, false),
        AccountMeta::new(f.trader_tokens, false),
        AccountMeta::new(trader_pc, false),
        AccountMeta::new_readonly(trader, true),
    ]);
    let input = 1_000_000_000u64;
    let mut data = vec![9];
    data.extend_from_slice(&input.to_le_bytes());
    data.extend_from_slice(&1u64.to_le_bytes());
    let before = tokens(&mut f.ctx, f.trader_tokens).await;
    send(
        &mut f.ctx,
        &[Instruction {
            program_id: contract::AMM_PROGRAM,
            accounts,
            data,
        }],
        &[&f.trader],
    )
    .await
    .expect("real swap after migration");
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, before - input);
    assert!(tokens(&mut f.ctx, trader_pc).await > 0);
}

#[tokio::test]
#[ignore = "requires downloaded deployed programs; scripts/test-external.sh"]
async fn real_raydium_rejection_rolls_back_holdnow_transfers() {
    let (mut f, mut i, _, _, _, _) = setup().await;
    let before = tokens(&mut f.ctx, f.curve_tokens).await;
    let sol = lamports(&mut f.ctx, f.vault).await;
    i.data[8] = i.data[8].wrapping_sub(1); // Invalid AMM authority nonce, rejected inside real Raydium.
    assert!(send(&mut f.ctx, &[i], &[&f.admin]).await.is_err());
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, before);
    assert_eq!(lamports(&mut f.ctx, f.vault).await, sol);
    assert!(!f.curve().await.migrated);
}

#[tokio::test]
#[ignore = "requires downloaded deployed programs; scripts/test-external.sh"]
async fn real_raydium_accepts_prefunded_pda_dust() {
    let (mut f, instruction, coin, pc, lp, _) = setup().await;
    let target = instruction.accounts[18].pubkey;
    // Use an actually admissible transaction: a newly funded system account
    // must meet the runtime's minimum rent balance at the transaction boundary.
    let prefund = solana_system_interface::instruction::transfer(
        &f.outsider.pubkey(),
        &target,
        Rent::default().minimum_balance(0),
    );
    send(&mut f.ctx, &[prefund], &[&f.outsider]).await.unwrap();
    let before = snapshot(&mut f).await;
    send(&mut f.ctx, &[instruction], &[&f.admin])
        .await
        .expect("unsolicited PDA lamport cannot invalidate rent budgeting");
    assert_price_and_supply(&mut f, &before, coin, pc).await;
    assert_eq!(tokens(&mut f.ctx, lp).await, 0);
    assert!(f.curve().await.migrated);
}

/// Regression F06: an occupied legacy identity cannot trap migration or receive Holdnow liquidity.
#[tokio::test]
#[ignore = "requires downloaded deployed programs; scripts/test-external.sh"]
async fn fixed_f06_real_pool_collision_recovers_with_fresh_atomic_backend_plan() {
    let (mut f, migration, _, _, _, lp) = setup().await;
    let attacker = f.trader.pubkey();
    let native = spl_token::native_mint::id();
    let attacker_pc = Pubkey::new_unique();
    token(&mut f, attacker_pc, native, attacker, true).await;
    let attacker_lp =
        spl_associated_token_account::address::get_associated_token_address(&attacker, &lp);
    let fund = solana_system_interface::instruction::transfer(&attacker, &attacker_pc, SOL);
    let sync = spl_token::instruction::sync_native(&spl_token::id(), &attacker_pc).unwrap();
    send(&mut f.ctx, &[fund, sync], &[&f.trader]).await.unwrap();
    let m = &migration.accounts;
    let mut accounts: Vec<_> = [
        7usize, 8, 9, 10, 11, 12, 13, 14, 3, 15, 16, 17, 18, 19, 2, 20, 21,
    ]
    .into_iter()
    .map(|index| m[index].clone())
    .collect();
    accounts.extend([
        AccountMeta::new(attacker, true),
        AccountMeta::new(f.trader_tokens, false),
        AccountMeta::new(attacker_pc, false),
        AccountMeta::new(attacker_lp, false),
    ]);
    let mut data = vec![1, migration.data[8]]; // initialize2, canonical authority bump
    data.extend_from_slice(&0u64.to_le_bytes());
    data.extend_from_slice(&SOL.to_le_bytes());
    data.extend_from_slice(&1_000_000_000u64.to_le_bytes());
    send(
        &mut f.ctx,
        &[Instruction {
            program_id: contract::AMM_PROGRAM,
            accounts,
            data,
        }],
        &[&f.trader],
    )
    .await
    .expect("a token holder can initialize this public market's AMM first");
    assert!(tokens(&mut f.ctx, attacker_lp).await > 0);
    let before = tokens(&mut f.ctx, f.curve_tokens).await;
    let sol_before = lamports(&mut f.ctx, f.vault).await;
    assert!(send(&mut f.ctx, &[migration], &[&f.admin]).await.is_err());
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, before);
    assert_eq!(lamports(&mut f.ctx, f.vault).await, sol_before);
    assert!(!f.curve().await.migrated);
    let attacker_lp_before = tokens(&mut f.ctx, attacker_lp).await;
    atomic_backend_migration(&mut f).await;
    assert_eq!(tokens(&mut f.ctx, attacker_lp).await, attacker_lp_before);
}

fn decode_planned_instruction(value: &serde_json::Value) -> Instruction {
    Instruction {
        program_id: value["programId"].as_str().unwrap().parse().unwrap(),
        accounts: value["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| AccountMeta {
                pubkey: k["pubkey"].as_str().unwrap().parse().unwrap(),
                is_signer: k["isSigner"].as_bool().unwrap(),
                is_writable: k["isWritable"].as_bool().unwrap(),
            })
            .collect(),
        data: value["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_u64().unwrap() as u8)
            .collect(),
    }
}

/// Execute the actual backend SDK planner's output, including real lookup-table
/// creation and a packet-size-checked signed v0 transaction, against pinned ELFs.
async fn atomic_backend_migration(f: &mut Fixture) -> Pubkey {
    use solana_sdk::{
        message::AddressLookupTableAccount,
        message::{v0, VersionedMessage},
        signature::Keypair,
        slot_hashes::SlotHashes,
        transaction::{Transaction, VersionedTransaction},
    };
    f.ctx.warp_to_slot(100).unwrap();
    let hashes_account = f
        .ctx
        .banks_client
        .get_account(sysvar::slot_hashes::id())
        .await
        .unwrap()
        .unwrap();
    let hashes: SlotHashes = bincode::deserialize(&hashes_account.data).unwrap();
    let recent_slot = hashes.first().unwrap().0;
    let authority = Keypair::new();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let node = std::env::var_os("HOLDNOW_TEST_NODE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join(".audit-tools/node/bin/node"));
    let input = serde_json::json!({
        "slot": recent_slot, "payer": f.admin.pubkey().to_string(), "programId": contract::ID.to_string(),
        "token": f.mint.pubkey().to_string(), "feeRecipient": f.fee.to_string(),
        "lookupAuthority": authority.pubkey().to_string(), "ammProgram": contract::AMM_PROGRAM.to_string(),
        "marketProgram": contract::MARKET_PROGRAM.to_string(), "raydiumFeeDestination": contract::AMM_FEE_DESTINATION.to_string(),
    });
    let output = std::process::Command::new(node)
        .arg(root.join("backend/node_modules/tsx/dist/cli.mjs"))
        .arg(root.join("backend/tests/fixtures/buildMigrationPlan.ts"))
        .arg(input.to_string())
        .output()
        .expect("Node 22 and backend test dependencies are required");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let market: Pubkey = plan["market"].as_str().unwrap().parse().unwrap();
    assert!(f
        .ctx
        .banks_client
        .get_account(market)
        .await
        .unwrap()
        .is_none());
    for group in plan["preparation"].as_array().unwrap() {
        let instructions: Vec<_> = group
            .as_array()
            .unwrap()
            .iter()
            .map(decode_planned_instruction)
            .collect();
        assert!(instructions
            .iter()
            .all(|i| i.accounts.iter().all(|a| a.pubkey != market)));
        let authority_needed = instructions.iter().any(|i| {
            i.accounts
                .iter()
                .any(|a| a.pubkey == authority.pubkey() && a.is_signer)
        });
        let mut signers = vec![&f.admin];
        if authority_needed {
            signers.push(&authority);
        }
        let tx = Transaction::new_signed_with_payer(
            &instructions,
            Some(&f.admin.pubkey()),
            &signers,
            f.ctx.last_blockhash,
        );
        assert!(bincode::serialize(&tx).unwrap().len() <= 1232);
        f.ctx
            .banks_client
            .process_transaction(tx)
            .await
            .expect("real preparation transaction");
        assert!(f
            .ctx
            .banks_client
            .get_account(market)
            .await
            .unwrap()
            .is_none());
    }
    f.ctx.warp_to_slot(101).unwrap();
    let table_key: Pubkey = plan["table"].as_str().unwrap().parse().unwrap();
    let table_account = f
        .ctx
        .banks_client
        .get_account(table_key)
        .await
        .unwrap()
        .unwrap();
    let actual = solana_address_lookup_table_program::state::AddressLookupTable::deserialize(
        &table_account.data,
    )
    .unwrap();
    let table = AddressLookupTableAccount {
        key: table_key,
        addresses: actual.addresses.to_vec(),
    };
    let instructions: Vec<_> = plan["final"]
        .as_array()
        .unwrap()
        .iter()
        .map(decode_planned_instruction)
        .collect();
    let message = v0::Message::try_compile(
        &f.admin.pubkey(),
        &instructions,
        &[table],
        f.ctx.last_blockhash,
    )
    .unwrap();
    let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[&f.admin]).unwrap();
    let size = bincode::serialize(&tx).unwrap().len();
    assert_eq!(size as u64, plan["packetBytes"].as_u64().unwrap());
    assert!(size <= 1232);
    f.ctx
        .banks_client
        .process_transaction(tx)
        .await
        .expect("atomic real OpenBook initialization and Holdnow/Raydium migration");
    assert!(f.curve().await.migrated);
    let account = f
        .ctx
        .banks_client
        .get_account(market)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&account.data[..5], b"serum");
    let lp = Pubkey::find_program_address(
        &[
            contract::AMM_PROGRAM.as_ref(),
            market.as_ref(),
            b"lp_mint_associated_seed",
        ],
        &contract::AMM_PROGRAM,
    )
    .0;
    let received_lp =
        spl_associated_token_account::address::get_associated_token_address(&f.vault, &lp);
    assert_eq!(tokens(&mut f.ctx, received_lp).await, 0);
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, 0);
    market
}

#[tokio::test]
#[ignore = "requires downloaded deployed programs and backend dependencies; scripts/test-external.sh"]
async fn real_backend_atomic_openbook_and_raydium_transaction_fits_and_executes() {
    let (mut f, _, _, _, _, _) = setup().await;
    atomic_backend_migration(&mut f).await;
}

// Imported snapshots are funded from the test payer so slot advancement can
// verify bank capitalization. No synthetic lamports are added after genesis.
async fn put_balanced(f: &mut Fixture, key: Pubkey, account: Account) {
    let old = f
        .ctx
        .banks_client
        .get_account(key)
        .await
        .unwrap()
        .map_or(0, |a| a.lamports);
    let payer_key = f.ctx.payer.pubkey();
    assert_ne!(key, payer_key);
    let mut payer = f
        .ctx
        .banks_client
        .get_account(payer_key)
        .await
        .unwrap()
        .unwrap();
    payer.lamports = payer
        .lamports
        .checked_add(old)
        .unwrap()
        .checked_sub(account.lamports)
        .unwrap();
    f.ctx
        .set_account(&payer_key, &AccountSharedData::from(payer));
    f.ctx.set_account(&key, &AccountSharedData::from(account));
}

#[tokio::test]
#[ignore = "requires downloaded deployed programs and backend dependencies; scripts/test-external.sh"]
async fn real_final_burn_then_atomic_migration_keeps_old_rewards_and_vesting_claimable() {
    let (mut f, _, _, _, _, _) = setup_with_final_burn(true).await;
    let c = f.curve().await;
    let initial = f.config.airdrop;
    let regular = c.stage_reward_pools[0];
    assert!(c.total_unallocated_burned > 0);
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, initial + regular);
    let before = snapshot(&mut f).await;
    let market = atomic_backend_migration(&mut f).await;
    let coin_vault = Pubkey::find_program_address(
        &[
            contract::AMM_PROGRAM.as_ref(),
            market.as_ref(),
            b"coin_vault_associated_seed",
        ],
        &contract::AMM_PROGRAM,
    )
    .0;
    let pc_vault = Pubkey::find_program_address(
        &[
            contract::AMM_PROGRAM.as_ref(),
            market.as_ref(),
            b"pc_vault_associated_seed",
        ],
        &contract::AMM_PROGRAM,
    )
    .0;
    assert_price_and_supply(&mut f, &before, coin_vault, pc_vault).await;
    let burned_inventory = before.inventory - tokens(&mut f.ctx, coin_vault).await;
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, initial + regular);
    clock(&mut f.ctx, c.at_start_stage + 10 * 365 * 86400).await;
    let claim = f.claim_ix(1, regular, vec![], None, None);
    send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
    let vested = f.claim_ix(2, 0, vec![], Some(initial), Some(vec![]));
    send(&mut f.ctx, &[vested], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
    let mint = f
        .ctx
        .banks_client
        .get_account(f.mint.pubkey())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Mint::unpack(&mint.data).unwrap().supply,
        SUPPLY - c.total_unallocated_burned - burned_inventory
    );
}

// Regression: an unviable AMM migration exposes a permissionless, permanent exit.
#[tokio::test]
#[ignore = "requires deployed programs; scripts/test-external.sh"]
async fn audit_r01_sold_out_launch_recovers_sol_through_redemption() {
    let (mut f, migrate, _, _, _, _) = setup_launch(false, 30 * SOL, 2).await;
    let c = f.curve().await;
    assert!(c.complete && !c.migrated);
    assert_eq!(c.real_token_reserves, 0);
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, 0);
    assert_eq!(c.real_sol_reserves, 30 * SOL);
    let before = lamports(&mut f.ctx, f.vault).await;
    custom_error(
        send(&mut f.ctx, &[migrate.clone()], &[&f.admin]).await,
        6000 + contract::error::HoldNowCode::InsufficientFunds as u32,
    );
    assert_eq!(lamports(&mut f.ctx, f.vault).await, before);
    let sale = f.sell_ix(f.trader.pubkey(), f.trader_tokens, 1_000_000, 0);
    custom_error(
        send(&mut f.ctx, &[sale], &[&f.trader]).await,
        6000 + contract::error::HoldNowCode::BondingCurveComplete as u32,
    );
    assert!(!f.curve().await.migrated);
    let activate = f.activate_redemption_ix();
    send(&mut f.ctx, &[activate], &[&f.trader]).await.unwrap();
    custom_error(
        send(&mut f.ctx, &[migrate], &[&f.admin]).await,
        6000 + contract::error::HoldNowCode::RedemptionActive as u32,
    );
    let claim_amount = f.curve().await.stage_reward_pools[0];
    let claim = f.claim_ix(1, claim_amount, vec![], None, None);
    send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
    let amount = tokens(&mut f.ctx, f.trader_tokens).await;
    let redeem = f.redeem_ix(amount, 1);
    send(&mut f.ctx, &[redeem], &[&f.trader]).await.unwrap();
    assert!(f.curve().await.redemption_mode);
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, 0);
    assert_eq!(
        lamports(&mut f.ctx, f.vault).await,
        Rent::default().minimum_balance(0)
    );
}

#[tokio::test]
#[ignore = "requires deployed programs; scripts/test-external.sh"]
async fn audit_r01_small_launch_recovers_sol_through_redemption() {
    let (mut f, migrate, _, _, _, _) = setup_launch(false, 1_000, 1).await;
    assert!(f.curve().await.complete);
    let before = lamports(&mut f.ctx, f.vault).await;
    let coin = tokens(&mut f.ctx, f.curve_tokens).await;
    custom_error(
        send(&mut f.ctx, &[migrate.clone()], &[&f.admin]).await,
        6000 + contract::error::HoldNowCode::InsufficientVaultFunds as u32,
    );
    assert_eq!(lamports(&mut f.ctx, f.vault).await, before);
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, coin);
    assert!(!f.curve().await.migrated);
    let activate = f.activate_redemption_ix();
    send(&mut f.ctx, &[activate], &[&f.trader]).await.unwrap();
    custom_error(
        send(&mut f.ctx, &[migrate], &[&f.admin]).await,
        6000 + contract::error::HoldNowCode::RedemptionActive as u32,
    );
    let claim_amount = f.curve().await.stage_reward_pools[0];
    let claim = f.claim_ix(1, claim_amount, vec![], None, None);
    send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
    let amount = tokens(&mut f.ctx, f.trader_tokens).await;
    let redeem = f.redeem_ix(amount, 1);
    send(&mut f.ctx, &[redeem], &[&f.trader]).await.unwrap();
    assert!(f.curve().await.redemption_mode);
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, 0);
    assert_eq!(
        lamports(&mut f.ctx, f.vault).await,
        Rent::default().minimum_balance(0)
    );
}

/// Fresh audit: validate security-relevant accounts against the actual AMM, not a stub.
#[tokio::test]
#[ignore = "requires deployed programs; scripts/test-external.sh"]
async fn fresh_v3_real_amm_rejects_account_substitutions_without_partial_value_movement() {
    let (mut f, migration, _, _, _, _) = setup().await;
    let tracked = [
        f.curve,
        f.vault,
        f.curve_tokens,
        f.reward_tokens,
        f.mint.pubkey(),
        f.fee,
        migration.accounts[23].pubkey,
        contract::AMM_FEE_DESTINATION,
    ];
    let mut before = Vec::new();
    for key in &tracked {
        before.push(f.ctx.banks_client.get_account(*key).await.unwrap());
    }
    // Account 13 is deliberately omitted: recent Raydium binaries no longer use OpenOrders.
    for index in [2usize, 11, 12, 14, 16, 17, 18, 19, 20, 21, 22, 23, 24] {
        let mut substituted = migration.clone();
        substituted.accounts[index].pubkey = f.fee;
        assert!(
            send(&mut f.ctx, &[substituted], &[&f.admin]).await.is_err(),
            "account index {index}"
        );
        for (position, key) in tracked.iter().enumerate() {
            assert_eq!(
                f.ctx.banks_client.get_account(*key).await.unwrap(),
                before[position],
                "account index {index}, affected {key}"
            );
        }
    }
    // The untouched transaction remains executable after every failed attempt.
    send(&mut f.ctx, &[migration], &[&f.admin]).await.unwrap();
    assert!(f.curve().await.migrated);
}

/// Regression: successful migration accounts for every available lamport.
#[tokio::test]
#[ignore = "requires deployed programs; scripts/test-external.sh"]
async fn fixed_v3_01_successful_migration_retains_only_rent() {
    let (mut f, migration, _, _, _, _) = setup().await;
    send(&mut f.ctx, &[migration.clone()], &[&f.admin])
        .await
        .unwrap();
    let remaining = lamports(&mut f.ctx, f.vault).await - Rent::default().minimum_balance(0);
    assert_eq!(remaining, 0);
    let c = f.curve().await;
    assert!(c.migrated);
    assert_eq!(c.real_sol_reserves, 0);
    let redeem = f.redeem_ix(1, 0);
    custom_error(
        send(&mut f.ctx, &[redeem], &[&f.trader]).await,
        6000 + contract::error::HoldNowCode::AlreadyMigrated as u32,
    );
    let activate = f.activate_redemption_ix();
    custom_error(
        send(&mut f.ctx, &[activate], &[&f.trader]).await,
        6000 + contract::error::HoldNowCode::AlreadyMigrated as u32,
    );
    custom_error(
        send(&mut f.ctx, &[migration], &[&f.admin]).await,
        6000 + contract::error::HoldNowCode::AlreadyMigrated as u32,
    );
    println!("MIGRATION_RESIDUAL locked_lamports={remaining}");
}

/// Exercise the current public rent through an actual epoch feature activation.
#[tokio::test]
#[ignore = "requires deployed programs; scripts/test-external.sh"]
async fn fixed_v3_01_current_cluster_rent_retains_no_unused_budget() {
    let (mut f, migration, _, _, _, _) = setup_launch_with_rent(false, 10 * SOL, 1, true).await;
    let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
    // Prove the runtime's rent check agrees with its sysvar: this transfer would
    // fail under the default bank's higher 890,880-lamport threshold.
    let probe = Pubkey::new_unique();
    let transfer = solana_system_interface::instruction::transfer(
        &f.creator.pubkey(),
        &probe,
        rent.minimum_balance(0),
    );
    send(&mut f.ctx, &[transfer], &[&f.creator]).await.unwrap();
    send(&mut f.ctx, &[migration], &[&f.admin]).await.unwrap();
    let remaining = lamports(&mut f.ctx, f.vault).await - rent.minimum_balance(0);
    // The budget uses current Rent; excess legacy prefunding enters liquidity.
    assert_eq!(remaining, 0);
    assert_eq!(f.curve().await.real_sol_reserves, 0);
    println!("CURRENT_RENT_MIGRATION locked_lamports={remaining}");
}

/// All pool-creation savings go to the authenticated WSOL pool vault, never to
/// an increased protocol fee or a stranded Holdnow balance. Also executes a swap.
#[tokio::test]
#[ignore = "requires deployed programs; scripts/test-external.sh"]
async fn fixed_v3_01_prefunded_accounts_conserve_sol_and_pool_remains_usable() {
    for current_rent in [false, true] {
        for prefund_all in [false, true] {
            let (mut f, migration, coin, pc, user_lp, _) =
                setup_launch_with_rent(false, 10 * SOL, 1, current_rent).await;
            let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
            let config = f
                .ctx
                .banks_client
                .get_account(migration.accounts[19].pubkey)
                .await
                .unwrap()
                .unwrap();
            let creation_fee = u64::from_le_bytes(config.data[536..544].try_into().unwrap());
            let accounts = [
                (14, 82),
                (16, 165),
                (17, 165),
                (24, 165),
                (11, 752),
                (18, 2208),
            ];
            let fresh_rents: u64 = accounts
                .iter()
                .map(|(_, size)| rent.minimum_balance(*size))
                .sum();
            let mut preparation = vec![];
            let mut missing_rent = 0;
            for (index, size) in accounts {
                let amount = if prefund_all {
                    rent.minimum_balance(size)
                } else if index == 18 {
                    rent.minimum_balance(0)
                } else {
                    0
                };
                if amount > 0 {
                    preparation.push(solana_system_interface::instruction::transfer(
                        &f.outsider.pubkey(),
                        &migration.accounts[index].pubkey,
                        amount,
                    ));
                }
                missing_rent += rent.minimum_balance(size) - amount;
            }
            send(&mut f.ctx, &preparation, &[&f.outsider])
                .await
                .unwrap();
            let before = snapshot(&mut f).await;
            let vault_before = lamports(&mut f.ctx, f.vault).await;
            let treasury_before = lamports(&mut f.ctx, f.fee).await;
            let expected_fee =
                (vault_before - rent.minimum_balance(0) - creation_fee - fresh_rents) / 100;
            // Current SPL SyncNative releases legacy excess native rent into the
            // wrapped balance as well; conserve that pre-existing funding too.
            let wrapper = migration.accounts[23].pubkey;
            let wrapper_before = lamports(&mut f.ctx, wrapper).await;
            let wrapper_rent = rent.minimum_balance(TokenAccount::LEN);
            let expected_pool =
                vault_before - rent.minimum_balance(0) - expected_fee - creation_fee - missing_rent
                    + (wrapper_before - wrapper_rent);
            send(&mut f.ctx, &[migration.clone()], &[&f.admin])
                .await
                .unwrap();
            assert_eq!(lamports(&mut f.ctx, f.vault).await, rent.minimum_balance(0));
            assert_eq!(
                lamports(&mut f.ctx, f.fee).await - treasury_before,
                expected_fee
            );
            assert_eq!(lamports(&mut f.ctx, wrapper).await, wrapper_rent);
            assert_eq!(tokens(&mut f.ctx, wrapper).await, 0);
            assert_eq!(tokens(&mut f.ctx, pc).await, expected_pool);
            assert_eq!(tokens(&mut f.ctx, user_lp).await, 0);
            assert_eq!(f.curve().await.real_sol_reserves, 0);
            assert_price_and_supply(&mut f, &before, coin, pc).await;
            assert_pool_can_swap(&mut f, &migration, coin, pc).await;
        }
    }
}

#[tokio::test]
#[ignore = "requires deployed programs; scripts/test-external.sh"]
async fn regression_pool_as_configured_treasury_synchronizes_fee_and_surplus() {
    let (mut f, mut migration, coin, pc, _, _) = setup().await;
    f.fee = pc;
    let params = f.params_ix(f.admin.pubkey(), 1, 1, SUPPLY, 30 * SOL, SUPPLY);
    send(&mut f.ctx, &[params], &[&f.admin]).await.unwrap();
    migration.accounts[1].pubkey = pc;
    let before = snapshot(&mut f).await;
    send(&mut f.ctx, &[migration.clone()], &[&f.admin])
        .await
        .unwrap();
    let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
    assert_eq!(
        tokens(&mut f.ctx, pc).await,
        lamports(&mut f.ctx, pc).await - rent.minimum_balance(TokenAccount::LEN)
    );
    assert_eq!(lamports(&mut f.ctx, f.vault).await, rent.minimum_balance(0));
    assert_price_and_supply(&mut f, &before, coin, pc).await;
    assert_pool_can_swap(&mut f, &migration, coin, pc).await;
}

/// Fresh WSOL uses current runtime rent, so no legacy rent refund hides the tested boundary.
async fn fresh_wrapper(f: &mut Fixture, migration: &mut Instruction) {
    let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
    let wrapper = solana_sdk::signature::Keypair::new();
    let create = solana_system_interface::instruction::create_account(
        &f.ctx.payer.pubkey(),
        &wrapper.pubkey(),
        rent.minimum_balance(TokenAccount::LEN),
        TokenAccount::LEN as u64,
        &spl_token::id(),
    );
    let initialize = spl_token::instruction::initialize_account3(
        &spl_token::id(),
        &wrapper.pubkey(),
        &spl_token::native_mint::id(),
        &f.vault,
    )
    .unwrap();
    send(&mut f.ctx, &[create, initialize], &[&wrapper])
        .await
        .unwrap();
    migration.accounts[23].pubkey = wrapper.pubkey();
}
async fn current_budget(f: &mut Fixture, migration: &Instruction) -> u64 {
    let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
    let config = f
        .ctx
        .banks_client
        .get_account(migration.accounts[19].pubkey)
        .await
        .unwrap()
        .unwrap();
    contract::MIGRATION_ACCOUNT_SIZES
        .iter()
        .map(|&size| rent.minimum_balance(size))
        .sum::<u64>()
        + u64::from_le_bytes(config.data[536..544].try_into().unwrap())
        + rent.minimum_balance(0)
}
/// Original vulnerable behavior is preserved in audit commit 97601e4.
#[tokio::test]
#[ignore = "requires deployed programs; scripts/test-external.sh"]
async fn fixed_sp01_tiny_donation_preserves_redemption_and_cannot_create_dust_pool() {
    for current_rent in [false, true] {
        let (mut f, mut migration, _, _, _, _) = setup_launch_with_shortfall(
            false,
            1,
            1,
            current_rent,
            Some(1),
            contract::MIN_MIGRATION_SOL,
        )
        .await;
        fresh_wrapper(&mut f, &mut migration).await;
        let reserved = current_budget(&mut f, &migration).await;
        assert_eq!(lamports(&mut f.ctx, f.vault).await, reserved - 1);
        let donate =
            solana_system_interface::instruction::transfer(&f.creator.pubkey(), &f.vault, 2);
        send(&mut f.ctx, &[donate], &[&f.creator]).await.unwrap();
        let before = f.ctx.banks_client.get_account(f.curve).await.unwrap();
        custom_error(
            send(&mut f.ctx, &[migration.clone()], &[&f.admin]).await,
            6000 + contract::error::HoldNowCode::InvalidLiquidity as u32,
        );
        assert_eq!(
            f.ctx.banks_client.get_account(f.curve).await.unwrap(),
            before
        );
        assert_eq!(lamports(&mut f.ctx, f.vault).await, reserved + 1);
        let activate = f.activate_redemption_ix();
        send(&mut f.ctx, &[activate], &[&f.trader]).await.unwrap();
        let pool = f.curve().await.stage_reward_pools[0];
        let claim = f.claim_ix(1, pool, vec![], None, None);
        send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
        assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
        let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
        let held = tokens(&mut f.ctx, f.trader_tokens).await;
        let redeem = f.redeem_ix(held, reserved + 1 - rent.minimum_balance(0));
        send(&mut f.ctx, &[redeem], &[&f.trader]).await.unwrap();
        assert_eq!(lamports(&mut f.ctx, f.vault).await, rent.minimum_balance(0));
        custom_error(
            send(&mut f.ctx, &[migration], &[&f.admin]).await,
            6000 + contract::error::HoldNowCode::RedemptionActive as u32,
        );
    }
}
#[tokio::test]
#[ignore = "requires deployed programs; scripts/test-external.sh"]
async fn economic_minimum_is_exact_and_real_pool_remains_tradable() {
    let mut minimums = vec![
        contract::MIN_MIGRATION_SOL,
        contract::NEW_LAUNCH_MIN_MIGRATION_SOL,
    ];
    minimums.dedup();
    for minimum in minimums {
        for current_rent in [false, true] {
            let (mut f, mut migration, coin, pc, user_lp, _) =
                setup_launch_with_shortfall(false, 1, 1, current_rent, Some(1), minimum).await;
            fresh_wrapper(&mut f, &mut migration).await;
            let reserved = current_budget(&mut f, &migration).await;
            let minimum = f.curve().await.minimum_migration_sol;
            let gross = ((minimum as u128 - 1) * 100 / 99 + 1) as u64;
            // Start just below the exact economic threshold, without altering curve state.
            let donate = solana_system_interface::instruction::transfer(
                &f.creator.pubkey(),
                &f.vault,
                gross,
            );
            send(&mut f.ctx, &[donate], &[&f.creator]).await.unwrap();
            assert_eq!(lamports(&mut f.ctx, f.vault).await, reserved + gross - 1);
            custom_error(
                send(&mut f.ctx, &[migration.clone()], &[&f.admin]).await,
                6000 + contract::error::HoldNowCode::InvalidLiquidity as u32,
            );
            assert_eq!(lamports(&mut f.ctx, f.vault).await, reserved + gross - 1);
            let last =
                solana_system_interface::instruction::transfer(&f.creator.pubkey(), &f.vault, 1);
            send(&mut f.ctx, &[last], &[&f.creator]).await.unwrap();
            let activate = f.activate_redemption_ix();
            custom_error(
                send(&mut f.ctx, &[activate], &[&f.trader]).await,
                6000 + contract::error::HoldNowCode::MigrationFunded as u32,
            );
            let fee_before = lamports(&mut f.ctx, f.fee).await;
            send(&mut f.ctx, &[migration.clone()], &[&f.admin])
                .await
                .unwrap();
            assert_eq!(tokens(&mut f.ctx, pc).await, minimum);
            assert_eq!(tokens(&mut f.ctx, user_lp).await, 0);
            assert_eq!(
                lamports(&mut f.ctx, f.fee).await - fee_before,
                gross - minimum
            );
            let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
            assert_eq!(lamports(&mut f.ctx, f.vault).await, rent.minimum_balance(0));
            assert_pool_can_swap(&mut f, &migration, coin, pc).await;
        }
    }
}

#[tokio::test]
#[ignore = "requires deployed programs; scripts/test-external.sh"]
async fn raising_global_default_cannot_reprice_an_existing_real_migration() {
    use anchor_lang::InstructionData;
    for current_rent in [false, true] {
        let (mut f, mut migration, coin, pc, _, _) =
            setup_launch_with_rent(false, 2 * SOL, 1, current_rent).await;
        fresh_wrapper(&mut f, &mut migration).await;
        let mut change = f.params_ix(f.admin.pubkey(), 1, 1, SUPPLY, 30 * SOL, SUPPLY);
        change.data = contract::instruction::SetParams {
            fee_recipient: f.fee,
            owner_wallet: f.admin.pubkey(),
            initial_virtual_token_reserves: SUPPLY,
            initial_virtual_sol_reserves: 30 * SOL,
            initial_real_token_reserves: SUPPLY,
            token_total_supply: SUPPLY,
            fee_basis_points: 1,
            create_fee: 6_900_000,
            staging_pool_fee: 1,
            minimum_migration_sol: 3 * contract::NEW_LAUNCH_MIN_MIGRATION_SOL,
        }
        .data();
        send(&mut f.ctx, &[change], &[&f.admin]).await.unwrap();
        assert_eq!(f.curve().await.minimum_migration_sol, SOL);
        let activation = f.activate_redemption_ix();
        custom_error(
            send(&mut f.ctx, &[activation], &[&f.trader]).await,
            6000 + contract::error::HoldNowCode::MigrationFunded as u32,
        );
        send(&mut f.ctx, &[migration.clone()], &[&f.admin])
            .await
            .unwrap();
        let liquidity = tokens(&mut f.ctx, pc).await;
        assert!(liquidity >= SOL && liquidity < 3 * SOL);
        assert!(f.curve().await.migrated);
        assert_pool_can_swap(&mut f, &migration, coin, pc).await;
        let reward = f.curve().await.stage_reward_pools[0];
        let claim = f.claim_ix(1, reward, vec![], None, None);
        send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
        assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
    }
}

#[tokio::test]
#[ignore = "requires pinned public program snapshots"]
async fn permissionless_migration_before_deadline_burns_lp_and_supports_real_swaps() {
    for current_cluster_rent in [false, true] {
        let (mut f, mut migration, coin, pc, user_lp, lp) =
            setup_launch_with_rent(false, 10 * SOL, 1, current_cluster_rent).await;
        let deadline = f.curve().await.migration_deadline().unwrap();
        clock(&mut f.ctx, deadline - 1).await;
        migration.accounts[25].pubkey = f.outsider.pubkey();
        // Unprivileged callers still cannot choose treasury or migration fees.
        let mut bad = migration.clone();
        bad.accounts[1].pubkey = f.outsider.pubkey();
        error(
            send(&mut f.ctx, &[bad], &[&f.outsider]).await,
            contract::error::HoldNowCode::InValidFeeRecipient,
        );
        send(&mut f.ctx, &[migration.clone()], &[&f.outsider])
            .await
            .unwrap();
        assert!(f.curve().await.migrated);
        assert_eq!(tokens(&mut f.ctx, user_lp).await, 0);
        assert_eq!(
            Mint::unpack(
                &f.ctx
                    .banks_client
                    .get_account(lp)
                    .await
                    .unwrap()
                    .unwrap()
                    .data
            )
            .unwrap()
            .supply,
            0
        );
        assert_pool_can_swap(&mut f, &migration, coin, pc).await;
        clock(&mut f.ctx, deadline).await;
        let activate = f.activate_redemption_ix();
        error(
            send(&mut f.ctx, &[activate], &[&f.trader]).await,
            contract::error::HoldNowCode::AlreadyMigrated,
        );
    }
}

#[tokio::test]
#[ignore = "requires pinned public program snapshots"]
async fn expired_real_migration_cannot_race_or_reopen_redemption() {
    let (mut f, mut migration, _, _, _, _) = setup().await;
    migration.accounts[25].pubkey = f.outsider.pubkey();
    let deadline = f.curve().await.migration_deadline().unwrap();
    clock(&mut f.ctx, deadline).await;
    let balance = lamports(&mut f.ctx, f.vault).await;
    error(
        send(&mut f.ctx, &[migration.clone()], &[&f.outsider]).await,
        contract::error::HoldNowCode::MigrationExpired,
    );
    assert_eq!(lamports(&mut f.ctx, f.vault).await, balance);
    let mut activate = f.activate_redemption_ix();
    activate.accounts[4].pubkey = system_program::id();
    send(&mut f.ctx, &[activate], &[&f.trader]).await.unwrap();
    let donate = solana_system_interface::instruction::transfer(&f.trader.pubkey(), &f.vault, SOL);
    send(&mut f.ctx, &[donate], &[&f.trader]).await.unwrap();
    error(
        send(&mut f.ctx, &[migration], &[&f.outsider]).await,
        contract::error::HoldNowCode::RedemptionActive,
    );
    assert!(!f.curve().await.migrated);
}

fn error(
    result: Result<(), solana_program_test::BanksClientError>,
    code: contract::error::HoldNowCode,
) {
    custom_error(result, 6000 + code as u32);
}

/// The live browser test exposed this with ~1.5 SOL and 30 virtual SOL.
/// Preserve price and rewards across both rent regimes and unsolicited SOL.
#[tokio::test]
#[ignore = "requires deployed programs; scripts/test-external.sh"]
async fn browser_price_regression_donations_and_post_migration_claim() {
    for current_rent in [false, true] {
        for donation in [0, SOL / 3, 40 * SOL] {
            let (mut f, migration, coin, pc, _, _) =
                setup_launch_with_rent(false, 3 * SOL / 2, 1, current_rent).await;
            if donation > 0 {
                let wrapper = migration.accounts[23].pubkey;
                let transfer = solana_system_interface::instruction::transfer(
                    &f.creator.pubkey(),
                    &wrapper,
                    donation,
                );
                send(&mut f.ctx, &[transfer], &[&f.creator]).await.unwrap();
                // Excess over rent in a not-yet-created pool WSOL account must
                // be included in final pricing, not strand migration.
                let pool_dust = solana_system_interface::instruction::transfer(
                    &f.creator.pubkey(),
                    &pc,
                    SOL / 10,
                );
                send(&mut f.ctx, &[pool_dust], &[&f.creator]).await.unwrap();
            }
            let before = snapshot(&mut f).await;
            send(&mut f.ctx, &[migration.clone()], &[&f.admin])
                .await
                .unwrap();
            assert_price_and_supply(&mut f, &before, coin, pc).await;
            if donation == 0 {
                assert!(tokens(&mut f.ctx, coin).await < before.inventory / 10);
            }
            if donation == 40 * SOL {
                assert_eq!(tokens(&mut f.ctx, coin).await, before.inventory);
            }
            let claim = f.claim_ix(1, before.rewards, vec![], None, None);
            send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
            assert_eq!(
                tokens(&mut f.ctx, f.trader_tokens).await,
                before.held + before.rewards
            );
            assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
            assert_pool_can_swap(&mut f, &migration, coin, pc).await;
        }
    }
}

#[tokio::test]
#[ignore = "requires pinned public program snapshots"]
async fn new_launch_profile_minimum_migrates_and_remains_tradable() {
    for current_rent in [false, true] {
        let minimum = contract::NEW_LAUNCH_MIN_MIGRATION_SOL;
        let (mut f, mut migration, coin, pc, user_lp, _) =
            setup_launch_with_shortfall(false, minimum + 2 * SOL, 1, current_rent, None, minimum)
                .await;
        fresh_wrapper(&mut f, &mut migration).await;
        assert_eq!(f.curve().await.minimum_migration_sol, minimum);
        send(&mut f.ctx, &[migration.clone()], &[&f.admin])
            .await
            .unwrap();
        assert!(tokens(&mut f.ctx, pc).await >= minimum);
        assert_eq!(tokens(&mut f.ctx, user_lp).await, 0);
        assert!(f.curve().await.migrated);
        assert_pool_can_swap(&mut f, &migration, coin, pc).await;
        let reward = f.curve().await.stage_reward_pools[0];
        let claim = f.claim_ix(1, reward, vec![], None, None);
        send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
        assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
    }
}

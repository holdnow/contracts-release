use anchor_lang::solana_program::system_program;
use solana_system_interface::instruction as system_instruction;
// Migration ABI boundary tests. The AMM is deliberately a local test double,
// not a claim that the real Raydium/OpenBook deployment has been integration-tested.
use super::*;
use solana_sdk::account::Account;
use spl_token::state::{Account as TokenAccount, AccountState, Mint};

fn token_account(mint: Pubkey, owner: Pubkey, native: bool) -> Account {
    let rent = solana_sdk::rent::Rent::default().minimum_balance(TokenAccount::LEN);
    let value = TokenAccount {
        mint,
        owner,
        amount: 0,
        delegate: COption::None,
        state: AccountState::Initialized,
        is_native: if native {
            COption::Some(rent)
        } else {
            COption::None
        },
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut data = vec![0; TokenAccount::LEN];
    TokenAccount::pack(value, &mut data).unwrap();
    Account {
        lamports: rent,
        data,
        owner: spl_token::id(),
        executable: false,
        rent_epoch: 0,
    }
}

async fn migration(
    f: &mut Fixture,
    fee_bps: u16,
) -> (solana_sdk::instruction::Instruction, Pubkey, Pubkey, Pubkey) {
    let native = spl_token::native_mint::id();
    if f.ctx
        .banks_client
        .get_account(native)
        .await
        .unwrap()
        .is_none()
    {
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
        f.ctx.set_account(
            &native,
            &AccountSharedData::from(Account {
                lamports: solana_sdk::rent::Rent::default().minimum_balance(data.len()),
                data,
                owner: spl_token::id(),
                executable: false,
                rent_epoch: 0,
            }),
        );
    }
    let market = Pubkey::new_unique();
    f.ctx.set_account(
        &market,
        &AccountSharedData::from(Account::new(10_000_000, 0, &contract::MARKET_PROGRAM)),
    );
    let amm = Pubkey::find_program_address(
        &[
            contract::AMM_PROGRAM.as_ref(),
            market.as_ref(),
            b"amm_associated_seed",
        ],
        &contract::AMM_PROGRAM,
    )
    .0;
    let coin_vault = Pubkey::find_program_address(
        &[
            contract::AMM_PROGRAM.as_ref(),
            market.as_ref(),
            b"coin_vault_associated_seed",
        ],
        &contract::AMM_PROGRAM,
    )
    .0;
    let user_pc = Pubkey::new_unique();
    let amm_authority = Pubkey::find_program_address(&[b"amm authority"], &contract::AMM_PROGRAM).0;
    let pc_vault = Pubkey::find_program_address(
        &[
            contract::AMM_PROGRAM.as_ref(),
            market.as_ref(),
            b"pc_vault_associated_seed",
        ],
        &contract::AMM_PROGRAM,
    )
    .0;
    f.ctx.set_account(
        &coin_vault,
        &AccountSharedData::from(token_account(f.mint.pubkey(), amm_authority, false)),
    );
    f.ctx.set_account(
        &pc_vault,
        &AccountSharedData::from(token_account(native, amm_authority, true)),
    );
    f.ctx.set_account(
        &user_pc,
        &AccountSharedData::from(token_account(native, f.vault, true)),
    );
    let lp = Pubkey::new_unique();
    let lp_token = Pubkey::new_unique();
    let mut data = vec![0; Mint::LEN];
    Mint::pack(
        Mint {
            mint_authority: COption::Some(f.vault),
            supply: 0,
            decimals: 9,
            is_initialized: true,
            freeze_authority: COption::None,
        },
        &mut data,
    )
    .unwrap();
    f.ctx.set_account(
        &lp,
        &AccountSharedData::from(Account {
            lamports: solana_sdk::rent::Rent::default().minimum_balance(data.len()),
            data,
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        }),
    );
    f.ctx.set_account(
        &lp_token,
        &AccountSharedData::from(token_account(lp, f.vault, false)),
    );
    let config =
        Pubkey::find_program_address(&[b"amm_config_account_seed"], &contract::AMM_PROGRAM).0;
    f.ctx.set_account(
        &config,
        &AccountSharedData::from(Account::new(10_000_000, 544, &contract::AMM_PROGRAM)),
    );
    let i = ix(
        contract::accounts::MoveRaydium {
            global: f.global,
            fee_recipient: f.fee,
            raydium_fee_destination: contract::AMM_FEE_DESTINATION,
            mint: f.mint.pubkey(),
            vault: f.vault,
            bonding_curve: f.curve,
            amm_program: contract::AMM_PROGRAM,
            token_program: spl_token::id(),
            associated_token_program: spl_associated_token_account::program::id(),
            system_program: system_program::id(),
            sysvar_rent: sysvar::rent::id(),
            amm,
            amm_authority,
            amm_open_orders: Pubkey::new_unique(),
            lp_mint: lp,
            pc_mint: native,
            coin_vault,
            pc_vault,
            target_orders: Pubkey::new_unique(),
            amm_config: config,
            market_program: contract::MARKET_PROGRAM,
            market,
            user_token_coin: f.curve_tokens,
            user_token_pc: user_pc,
            user_token_lp: lp_token,
            user: f.admin.pubkey(),
            clock: sysvar::clock::id(),
        },
        contract::instruction::MoveRaydium { nonce: 0, fee_bps },
    );
    (i, coin_vault, pc_vault, lp_token)
}
#[tokio::test]
async fn authorized_migration_moves_assets_burns_lp_and_cannot_repeat() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(10 * SOL).await;
    f.end_stage().await;
    f.next_stage().await;
    let before = tokens(&mut f.ctx, f.curve_tokens).await;
    let (i, coin, pc, lp) = migration(&mut f, 100).await;
    send(&mut f.ctx, &[i.clone()], &[&f.admin]).await.unwrap();
    let curve = f.curve().await;
    let matched = (tokens(&mut f.ctx, pc).await as u128 * curve.virtual_token_reserves as u128
        / curve.virtual_sol_reserves as u128)
        .min(before as u128) as u64;
    assert_eq!(tokens(&mut f.ctx, coin).await, matched);
    let mint = f
        .ctx
        .banks_client
        .get_account(f.mint.pubkey())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Mint::unpack(&mint.data).unwrap().supply,
        SUPPLY - (before - matched)
    );
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, 0);
    assert!(tokens(&mut f.ctx, pc).await > 9 * SOL);
    assert_eq!(tokens(&mut f.ctx, lp).await, 0);
    assert!(f.curve().await.migrated);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.admin]).await,
        code(HoldNowCode::AlreadyMigrated),
    );
}
#[tokio::test]
async fn regression_migration_rejects_untrusted_program_and_unsigned_caller() {
    let mut f = Fixture::launched(Config::default()).await;
    f.buy(10 * SOL).await;
    let mut c = f.curve().await;
    c.complete = true;
    c.stage_completion_times[c.number_stage as usize] = START;
    f.set_curve(&c).await;
    let (mut i, _, _, _) = migration(&mut f, 100).await;
    i.accounts[25].pubkey = f.outsider.pubkey();
    i.accounts[25].is_signer = false;
    custom_error(
        send(&mut f.ctx, &[i], &[]).await,
        3010, // Anchor AccountNotSigner: permissionless still requires a signer.
    );
    let (mut i, _, _, _) = migration(&mut f, 100).await;
    i.accounts[6].pubkey = MOCK_AMM;
    custom_error(
        send(&mut f.ctx, &[i], &[&f.admin]).await,
        code(HoldNowCode::InvalidMigrationAccount),
    );
}
#[tokio::test]
async fn regression_migration_rejects_changed_fee_recipient_rate_and_asset_owner() {
    let mut f = Fixture::launched(Config::default()).await;
    f.buy(10 * SOL).await;
    let mut c = f.curve().await;
    c.complete = true;
    c.stage_completion_times[c.number_stage as usize] = START;
    f.set_curve(&c).await;
    let (mut i, _, _, _) = migration(&mut f, 100).await;
    i.accounts[1].pubkey = f.outsider.pubkey();
    custom_error(
        send(&mut f.ctx, &[i], &[&f.admin]).await,
        code(HoldNowCode::InValidFeeRecipient),
    );
    let (i, _, _, _) = migration(&mut f, 9_900).await;
    custom_error(
        send(&mut f.ctx, &[i], &[&f.admin]).await,
        code(HoldNowCode::InvalidFeeRate),
    );
    let (mut i, _, _, _) = migration(&mut f, 100).await;
    i.accounts[23].pubkey = f.trader_tokens;
    custom_error(
        send(&mut f.ctx, &[i], &[&f.admin]).await,
        code(HoldNowCode::InvalidMigrationAccount),
    );
}
#[tokio::test]
async fn migration_rejects_unfinished_curve_bad_pdas_and_wrong_destination() {
    let mut f = Fixture::launched(Config::default()).await;
    f.buy(10 * SOL).await;
    let (i, _, _, _) = migration(&mut f, 100).await;
    custom_error(
        send(&mut f.ctx, &[i], &[&f.admin]).await,
        code(HoldNowCode::BondingCurveNotComplete),
    );
    let mut c = f.curve().await;
    c.complete = true;
    c.stage_completion_times[c.number_stage as usize] = START;
    c.token_pool_destination = 2;
    f.set_curve(&c).await;
    let (i, _, _, _) = migration(&mut f, 100).await;
    custom_error(
        send(&mut f.ctx, &[i], &[&f.admin]).await,
        code(HoldNowCode::IncorrectPoolDestination),
    );
    c.token_pool_destination = 3;
    f.set_curve(&c).await;
    let (mut i, _, _, _) = migration(&mut f, 100).await;
    i.accounts[4].pubkey = f.outsider.pubkey();
    custom_error(
        send(&mut f.ctx, &[i], &[&f.admin]).await,
        code(HoldNowCode::InvalidMigrationAccount),
    );
}

#[tokio::test]
async fn missing_lp_rolls_back_every_transfer() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(10 * SOL).await;
    f.end_stage().await;
    f.next_stage().await;
    let (i, _, _, _) = migration(&mut f, 100).await;
    let config = i.accounts[19].pubkey;
    let mut account = Account::new(10_000_000, 544, &contract::AMM_PROGRAM);
    account.data[0] = 1;
    f.ctx
        .set_account(&config, &AccountSharedData::from(account));
    let coin = tokens(&mut f.ctx, f.curve_tokens).await;
    let sol = lamports(&mut f.ctx, f.vault).await;
    custom_error(
        send(&mut f.ctx, &[i], &[&f.admin]).await,
        code(HoldNowCode::InvalidLiquidity),
    );
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, coin);
    assert_eq!(lamports(&mut f.ctx, f.vault).await, sol);
    assert!(!f.curve().await.migrated);
}
#[tokio::test]
async fn wrapping_account_dust_cannot_block_migration() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(10 * SOL).await;
    f.end_stage().await;
    f.next_stage().await;
    let (i, _, _, _) = migration(&mut f, 100).await;
    let pc = i.accounts[23].pubkey;
    let donation = system_instruction::transfer(&f.outsider.pubkey(), &pc, 1);
    send(&mut f.ctx, &[donation], &[&f.outsider]).await.unwrap();
    send(&mut f.ctx, &[i], &[&f.admin]).await.unwrap();
    assert!(f.curve().await.migrated);
}

#[tokio::test]
async fn canonical_amm_configuration_rejects_wrong_address_owner_or_layout_atomically() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(10 * SOL).await;
    f.end_stage().await;
    f.next_stage().await;
    for invalid in 0..3 {
        let (mut instruction, _, _, _) = migration(&mut f, 100).await;
        let config = instruction.accounts[19].pubkey;
        if invalid == 0 {
            instruction.accounts[19].pubkey = f.outsider.pubkey();
        } else {
            // Model incompatible external-program configuration, not a user's ability to rewrite it.
            let account = if invalid == 1 {
                Account::new(10_000_000, 544, &system_program::id())
            } else {
                Account::new(10_000_000, 543, &contract::AMM_PROGRAM)
            };
            f.ctx
                .set_account(&config, &AccountSharedData::from(account));
        }
        let before = tokens(&mut f.ctx, f.curve_tokens).await;
        let sol = lamports(&mut f.ctx, f.vault).await;
        custom_error(
            send(&mut f.ctx, &[instruction], &[&f.admin]).await,
            code(HoldNowCode::InvalidMigrationAccount),
        );
        assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, before);
        assert_eq!(lamports(&mut f.ctx, f.vault).await, sol);
        assert!(!f.curve().await.migrated);
    }
}
#[tokio::test]
async fn incorrect_market_owner_and_quote_mint_are_rejected_before_transfers() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(10 * SOL).await;
    f.end_stage().await;
    f.next_stage().await;
    for invalid in 0..2 {
        let (mut instruction, _, _, _) = migration(&mut f, 100).await;
        if invalid == 0 {
            instruction.accounts[21].pubkey = f.outsider.pubkey();
        } else {
            instruction.accounts[15].pubkey = f.mint.pubkey();
        }
        let before = lamports(&mut f.ctx, f.vault).await;
        custom_error(
            send(&mut f.ctx, &[instruction], &[&f.admin]).await,
            if invalid == 0 {
                code(HoldNowCode::InvalidMigrationAccount)
            } else {
                2040
            }, // Anchor rejects duplicate mutable mint accounts
        );
        assert_eq!(lamports(&mut f.ctx, f.vault).await, before);
    }
}
#[tokio::test]
async fn an_unaffordable_external_creation_fee_cannot_partially_spend_the_vault() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(10 * SOL).await;
    f.end_stage().await;
    f.next_stage().await;
    let (instruction, _, _, _) = migration(&mut f, 100).await;
    let config = instruction.accounts[19].pubkey;
    let mut account = Account::new(10_000_000, 544, &contract::AMM_PROGRAM);
    account.data[536..544].copy_from_slice(&(1000 * SOL).to_le_bytes());
    f.ctx
        .set_account(&config, &AccountSharedData::from(account));
    let before = lamports(&mut f.ctx, f.vault).await;
    custom_error(
        send(&mut f.ctx, &[instruction], &[&f.admin]).await,
        code(HoldNowCode::InsufficientVaultFunds),
    );
    assert_eq!(lamports(&mut f.ctx, f.vault).await, before);
    assert!(!f.curve().await.migrated);
}
#[tokio::test]
async fn wrong_lp_recipient_owner_rolls_back_even_after_the_amm_moves_assets() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(10 * SOL).await;
    f.end_stage().await;
    f.next_stage().await;
    let (instruction, _, _, lp) = migration(&mut f, 100).await;
    f.ctx.set_account(
        &lp,
        &AccountSharedData::from(token_account(
            instruction.accounts[14].pubkey,
            f.outsider.pubkey(),
            false,
        )),
    );
    let before = tokens(&mut f.ctx, f.curve_tokens).await;
    let sol = lamports(&mut f.ctx, f.vault).await;
    custom_error(
        send(&mut f.ctx, &[instruction], &[&f.admin]).await,
        code(HoldNowCode::InvalidLiquidity),
    );
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, before);
    assert_eq!(lamports(&mut f.ctx, f.vault).await, sol);
    assert!(!f.curve().await.migrated);
}

#[tokio::test]
async fn regression_surplus_rejects_wrong_pool_owner_after_cpi_without_partial_movement() {
    let mut f = Fixture::launched(Config {
        stages: 1,
        ..Config::default()
    })
    .await;
    f.buy(10 * SOL).await;
    f.end_stage().await;
    f.next_stage().await;
    let (migration, coin, pc, lp) = migration(&mut f, 100).await;
    let original = f.ctx.banks_client.get_account(pc).await.unwrap().unwrap();
    let mut bad = original.clone();
    let mut token = TokenAccount::unpack(&bad.data).unwrap();
    token.owner = f.outsider.pubkey();
    TokenAccount::pack(token, &mut bad.data).unwrap();
    f.ctx.set_account(&pc, &AccountSharedData::from(bad));
    let keys = [
        f.curve,
        f.vault,
        f.curve_tokens,
        f.fee,
        coin,
        pc,
        lp,
        migration.accounts[14].pubkey,
    ];
    let mut before = vec![];
    for key in keys {
        before.push(f.ctx.banks_client.get_account(key).await.unwrap());
    }
    custom_error(
        send(&mut f.ctx, &[migration.clone()], &[&f.admin]).await,
        code(HoldNowCode::InvalidMigrationAccount),
    );
    for (key, expected) in keys.into_iter().zip(before) {
        assert_eq!(f.ctx.banks_client.get_account(key).await.unwrap(), expected);
    }
    f.ctx.set_account(&pc, &AccountSharedData::from(original));
    send(&mut f.ctx, &[migration], &[&f.admin]).await.unwrap();
    assert_eq!(
        lamports(&mut f.ctx, f.vault).await,
        solana_sdk::rent::Rent::default().minimum_balance(0)
    );
}

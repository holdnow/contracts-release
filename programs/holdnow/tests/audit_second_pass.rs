#![allow(dead_code, unused_imports)]
//! Stateful SBPF audit: real instructions only after the common valid Global genesis.
//! No mutation of curve, mint, sale or claim state is used to manufacture success.
mod common;
use anchor_lang::InstructionData;
use common::*;
use solana_sdk::{
    account::{Account, AccountSharedData},
    instruction::Instruction,
    program_pack::Pack,
    pubkey::Pubkey,
    rent::Rent,
    signature::Signer,
};
use spl_token::state::Mint;

fn next(rng: &mut u64) -> u64 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    *rng
}
fn pair(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    let (a, b) = if a < b { (a, b) } else { (b, a) };
    solana_sdk::keccak::hashv(&[&a, &b]).to_bytes()
}
fn tree(
    users: &[Pubkey; 3],
    mint: Pubkey,
    stage: u8,
    amounts: &[u64; 3],
) -> (Pubkey, [Vec<[u8; 32]>; 3]) {
    let leaves = [
        regular_leaf(users[0], mint, stage, amounts[0]),
        regular_leaf(users[1], mint, stage, amounts[1]),
        regular_leaf(users[2], mint, stage, amounts[2]),
        [0; 32],
    ];
    let left = pair(leaves[0], leaves[1]);
    let right = pair(leaves[2], leaves[3]);
    (
        Pubkey::new_from_array(pair(left, right)),
        [
            vec![leaves[1], right],
            vec![leaves[0], right],
            vec![leaves[3], left],
        ],
    )
}
fn buy_for(f: &Fixture, user: Pubkey, token: Pubkey, amount: u64) -> Instruction {
    let mut ix = f.buy_ix(amount, 0);
    ix.accounts[8].pubkey = token;
    ix.accounts[9].pubkey = user;
    ix
}
fn claim_for(
    f: &Fixture,
    user: Pubkey,
    token: Pubkey,
    stage: u8,
    amount: u64,
    proof: Vec<[u8; 32]>,
) -> Instruction {
    let mut ix = f.claim_ix(stage, amount, proof, None, None);
    for (index, seed) in [
        (2, contract::USER_CLAIM_STATE_SEED),
        (3, contract::USER_VESTING_STATE_SEED),
    ] {
        ix.accounts[index].pubkey = Pubkey::find_program_address(
            &[seed, f.mint.pubkey().as_ref(), user.as_ref()],
            &contract::ID,
        )
        .0;
    }
    ix.accounts[6].pubkey = token;
    ix.accounts[7].pubkey = user;
    ix
}
async fn invariants(
    f: &mut Fixture,
    accounts: &[Pubkey; 3],
    extra_curve_tokens: u64,
    extra_sol: u64,
) {
    let c = f.curve().await;
    let reserve_tokens = tokens(&mut f.ctx, f.curve_tokens).await;
    let rewards = tokens(&mut f.ctx, f.reward_tokens).await;
    let mut circulating = 0u64;
    for account in accounts {
        circulating += tokens(&mut f.ctx, *account).await;
    }
    let mint = f
        .ctx
        .banks_client
        .get_account(f.mint.pubkey())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Mint::unpack(&mint.data).unwrap().supply,
        reserve_tokens + rewards + circulating
    );
    assert_eq!(reserve_tokens, c.real_token_reserves + extra_curve_tokens);
    assert_eq!(
        rewards,
        c.stage_reward_pools.iter().sum::<u64>() - c.total_regular_claimed
    );
    let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
    if !c.redemption_mode {
        assert_eq!(
            lamports(&mut f.ctx, f.vault).await,
            rent.minimum_balance(0) + c.real_sol_reserves + extra_sol
        );
    }
}
async fn snapshot(f: &mut Fixture, keys: &[Pubkey]) -> Vec<Option<Account>> {
    let mut out = vec![];
    for key in keys {
        out.push(f.ctx.banks_client.get_account(*key).await.unwrap());
    }
    out
}

#[tokio::test]
async fn seeded_multistage_trading_settlement_and_redemption_conserve_assets() {
    // Eight lifecycles/configuration, three wallets, both actual runtime rent schedules.
    for current_cluster_rent in [false, true] {
        for seed in [1u64, 7, 29, 101] {
            let mut rng = seed;
            let mut f = Fixture::minted_with_vault_prefunding(
                Config {
                    current_cluster_rent,
                    ..Config::default()
                },
                false,
            )
            .await;
            let init = f.initialize_curve_ix(f.creator.pubkey(), 3);
            send(&mut f.ctx, &[init], &[&f.creator]).await.unwrap();
            let create = f.create_ix();
            send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
            let owners = [
                f.trader.insecure_clone(),
                f.creator.insecure_clone(),
                f.outsider.insecure_clone(),
            ];
            let users = owners.each_ref().map(|u| u.pubkey());
            let third = spl_associated_token_account::address::get_associated_token_address(
                &users[2],
                &f.mint.pubkey(),
            );
            let accounts = [f.trader_tokens, f.creator_tokens, third];
            let ata = spl_associated_token_account::instruction::create_associated_token_account(
                &f.ctx.payer.pubkey(),
                &users[2],
                &f.mint.pubkey(),
                &spl_token::id(),
            );
            send(&mut f.ctx, &[ata], &[]).await.unwrap();
            let mut cumulative = [0u64; 3];
            let mut paid = [0u64; 3];
            let mut publications = vec![];
            let mut extra_curve = 0;
            let mut extra_sol = 0;
            let mut user_burned = 0;
            let mut swaps = 0;
            for stage in 1..=3u8 {
                let start = f.curve().await.at_start_stage;
                // Every wallet starts each stage with a material balance.
                for j in 0..3 {
                    let buy = buy_for(&f, users[j], accounts[j], 30_000 + next(&mut rng) % 20_000);
                    send(&mut f.ctx, &[buy], &[&owners[j]]).await.unwrap();
                    swaps += 1;
                    invariants(&mut f, &accounts, extra_curve, extra_sol).await;
                }
                let mut sold = [false; 3];
                for step in 0..15 {
                    clock(
                        &mut f.ctx,
                        start + 1 + step * (contract::STAGE_UNIT_SECONDS as i64 - 2) / 15,
                    )
                    .await;
                    let j = next(&mut rng) as usize % 3;
                    let held = tokens(&mut f.ctx, accounts[j]).await;
                    let before = f.curve().await;
                    match next(&mut rng) % 5 {
                        0 => {
                            let buy = buy_for(
                                &f,
                                users[j],
                                accounts[j],
                                10_000 + next(&mut rng) % 20_000,
                            );
                            send(&mut f.ctx, &[buy], &[&owners[j]]).await.unwrap();
                            swaps += 1;
                        }
                        1 if held > 100_000 => {
                            let sell = f.sell_ix(users[j], accounts[j], held / 7, 1);
                            send(&mut f.ctx, &[sell], &[&owners[j]]).await.unwrap();
                            sold[j] = true;
                            swaps += 1;
                        }
                        2 if held > 100 => {
                            let transfer = spl_token::instruction::transfer(
                                &spl_token::id(),
                                &accounts[j],
                                &accounts[(j + 1) % 3],
                                &users[j],
                                &[],
                                held / 11,
                            )
                            .unwrap();
                            send(&mut f.ctx, &[transfer], &[&owners[j]]).await.unwrap();
                        }
                        3 if held > 100 => {
                            let amount = 1 + next(&mut rng) % 99;
                            let burn = spl_token::instruction::burn(
                                &spl_token::id(),
                                &accounts[j],
                                &f.mint.pubkey(),
                                &users[j],
                                &[],
                                amount,
                            )
                            .unwrap();
                            send(&mut f.ctx, &[burn], &[&owners[j]]).await.unwrap();
                            user_burned += amount;
                        }
                        _ => {
                            let donate = solana_system_interface::instruction::transfer(
                                &users[j], &f.vault, 13,
                            );
                            let token = spl_token::instruction::transfer(
                                &spl_token::id(),
                                &accounts[j],
                                &f.curve_tokens,
                                &users[j],
                                &[],
                                1,
                            )
                            .unwrap();
                            send(&mut f.ctx, &[donate, token], &[&owners[j]])
                                .await
                                .unwrap();
                            extra_sol += 13;
                            extra_curve += 1;
                        }
                    }
                    let after = f.curve().await;
                    assert!(
                        after.virtual_sol_reserves as u128 * after.virtual_token_reserves as u128
                            >= before.virtual_sol_reserves as u128
                                * before.virtual_token_reserves as u128
                    );
                    invariants(&mut f, &accounts, extra_curve, extra_sol).await;
                }
                // Explicitly exercise intermediate carry and final burn with real seller records.
                if stage == 1 || (stage == 3 && seed % 3 != 1) {
                    for j in 0..3 {
                        let amount = tokens(&mut f.ctx, accounts[j]).await / 20;
                        let sell = f.sell_ix(users[j], accounts[j], amount, 1);
                        send(&mut f.ctx, &[sell], &[&owners[j]]).await.unwrap();
                        sold[j] = true;
                        swaps += 1;
                    }
                }
                f.end_stage().await;
                let mut balances = [0u64; 3];
                for j in 0..3 {
                    let sale = Pubkey::find_program_address(
                        &[
                            contract::USER_SALE_STATE_SEED,
                            f.mint.pubkey().as_ref(),
                            users[j].as_ref(),
                        ],
                        &contract::ID,
                    )
                    .0;
                    let record = f.ctx.banks_client.get_account(sale).await.unwrap();
                    let chain_sold = if record.is_some() {
                        state::<contract::UserSaleState>(&mut f.ctx, sale)
                            .await
                            .last_sold_stage
                            == stage
                    } else {
                        false
                    };
                    assert_eq!(sold[j], chain_sold);
                    if !chain_sold {
                        balances[j] = tokens(&mut f.ctx, accounts[j]).await;
                    }
                }
                let eligible: u64 = balances.iter().sum();
                let pool = f.curve().await.stage_reward_pools[stage as usize - 1];
                if eligible > 0 {
                    // Independent exact apportionment oracle; published roots use cumulative totals.
                    let mut awards =
                        balances.map(|b| (b as u128 * pool as u128 / eligible as u128) as u64);
                    let mut order = [0usize, 1, 2];
                    order.sort_by(|&a, &b| {
                        (balances[b] as u128 * pool as u128 % eligible as u128)
                            .cmp(&(balances[a] as u128 * pool as u128 % eligible as u128))
                            .then_with(|| users[a].to_string().cmp(&users[b].to_string()))
                    });
                    for index in order
                        .into_iter()
                        .take((pool - awards.iter().sum::<u64>()) as usize)
                    {
                        awards[index] += 1;
                    }
                    assert_eq!(awards.iter().sum::<u64>(), pool);
                    for j in 0..3 {
                        cumulative[j] += awards[j];
                    }
                }
                let (root, proofs) = tree(&users, f.mint.pubkey(), stage, &cumulative);
                let mut publish = f.root_ix(f.admin.pubkey(), stage, root);
                publish.data = contract::instruction::UpdateMerkleRoot {
                    stage,
                    merkle_root: root,
                    total_eligible_supply: eligible,
                }
                .data();
                send(&mut f.ctx, &[publish], &[&f.admin]).await.unwrap();
                invariants(&mut f, &accounts, extra_curve, extra_sol).await;
                let j = (seed as usize + stage as usize) % 3;
                if cumulative[j] > paid[j] {
                    let claim = claim_for(
                        &f,
                        users[j],
                        accounts[j],
                        stage,
                        cumulative[j],
                        proofs[j].clone(),
                    );
                    send(&mut f.ctx, &[claim], &[&owners[j]]).await.unwrap();
                    paid[j] = cumulative[j];
                    invariants(&mut f, &accounts, extra_curve, extra_sol).await;
                }
                publications.push((stage, cumulative, proofs));
                f.next_stage().await;
            }
            assert!(f.curve().await.complete);
            let cluster = if cfg!(feature = "devnet") {
                "devnet"
            } else {
                "mainnet-beta"
            };
            let data = std::fs::read(format!("../../audit/external/{cluster}-config.bin")).unwrap();
            let rent: Rent = f.ctx.banks_client.get_sysvar().await.unwrap();
            let config =
                Pubkey::find_program_address(&[b"amm_config_account_seed"], &contract::AMM_PROGRAM)
                    .0;
            f.ctx.set_account(
                &config,
                &AccountSharedData::from(Account {
                    lamports: rent.minimum_balance(data.len()),
                    data,
                    owner: contract::AMM_PROGRAM,
                    executable: false,
                    rent_epoch: 0,
                }),
            );
            let activate = f.activate_redemption_ix();
            send(&mut f.ctx, &[activate], &[&f.trader]).await.unwrap();
            // Claims stay valid after completion/redemption activation, newest then oldest.
            for (stage, amounts, proofs) in publications.iter().rev() {
                for j in 0..3 {
                    let before = tokens(&mut f.ctx, accounts[j]).await;
                    let claim = claim_for(
                        &f,
                        users[j],
                        accounts[j],
                        *stage,
                        amounts[j],
                        proofs[j].clone(),
                    );
                    let result = send(&mut f.ctx, &[claim], &[&owners[j]]).await;
                    if amounts[j] > paid[j] {
                        result.unwrap();
                        assert_eq!(
                            tokens(&mut f.ctx, accounts[j]).await - before,
                            amounts[j] - paid[j]
                        );
                        paid[j] = amounts[j];
                    } else {
                        assert!(result.is_err());
                        assert_eq!(tokens(&mut f.ctx, accounts[j]).await, before);
                    }
                    invariants(&mut f, &accounts, extra_curve, extra_sol).await;
                }
            }
            assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
            let mint = f
                .ctx
                .banks_client
                .get_account(f.mint.pubkey())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                SUPPLY - Mint::unpack(&mint.data).unwrap().supply,
                user_burned + f.curve().await.total_unallocated_burned
            );
            for j in 0..3 {
                let held = tokens(&mut f.ctx, accounts[j]).await;
                let mut redeem = f.redeem_ix(held, 1);
                redeem.accounts[4].pubkey = accounts[j];
                redeem.accounts[5].pubkey = users[j];
                send(&mut f.ctx, &[redeem], &[&owners[j]]).await.unwrap();
                invariants(&mut f, &accounts, extra_curve, extra_sol).await;
            }
            assert_eq!(lamports(&mut f.ctx, f.vault).await, rent.minimum_balance(0));
            println!("STATEFUL_LIFECYCLE seed={seed} current_rent={current_cluster_rent} swaps={swaps} final_burn={} total_paid={}", f.curve().await.total_unallocated_burned, paid.iter().sum::<u64>());
        }
    }
}

#[tokio::test]
async fn failed_later_instruction_rolls_back_sale_and_seller_eligibility() {
    let mut f = Fixture::launched(Config::default()).await;
    f.buy(SOL).await;
    let sell = f.sell_ix(f.trader.pubkey(), f.trader_tokens, 1_000_000, 1);
    let seller_state = sell.accounts.last().unwrap().pubkey;
    let keys = [
        f.curve,
        f.vault,
        f.reward_tokens,
        f.curve_tokens,
        f.trader_tokens,
        f.trader.pubkey(),
        f.fee,
        seller_state,
    ];
    let before = snapshot(&mut f, &keys).await;
    let invalid = f.claim_ix(1, 1, vec![], None, None); // stage is still trading
    assert!(send(&mut f.ctx, &[sell.clone(), invalid], &[&f.trader])
        .await
        .is_err());
    assert_eq!(snapshot(&mut f, &keys).await, before);
    assert!(f
        .ctx
        .banks_client
        .get_account(seller_state)
        .await
        .unwrap()
        .is_none());
    send(&mut f.ctx, &[sell], &[&f.trader]).await.unwrap();
    assert_eq!(
        state::<contract::UserSaleState>(&mut f.ctx, seller_state)
            .await
            .last_sold_stage,
        1
    );
}

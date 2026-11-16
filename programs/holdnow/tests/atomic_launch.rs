#![allow(dead_code, unused_imports)]
mod common;
use common::*;
use solana_sdk::{pubkey::Pubkey, signature::Signer};

#[tokio::test]
async fn creation_csv_escrow_and_creator_buy_commit_or_roll_back_together() {
    for allocation in [0, 199_676_443_201_226] {
        for stages in [1, 3, 100] {
            for fail_buy in [false, true] {
                let mut f = Fixture::minted_with_vault_prefunding(
                    Config {
                        airdrop: allocation,
                        stages,
                        root: if allocation == 0 {
                            Pubkey::default()
                        } else {
                            Pubkey::new_unique()
                        },
                        ..Config::default()
                    },
                    false,
                )
                .await;
                let creator_sol = lamports(&mut f.ctx, f.creator.pubkey()).await;
                let fee_sol = lamports(&mut f.ctx, f.fee).await;
                let (tokens_left, virtual_sol) =
                    contract::utils::launch_reserves(SUPPLY, 30 * SOL, allocation).unwrap();
                let net = 99_000_000;
                let gross = contract::utils::curve_output(net, virtual_sol, tokens_left).unwrap();
                let reward = gross / 100;
                let output = gross - reward;
                let mut buy = f.buy_ix(
                    100_000_000,
                    if fail_buy { SUPPLY } else { output * 99 / 100 },
                );
                buy.accounts[8].pubkey = f.creator_tokens;
                buy.accounts[9].pubkey = f.creator.pubkey();
                let instructions = [
                    f.initialize_curve_ix(f.creator.pubkey(), stages),
                    f.create_ix(),
                    buy,
                ];
                let result = send(&mut f.ctx, &instructions, &[&f.creator]).await;
                if fail_buy {
                    assert!(result.is_err());
                    assert_eq!(tokens(&mut f.ctx, f.creator_tokens).await, SUPPLY);
                    assert_eq!(lamports(&mut f.ctx, f.creator.pubkey()).await, creator_sol);
                    assert_eq!(lamports(&mut f.ctx, f.fee).await, fee_sol);
                    for account in [
                        f.curve,
                        f.curve_tokens,
                        f.reward_authority,
                        f.reward_tokens,
                        f.vault,
                    ] {
                        assert!(f
                            .ctx
                            .banks_client
                            .get_account(account)
                            .await
                            .unwrap()
                            .is_none());
                    }
                } else {
                    result.unwrap();
                    let curve = f.curve().await;
                    assert_eq!(curve.initial_allocation_merkle_root, f.config.root);
                    assert_eq!(curve.airdrop_amount, allocation);
                    assert_eq!(curve.current_stage, 1);
                    assert_eq!(curve.real_sol_reserves, net);
                    assert_eq!(curve.stage_reward_pools[0], reward);
                    assert_eq!(tokens(&mut f.ctx, f.creator_tokens).await, output);
                    assert_eq!(
                        tokens(&mut f.ctx, f.reward_tokens).await,
                        allocation + reward
                    );
                    assert_eq!(
                        tokens(&mut f.ctx, f.curve_tokens).await,
                        SUPPLY - allocation - gross
                    );
                }
            }
        }
    }
}

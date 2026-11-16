#![allow(dead_code, unused_imports)]
use anchor_lang::solana_program::system_program;
// Transaction-level tests against the actual Anchor entrypoint and SPL programs.
// `regression_` tests require the previously demonstrated attacks to fail.
mod common;
use anchor_lang::{AccountDeserialize, AccountSerialize, InstructionData};
use common::*;
use contract::{error::HoldNowCode, BondingCurve, Global, UserClaimState, UserVestingState};
use solana_sdk::{
    account::AccountSharedData,
    program_option::COption,
    program_pack::Pack,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    sysvar,
};

fn code(e: HoldNowCode) -> u32 {
    6000 + e as u32
}

#[tokio::test]
async fn lifecycle_creation_buy_sell_stage_claim_next_stage_and_completion() {
    let mut f = Fixture::launched(Config::default()).await;
    let c = f.curve().await;
    assert_eq!(c.current_stage, 1);
    assert!(!c.complete);
    assert!(!c.current_stage_complete);
    assert_eq!(c.stage_duration, contract::STAGE_UNIT_SECONDS as i64);
    assert_eq!(c.claim_duration, 60);
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, SUPPLY);
    assert_eq!(tokens(&mut f.ctx, f.creator_tokens).await, 0);
    assert_eq!(c.cumulative_merkle_roots.len(), 4);
    let fee_before = lamports(&mut f.ctx, f.fee).await;
    f.buy(SOL).await;
    let bought = tokens(&mut f.ctx, f.trader_tokens).await;
    assert!(bought > 0);
    assert_eq!(lamports(&mut f.ctx, f.fee).await - fee_before, SOL / 100);
    assert_eq!(
        f.curve().await.stage_reward_pools[0],
        tokens(&mut f.ctx, f.reward_tokens).await
    );
    clock(&mut f.ctx, START + contract::STAGE_UNIT_SECONDS as i64 / 2).await;
    let sale = f.sell_ix(f.trader.pubkey(), f.trader_tokens, bought / 2, 0);
    send(&mut f.ctx, &[sale], &[&f.trader]).await.unwrap();
    assert_eq!(
        tokens(&mut f.ctx, f.trader_tokens).await,
        bought - bought / 2
    );
    f.end_stage().await;
    assert!(f.curve().await.current_stage_complete);
    let pool = f.curve().await.stage_reward_pools[0];
    let root = Pubkey::new_from_array(regular_leaf(f.trader.pubkey(), f.mint.pubkey(), 1, pool));
    let update = f.root_ix(f.admin.pubkey(), 1, root);
    send(&mut f.ctx, &[update], &[&f.admin]).await.unwrap();
    let before = tokens(&mut f.ctx, f.trader_tokens).await;
    let claim = f.claim_ix(1, pool, vec![], None, None);
    send(&mut f.ctx, &[claim], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, before + pool);
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, 0);
    let claim = f.claim_ix(1, pool, vec![], None, None);
    custom_error(
        send(&mut f.ctx, &[claim], &[&f.trader]).await,
        code(HoldNowCode::NothingToClaim),
    );
    f.next_stage().await;
    assert_eq!(f.curve().await.current_stage, 2);
    f.end_stage().await;
    f.next_stage().await;
    f.end_stage().await;
    f.next_stage().await;
    assert!(f.curve().await.complete);
    assert_eq!(f.curve().await.current_stage, 4);
}

#[tokio::test]
async fn authority_checks_reject_unauthorized_global_initialization_configuration_and_roots() {
    let mut f = Fixture::launched(Config::default()).await;
    let i = f.params_ix(f.outsider.pubkey(), 1, 1, SUPPLY, 30 * SOL, SUPPLY);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.outsider]).await,
        code(HoldNowCode::NotAuthorized),
    );
    let i = f.root_ix(f.outsider.pubkey(), 1, Pubkey::new_unique());
    custom_error(send(&mut f.ctx, &[i], &[&f.outsider]).await, 2001); // Anchor has_one
    let i = f.initial_ix(
        f.creator.pubkey(),
        f.outsider.pubkey(),
        Pubkey::new_unique(),
        3,
    );
    custom_error(
        send(&mut f.ctx, &[i], &[&f.creator, &f.outsider]).await,
        code(HoldNowCode::NotAuthorized),
    );
    // Clear the fixture's global account only to exercise the real initialization transaction.
    f.ctx.set_account(&f.global, &AccountSharedData::default());
    let i = ix(
        contract::accounts::Initialize {
            global: f.global,
            owner: f.outsider.pubkey(),
            system_program: system_program::id(),
        },
        contract::instruction::Initialize {},
    );
    custom_error(
        send(&mut f.ctx, &[i], &[&f.outsider]).await,
        code(HoldNowCode::NotAuthorized),
    );
}

#[tokio::test]
async fn trading_rejects_zero_slippage_wrong_fee_and_paused_or_completed_stages_atomically() {
    let mut f = Fixture::launched(Config::default()).await;
    let before = tokens(&mut f.ctx, f.curve_tokens).await;
    let i = f.buy_ix(0, 0);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::ZeroAmount),
    );
    let i = f.buy_ix(SOL, u64::MAX);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::InsufficientOutput),
    );
    let mut i = f.buy_ix(SOL, 0);
    i.accounts[1].pubkey = f.outsider.pubkey();
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::InValidFeeRecipient),
    );
    assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, before);
    f.buy(SOL).await;
    let i = f.sell_ix(f.trader.pubkey(), f.trader_tokens, 0, 0);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::ZeroAmount),
    );
    let i = f.sell_ix(f.trader.pubkey(), f.trader_tokens, 1_000_000, u64::MAX);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::TooLittleSolReceived),
    );
    f.end_stage().await;
    let i = f.buy_ix(SOL, 0);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::CurrentStageComplete),
    );
    let i = f.sell_ix(f.trader.pubkey(), f.trader_tokens, 1, 0);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::CurrentStageComplete),
    );
    let mut c = f.curve().await;
    c.current_stage_complete = false;
    c.complete = true;
    f.set_curve(&c).await;
    let i = f.buy_ix(SOL, 0);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::BondingCurveComplete),
    );
    let i = f.sell_ix(f.trader.pubkey(), f.trader_tokens, 1, 0);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::BondingCurveComplete),
    );
}

#[tokio::test]
async fn stage_time_boundaries_and_claim_time_boundaries_are_enforced() {
    let mut f = Fixture::launched(Config::default()).await;
    clock(&mut f.ctx, START + contract::STAGE_UNIT_SECONDS as i64 - 1).await;
    let i = f.stage_ix();
    custom_error(
        send(&mut f.ctx, &[i], &[&f.outsider]).await,
        code(HoldNowCode::NotStageComplete),
    );
    f.end_stage().await;
    f.settle_stage().await;
    clock(&mut f.ctx, START + contract::STAGE_UNIT_SECONDS as i64 + 59).await;
    let i = f.stage_ix();
    custom_error(
        send(&mut f.ctx, &[i], &[&f.outsider]).await,
        code(HoldNowCode::NotMoveNextStage),
    );
    f.next_stage().await;
    assert_eq!(f.curve().await.current_stage, 2);
}

#[tokio::test]
async fn creation_rejects_invalid_claim_windows_and_airdrops_without_partial_transfers() {
    for (claim, expected) in [
        (59, HoldNowCode::ClaimWindowTooShort),
        (604_801, HoldNowCode::ClaimWindowTooLong),
    ] {
        let mut f = Fixture::initialized(Config {
            claim,
            ..Config::default()
        })
        .await;
        let i = f.create_ix();
        custom_error(send(&mut f.ctx, &[i], &[&f.creator]).await, code(expected));
        assert_eq!(tokens(&mut f.ctx, f.creator_tokens).await, SUPPLY);
        assert_eq!(tokens(&mut f.ctx, f.curve_tokens).await, 0);
    }
    let mut f = Fixture::initialized(Config {
        airdrop: SUPPLY + 1,
        ..Config::default()
    })
    .await;
    let i = f.create_ix();
    custom_error(
        send(&mut f.ctx, &[i], &[&f.creator]).await,
        code(HoldNowCode::InvalidAirdropAmount),
    );
}

#[tokio::test]
async fn configuration_validates_authority_and_zero_reserves() {
    let mut f = Fixture::launched(Config::default()).await;
    for (fee, t, s, supply, expected) in [
        (
            10_001,
            SUPPLY,
            30 * SOL,
            SUPPLY,
            HoldNowCode::InvalidFeeBasisPoints,
        ),
        (1, 0, 30 * SOL, SUPPLY, HoldNowCode::InvalidAmount),
        (1, SUPPLY, 0, SUPPLY, HoldNowCode::InvalidAmount),
        (1, SUPPLY, 30 * SOL, 0, HoldNowCode::InvalidSupply),
    ] {
        let i = f.params_ix(f.admin.pubkey(), fee, 1, t, s, supply);
        custom_error(send(&mut f.ctx, &[i], &[&f.admin]).await, code(expected));
    }
    let i = f.params_ix(f.admin.pubkey(), 2, 2, SUPPLY, 30 * SOL, SUPPLY);
    send(&mut f.ctx, &[i], &[&f.admin]).await.unwrap();
    let g: Global = state(&mut f.ctx, f.global).await;
    assert_eq!(g.fee_basis_points, 2);
}

#[tokio::test]
async fn claim_proofs_bind_user_mint_stage_amount_and_payout_is_atomic() {
    let mut f = Fixture::launched(Config::default()).await;
    f.buy(SOL).await;
    f.end_stage().await;
    let pool = f.curve().await.stage_reward_pools[0];
    let other = regular_leaf(f.outsider.pubkey(), f.mint.pubkey(), 1, 20);
    let leaf = regular_leaf(f.trader.pubkey(), f.mint.pubkey(), 1, pool);
    let (a, b) = if leaf < other {
        (leaf, other)
    } else {
        (other, leaf)
    };
    let root = Pubkey::new_from_array(solana_sdk::keccak::hashv(&[&a, &b]).to_bytes());
    let i = f.root_ix(f.admin.pubkey(), 1, root);
    send(&mut f.ctx, &[i], &[&f.admin]).await.unwrap();
    for (amount, proof) in [
        (pool + 1, vec![other]),
        (pool, vec![]),
        (pool, vec![[0; 32]]),
    ] {
        let i = f.claim_ix(1, amount, proof, None, None);
        custom_error(
            send(&mut f.ctx, &[i], &[&f.trader]).await,
            code(HoldNowCode::InvalidMerkleProof),
        );
        assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, pool);
    }
    let i = f.claim_ix(1, pool, vec![other], None, None);
    send(&mut f.ctx, &[i], &[&f.trader]).await.unwrap();
    let claim_state = Pubkey::find_program_address(
        &[
            contract::USER_CLAIM_STATE_SEED,
            f.mint.pubkey().as_ref(),
            f.trader.pubkey().as_ref(),
        ],
        &contract::ID,
    )
    .0;
    let c: UserClaimState = state(&mut f.ctx, claim_state).await;
    assert_eq!(c.total_claimed_regular, pool);
}

#[tokio::test]
async fn cumulative_claims_pay_only_the_increment_and_preserve_earlier_entitlements() {
    let mut f = Fixture::launched(Config::default()).await;
    f.buy(SOL).await;
    f.end_stage().await;
    let first = f.curve().await.stage_reward_pools[0];
    let i = f.root_ix(
        f.admin.pubkey(),
        1,
        Pubkey::new_from_array(regular_leaf(f.trader.pubkey(), f.mint.pubkey(), 1, first)),
    );
    send(&mut f.ctx, &[i], &[&f.admin]).await.unwrap();
    let i = f.claim_ix(1, first, vec![], None, None);
    send(&mut f.ctx, &[i], &[&f.trader]).await.unwrap();
    f.next_stage().await;
    f.buy(SOL).await;
    f.end_stage().await;
    let second = f.curve().await.stage_reward_pools[1];
    let i = f.root_ix(
        f.admin.pubkey(),
        2,
        Pubkey::new_from_array(regular_leaf(
            f.trader.pubkey(),
            f.mint.pubkey(),
            2,
            first + second,
        )),
    );
    send(&mut f.ctx, &[i], &[&f.admin]).await.unwrap();
    let before = tokens(&mut f.ctx, f.trader_tokens).await;
    let i = f.claim_ix(2, first + second, vec![], None, None);
    send(&mut f.ctx, &[i], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, before + second);
    let i = f.claim_ix(1, first, vec![], None, None);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::NothingToClaim),
    );
}

#[tokio::test]
async fn initial_vesting_and_regular_rewards_can_be_claimed_together() {
    let initial = 900_000_000;
    let mut f = Fixture::initialized(Config {
        airdrop: initial,
        ..Config::default()
    })
    .await;
    f.config.root =
        Pubkey::new_from_array(initial_leaf(f.trader.pubkey(), f.mint.pubkey(), initial));
    let i = f.create_ix();
    send(&mut f.ctx, &[i], &[&f.creator]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, initial);
    f.buy(SOL).await;
    f.end_stage().await;
    let regular = f.curve().await.stage_reward_pools[0];
    let i = f.root_ix(
        f.admin.pubkey(),
        1,
        Pubkey::new_from_array(regular_leaf(f.trader.pubkey(), f.mint.pubkey(), 1, regular)),
    );
    send(&mut f.ctx, &[i], &[&f.admin]).await.unwrap();
    let before = tokens(&mut f.ctx, f.trader_tokens).await;
    let i = f.claim_ix(1, regular, vec![], Some(initial + 1), Some(vec![]));
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::InvalidMerkleProof),
    );
    let i = f.claim_ix(1, regular, vec![], Some(initial), Some(vec![]));
    send(&mut f.ctx, &[i], &[&f.trader]).await.unwrap();
    assert_eq!(
        tokens(&mut f.ctx, f.trader_tokens).await,
        before + regular + initial / 3
    );
    assert_eq!(
        tokens(&mut f.ctx, f.reward_tokens).await,
        initial - initial / 3
    );
    let i = f.initial_ix(
        f.creator.pubkey(),
        f.admin.pubkey(),
        Pubkey::new_unique(),
        3,
    );
    custom_error(
        send(&mut f.ctx, &[i], &[&f.creator, &f.admin]).await,
        code(HoldNowCode::InitialAllocationsAlreadySet),
    );
}

#[tokio::test]
async fn regression_final_stage_root_is_accepted_including_single_stage_tokens() {
    for stages in [1, 3] {
        let mut f = Fixture::launched(Config {
            stages,
            ..Config::default()
        })
        .await;
        for _ in 1..stages {
            f.end_stage().await;
            f.next_stage().await;
        }
        f.end_stage().await;
        let i = f.root_ix(f.admin.pubkey(), stages as u8, Pubkey::new_unique());
        send(&mut f.ctx, &[i], &[&f.admin]).await.unwrap();
        assert!(f.curve().await.stage_completion_times[stages as usize] > 0);
    }
}
#[tokio::test]
async fn regression_buy_reserves_match_vault_exactly() {
    let mut f = Fixture::launched(Config::default()).await;
    f.buy(SOL).await;
    let c = f.curve().await;
    let actual = tokens(&mut f.ctx, f.curve_tokens).await;
    assert_eq!(c.real_token_reserves, actual);
    assert_eq!(c.virtual_token_reserves, actual);
}
#[tokio::test]
async fn regression_retained_mint_and_freeze_authorities_are_rejected() {
    for freeze in [false, true] {
        let mut f = Fixture::initialized(Config::default()).await;
        let mut a = f
            .ctx
            .banks_client
            .get_account(f.mint.pubkey())
            .await
            .unwrap()
            .unwrap();
        let mut mint = spl_token::state::Mint::unpack(&a.data).unwrap();
        if freeze {
            mint.freeze_authority = COption::Some(f.creator.pubkey());
        } else {
            mint.mint_authority = COption::Some(f.creator.pubkey());
        }
        spl_token::state::Mint::pack(mint, &mut a.data).unwrap();
        f.ctx
            .set_account(&f.mint.pubkey(), &AccountSharedData::from(a));
        let i = f.create_ix();
        custom_error(
            send(&mut f.ctx, &[i], &[&f.creator]).await,
            code(if freeze {
                HoldNowCode::FreezeAuthorityEnabled
            } else {
                HoldNowCode::MitAuthorityEnabled
            }),
        );
    }
    let mut f = Fixture::launched(Config::default()).await;
    f.buy(10 * SOL).await;
    let i = spl_token::instruction::mint_to(
        &spl_token::id(),
        &f.mint.pubkey(),
        &f.creator_tokens,
        &f.creator.pubkey(),
        &[],
        SUPPLY,
    )
    .unwrap();
    assert!(send(&mut f.ctx, &[i], &[&f.creator]).await.is_err());
}
#[tokio::test]
async fn regression_creator_controlled_deposit_destination_is_rejected() {
    let mut f = Fixture::initialized(Config::default()).await;
    let mut i = f.create_ix();
    i.accounts[4].pubkey = f.creator_tokens;
    assert!(send(&mut f.ctx, &[i], &[&f.creator]).await.is_err());
    assert_eq!(tokens(&mut f.ctx, f.creator_tokens).await, SUPPLY);
}
#[tokio::test]
async fn regression_unfunded_initial_allocation_is_rejected() {
    let mut f = Fixture::initialized(Config {
        airdrop: 1_000_000_000,
        ..Config::default()
    })
    .await;
    let i = f.create_ix();
    custom_error(
        send(&mut f.ctx, &[i], &[&f.creator]).await,
        code(HoldNowCode::InvalidAirdropAmount),
    );
}
#[tokio::test]
async fn regression_root_stage_zero_future_stage_and_root_rewrite_are_rejected() {
    let mut f = Fixture::launched(Config::default()).await;
    let i = f.root_ix(f.admin.pubkey(), 0, Pubkey::new_unique());
    custom_error(
        send(&mut f.ctx, &[i], &[&f.admin]).await,
        code(HoldNowCode::InvalidStage),
    );
    let i = f.root_ix(f.admin.pubkey(), 2, Pubkey::new_unique());
    custom_error(
        send(&mut f.ctx, &[i], &[&f.admin]).await,
        code(HoldNowCode::StageNotComplete),
    );
    f.end_stage().await;
    let i = f.root_ix(f.admin.pubkey(), 1, Pubkey::new_unique());
    send(&mut f.ctx, &[i], &[&f.admin]).await.unwrap();
    let i = f.root_ix(f.admin.pubkey(), 1, Pubkey::new_unique());
    custom_error(
        send(&mut f.ctx, &[i], &[&f.admin]).await,
        code(HoldNowCode::MerkleRootAlreadySet),
    );
}
#[tokio::test]
async fn regression_initial_allocation_requires_creator_before_launch() {
    let mut f = Fixture::initialized(Config::default()).await;
    let i = f.initial_ix(
        f.outsider.pubkey(),
        f.admin.pubkey(),
        Pubkey::new_unique(),
        1,
    );
    custom_error(
        send(&mut f.ctx, &[i], &[&f.outsider, &f.admin]).await,
        code(HoldNowCode::NotAuthorized),
    );
    let i = f.initial_ix(
        f.creator.pubkey(),
        f.admin.pubkey(),
        Pubkey::new_unique(),
        0,
    );
    custom_error(
        send(&mut f.ctx, &[i], &[&f.creator, &f.admin]).await,
        code(HoldNowCode::InvalidStageCount),
    );
}
#[tokio::test]
async fn regression_invalid_fee_configuration_is_rejected() {
    let mut f = Fixture::launched(Config::default()).await;
    for (fee, staging) in [(101, 1), (1, u64::MAX)] {
        let i = f.params_ix(f.admin.pubkey(), fee, staging, SUPPLY, 30 * SOL, SUPPLY);
        custom_error(
            send(&mut f.ctx, &[i], &[&f.admin]).await,
            code(HoldNowCode::InvalidFeeBasisPoints),
        );
    }
}
#[tokio::test]
async fn regression_invalid_tax_duration_destination_and_stage_count_are_rejected() {
    for config in [
        Config {
            tax_min: 90,
            tax_max: 10,
            ..Config::default()
        },
        Config {
            duration: 0,
            ..Config::default()
        },
        Config {
            destination: 2,
            ..Config::default()
        },
        Config {
            decay: 101,
            ..Config::default()
        },
        Config {
            tax_max: 100,
            ..Config::default()
        },
    ] {
        let mut f = Fixture::initialized(config).await;
        let i = f.create_ix();
        custom_error(
            send(&mut f.ctx, &[i], &[&f.creator]).await,
            code(HoldNowCode::InvalidConfiguration),
        );
    }
    let mut f = Fixture::initialized(Config::default()).await;
    f.config.stages = 0;
    let i = f.create_ix();
    custom_error(
        send(&mut f.ctx, &[i], &[&f.creator]).await,
        code(HoldNowCode::InvalidStageCount),
    );
}
#[tokio::test]
async fn regression_trading_stops_at_deadline_without_crank() {
    let mut f = Fixture::launched(Config::default()).await;
    f.buy(SOL).await;
    clock(&mut f.ctx, START + contract::STAGE_UNIT_SECONDS as i64).await;
    let i = f.buy_ix(SOL, 0);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::StageExpired),
    );
    let i = f.sell_ix(f.trader.pubkey(), f.trader_tokens, 1_000_000, 0);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::StageExpired),
    );
}
#[tokio::test]
async fn regression_out_of_range_claim_returns_invalid_stage() {
    let mut f = Fixture::launched(Config::default()).await;
    let i = f.claim_ix(255, 0, vec![], None, None);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::InvalidStage),
    );
}
#[tokio::test]
async fn regression_regular_rewards_cannot_spend_vesting_budget() {
    let initial = SUPPLY / 10;
    let mut f = Fixture::initialized(Config {
        airdrop: initial,
        ..Config::default()
    })
    .await;
    f.config.root =
        Pubkey::new_from_array(initial_leaf(f.trader.pubkey(), f.mint.pubkey(), initial));
    let i = f.create_ix();
    send(&mut f.ctx, &[i], &[&f.creator]).await.unwrap();
    f.buy(SOL).await;
    f.end_stage().await;
    let regular = f.curve().await.stage_reward_pools[0];
    let inflated = regular + 1;
    let i = f.root_ix(
        f.admin.pubkey(),
        1,
        Pubkey::new_from_array(regular_leaf(
            f.trader.pubkey(),
            f.mint.pubkey(),
            1,
            inflated,
        )),
    );
    send(&mut f.ctx, &[i], &[&f.admin]).await.unwrap();
    let before = tokens(&mut f.ctx, f.reward_tokens).await;
    let i = f.claim_ix(1, inflated, vec![], None, None);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::RewardBudgetExceeded),
    );
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, before);
    assert_eq!(f.curve().await.total_regular_claimed, 0);
}

#[path = "audit_cases/migration.rs"]
mod migration;

#[tokio::test]
async fn reserves_and_product_are_conserved_over_many_mixed_trades() {
    let mut f = Fixture::launched(Config {
        tax_max: 0,
        ..Config::default()
    })
    .await;
    let rent = lamports(&mut f.ctx, f.vault).await;
    let mut random = 17u64;
    for _ in 0..128 {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        let before = f.curve().await;
        let k = before.virtual_sol_reserves as u128 * before.virtual_token_reserves as u128;
        let balance = tokens(&mut f.ctx, f.trader_tokens).await;
        if random % 2 == 0 || balance < 1_000_000 {
            f.buy(1_000_000 + random % 100_000_000).await;
        } else {
            let i = f.sell_ix(f.trader.pubkey(), f.trader_tokens, balance / 3, 0);
            send(&mut f.ctx, &[i], &[&f.trader]).await.unwrap();
        }
        let c = f.curve().await;
        assert_eq!(
            c.real_token_reserves,
            tokens(&mut f.ctx, f.curve_tokens).await
        );
        assert_eq!(
            c.real_sol_reserves,
            lamports(&mut f.ctx, f.vault).await - rent
        );
        assert_eq!(
            c.stage_reward_pools[0],
            tokens(&mut f.ctx, f.reward_tokens).await
        );
        assert!(c.virtual_sol_reserves as u128 * c.virtual_token_reserves as u128 >= k);
        assert_eq!(
            tokens(&mut f.ctx, f.trader_tokens).await
                + tokens(&mut f.ctx, f.curve_tokens).await
                + tokens(&mut f.ctx, f.reward_tokens).await,
            SUPPLY
        );
    }
}
#[tokio::test]
async fn initial_vesting_can_finish_and_be_claimed_after_launch_completion() {
    let amount = 900_000_000;
    let mut f = Fixture::initialized(Config {
        stages: 1,
        airdrop: amount,
        ..Config::default()
    })
    .await;
    f.config.root =
        Pubkey::new_from_array(initial_leaf(f.trader.pubkey(), f.mint.pubkey(), amount));
    let i = f.create_ix();
    send(&mut f.ctx, &[i], &[&f.creator]).await.unwrap();
    f.end_stage().await;
    f.next_stage().await;
    let i = f.claim_ix(1, 0, vec![], Some(amount), Some(vec![]));
    send(&mut f.ctx, &[i], &[&f.trader]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.trader_tokens).await, amount);
    assert_eq!(f.curve().await.total_initial_claimed, amount);
}
#[tokio::test]
async fn initial_allocation_can_be_precommitted_by_creator_and_backend() {
    let amount = 900_000_000;
    let mut f = Fixture::initialized(Config {
        airdrop: amount,
        ..Config::default()
    })
    .await;
    f.config.root =
        Pubkey::new_from_array(initial_leaf(f.trader.pubkey(), f.mint.pubkey(), amount));
    let i = f.initial_ix(f.creator.pubkey(), f.admin.pubkey(), f.config.root, 3);
    send(&mut f.ctx, &[i], &[&f.creator, &f.admin])
        .await
        .unwrap();
    let i = f.create_ix();
    send(&mut f.ctx, &[i], &[&f.creator]).await.unwrap();
    assert_eq!(tokens(&mut f.ctx, f.reward_tokens).await, amount);
}
#[tokio::test]
async fn maximum_stage_configuration_fits_and_duplicate_launch_is_rejected() {
    let mut f = Fixture::launched(Config {
        stages: 100,
        ..Config::default()
    })
    .await;
    assert_eq!(f.curve().await.cumulative_merkle_roots.len(), 101);
    let i = f.create_ix();
    assert!(send(&mut f.ctx, &[i], &[&f.creator]).await.is_err());
}
#[tokio::test]
async fn full_tax_and_instant_decay_have_defined_nonnegative_outputs() {
    let mut f = Fixture::launched(Config {
        tax_max: 98,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    let amount = tokens(&mut f.ctx, f.trader_tokens).await / 2;
    let i = f.sell_ix(f.trader.pubkey(), f.trader_tokens, amount, 0);
    send(&mut f.ctx, &[i], &[&f.trader]).await.unwrap();
    clock(&mut f.ctx, START + contract::STAGE_UNIT_SECONDS as i64 - 1).await;
    let i = f.sell_ix(f.trader.pubkey(), f.trader_tokens, amount, 0);
    send(&mut f.ctx, &[i], &[&f.trader]).await.unwrap();
    let mut f = Fixture::launched(Config {
        decay: 0,
        ..Config::default()
    })
    .await;
    f.buy(SOL).await;
    let amount = tokens(&mut f.ctx, f.trader_tokens).await / 2;
    let i = f.sell_ix(f.trader.pubkey(), f.trader_tokens, amount, 0);
    send(&mut f.ctx, &[i], &[&f.trader]).await.unwrap();
}

#[tokio::test]
async fn legacy_state_is_rejected_until_explicit_reconciliation() {
    let mut f = Fixture::launched(Config::default()).await;
    let mut global: Global = state(&mut f.ctx, f.global).await;
    global.protocol_version = 0;
    f.ctx.set_account(
        &f.global,
        &AccountSharedData::from(state_account(&global, 256)),
    );
    let i = f.buy_ix(SOL, 0);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::UnsupportedVersion),
    );
    global.protocol_version = contract::PROTOCOL_VERSION;
    f.ctx.set_account(
        &f.global,
        &AccountSharedData::from(state_account(&global, 256)),
    );
    let mut c = f.curve().await;
    c.protocol_version = 0;
    f.set_curve(&c).await;
    let i = f.buy_ix(SOL, 0);
    custom_error(
        send(&mut f.ctx, &[i], &[&f.trader]).await,
        code(HoldNowCode::UnsupportedVersion),
    );
}

#[tokio::test]
async fn vested_claim_does_not_require_an_unrelated_regular_proof() {
    let amount = 900_000_000;
    let mut f = Fixture::initialized(Config {
        airdrop: amount,
        ..Config::default()
    })
    .await;
    f.config.root =
        Pubkey::new_from_array(initial_leaf(f.trader.pubkey(), f.mint.pubkey(), amount));
    let i = f.create_ix();
    send(&mut f.ctx, &[i], &[&f.creator]).await.unwrap();
    f.buy(SOL).await;
    f.end_stage().await;
    let regular = f.curve().await.stage_reward_pools[0];
    let i = f.root_ix(
        f.admin.pubkey(),
        1,
        Pubkey::new_from_array(regular_leaf(
            f.outsider.pubkey(),
            f.mint.pubkey(),
            1,
            regular,
        )),
    );
    send(&mut f.ctx, &[i], &[&f.admin]).await.unwrap();
    let before = tokens(&mut f.ctx, f.trader_tokens).await;
    let i = f.claim_ix(1, 0, vec![], Some(amount), Some(vec![]));
    send(&mut f.ctx, &[i], &[&f.trader]).await.unwrap();
    assert_eq!(
        tokens(&mut f.ctx, f.trader_tokens).await,
        before + amount / 3
    );
}
#[tokio::test]
async fn maximum_stage_full_vesting_and_regular_claim_fit_runtime_limits() {
    let amount = 900_000_000;
    let mut f = Fixture::initialized(Config {
        stages: 100,
        airdrop: amount,
        ..Config::default()
    })
    .await;
    f.config.root =
        Pubkey::new_from_array(initial_leaf(f.trader.pubkey(), f.mint.pubkey(), amount));
    let i = f.create_ix();
    send(&mut f.ctx, &[i], &[&f.creator]).await.unwrap();
    let mut c = f.curve().await;
    c.current_stage = 100;
    f.set_curve(&c).await;
    f.buy(SOL).await;
    f.end_stage().await;
    let regular = f.curve().await.stage_reward_pools[99];
    let i = f.root_ix(
        f.admin.pubkey(),
        100,
        Pubkey::new_from_array(regular_leaf(
            f.trader.pubkey(),
            f.mint.pubkey(),
            100,
            regular,
        )),
    );
    send(&mut f.ctx, &[i], &[&f.admin]).await.unwrap();
    let before = tokens(&mut f.ctx, f.trader_tokens).await;
    let i = f.claim_ix(100, regular, vec![], Some(amount), Some(vec![]));
    send(&mut f.ctx, &[i], &[&f.trader]).await.unwrap();
    assert_eq!(
        tokens(&mut f.ctx, f.trader_tokens).await,
        before + regular + amount
    );
}

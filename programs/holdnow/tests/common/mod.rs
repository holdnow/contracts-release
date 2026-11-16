use anchor_lang::solana_program::system_program;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_system_interface::instruction as system_instruction;
// Local Solana bank fixtures. No RPC endpoint, external wallet, or network transaction is used.
pub use ::holdnow as contract;
use anchor_lang::{AccountDeserialize, AccountSerialize, InstructionData, ToAccountMetas};
use contract::{constants::*, BondingCurve, Global};
use solana_program_test::{processor, BanksClientError, ProgramTest, ProgramTestContext};
use solana_sdk::{
    account::{Account, AccountSharedData},
    account_info::AccountInfo,
    clock::Clock,
    entrypoint::ProgramResult,
    instruction::Instruction,
    instruction::InstructionError,
    program::invoke,
    program_error::ProgramError,
    program_option::COption,
    program_pack::Pack,
    pubkey::Pubkey,
    rent::Rent,
    signature::{Keypair, Signer},
    sysvar,
    transaction::{Transaction, TransactionError},
};
use spl_token::state::{Account as TokenAccount, Mint};
use std::sync::atomic::{AtomicU32, Ordering};

pub const SUPPLY: u64 = 1_000_000_000_000_000;
pub const SOL: u64 = 1_000_000_000;
pub const START: i64 = 1_700_000_000;
pub const NATIVE_PANIC: u32 = 0xface;
pub const MOCK_AMM: Pubkey = Pubkey::new_from_array([77; 32]);
static NONCE: AtomicU32 = AtomicU32::new(0);

// Deliberately untrusted migration target. It exercises the *real* token CPI using
// the signer privilege the Holdnow program passes to a caller-selected AMM.
fn attacker_amm(_id: &Pubkey, a: &[AccountInfo], _data: &[u8]) -> ProgramResult {
    let amount = TokenAccount::unpack(&a[18].try_borrow_data()?)?.amount;
    if amount > 0 {
        invoke(
            &spl_token::instruction::transfer(
                &spl_token::id(),
                a[18].key,
                a[10].key,
                a[17].key,
                &[],
                amount,
            )?,
            &[a[18].clone(), a[10].clone(), a[17].clone(), a[0].clone()],
        )?;
    }
    Ok(())
}

// ABI-compatible local stand-in: real SPL transfers and LP issuance. It does
// not model Raydium's market validation or internal AMM economics.
fn trusted_amm(_id: &Pubkey, a: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let pc = u64::from_le_bytes(data[10..18].try_into().unwrap());
    let coin = u64::from_le_bytes(data[18..26].try_into().unwrap());
    for (from, to, amount) in [(18, 10, coin), (19, 11, pc)] {
        invoke(
            &spl_token::instruction::transfer(
                &spl_token::id(),
                a[from].key,
                a[to].key,
                a[17].key,
                &[],
                amount,
            )?,
            &[a[from].clone(), a[to].clone(), a[17].clone(), a[0].clone()],
        )?;
    }
    if a[13].try_borrow_data()?.first() == Some(&1) {
        return Ok(());
    }
    invoke(
        &spl_token::instruction::mint_to(
            &spl_token::id(),
            a[7].key,
            a[20].key,
            a[17].key,
            &[],
            1_000,
        )?,
        &[a[7].clone(), a[20].clone(), a[17].clone(), a[0].clone()],
    )?;
    Ok(())
}
fn market_stub(_id: &Pubkey, _a: &[AccountInfo], _d: &[u8]) -> ProgramResult {
    Ok(())
}

pub fn state_account<T: AccountSerialize>(value: &T, space: usize) -> Account {
    let mut data = vec![];
    value.try_serialize(&mut data).unwrap();
    data.resize(space.max(data.len()), 0);
    Account {
        lamports: Rent::default().minimum_balance(data.len()),
        data,
        owner: contract::ID,
        executable: false,
        rent_epoch: 0,
    }
}

pub fn ix<A: ToAccountMetas, D: InstructionData>(accounts: A, data: D) -> Instruction {
    Instruction {
        program_id: contract::ID,
        accounts: accounts.to_account_metas(None),
        data: data.data(),
    }
}

pub async fn send(
    ctx: &mut ProgramTestContext,
    instructions: &[Instruction],
    signers: &[&Keypair],
) -> Result<(), BanksClientError> {
    let mut all = vec![ComputeBudgetInstruction::set_compute_unit_limit(
        1_400_000 - NONCE.fetch_add(1, Ordering::Relaxed) % 100_000,
    )];
    all.extend_from_slice(instructions);
    let hash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let mut keys = vec![&ctx.payer];
    keys.extend_from_slice(signers);
    let tx = Transaction::new_signed_with_payer(&all, Some(&ctx.payer.pubkey()), &keys, hash);
    ctx.banks_client.process_transaction(tx).await
}

pub fn custom_error(result: Result<(), BanksClientError>, expected: u32) {
    assert_eq!(
        result.unwrap_err().unwrap(),
        TransactionError::InstructionError(1, InstructionError::Custom(expected))
    );
}

pub async fn state<T: AccountDeserialize>(ctx: &mut ProgramTestContext, key: Pubkey) -> T {
    let account = ctx.banks_client.get_account(key).await.unwrap().unwrap();
    T::try_deserialize(&mut account.data.as_slice()).unwrap()
}

pub async fn tokens(ctx: &mut ProgramTestContext, key: Pubkey) -> u64 {
    let account = ctx.banks_client.get_account(key).await.unwrap().unwrap();
    TokenAccount::unpack(&account.data).unwrap().amount
}

pub async fn lamports(ctx: &mut ProgramTestContext, key: Pubkey) -> u64 {
    ctx.banks_client
        .get_account(key)
        .await
        .unwrap()
        .map_or(0, |a| a.lamports)
}

pub async fn clock(ctx: &mut ProgramTestContext, timestamp: i64) {
    let mut c: Clock = ctx.banks_client.get_sysvar().await.unwrap();
    c.unix_timestamp = timestamp;
    ctx.set_sysvar(&c);
}

#[derive(Clone)]
pub struct Config {
    pub stages: u64,
    pub duration: u64,
    pub claim: u64,
    pub tax_min: u8,
    pub tax_max: u8,
    pub decay: u8,
    pub destination: u8,
    pub airdrop: u64,
    pub root: Pubkey,
    pub mint_authority: bool,
    pub freeze_authority: bool,
    /// Activate SIMD-0437 at an epoch boundary, matching public rent on 2026-11-07.
    pub current_cluster_rent: bool,
    pub minimum_migration_sol: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            stages: 3,
            duration: 1,
            claim: 60,
            tax_min: 0,
            tax_max: 20,
            decay: 100,
            destination: 3,
            airdrop: 0,
            root: Pubkey::default(),
            mint_authority: false,
            freeze_authority: false,
            current_cluster_rent: false,
            minimum_migration_sol: contract::MIN_MIGRATION_SOL,
        }
    }
}

pub struct Fixture {
    pub ctx: ProgramTestContext,
    pub admin: Keypair,
    pub creator: Keypair,
    pub trader: Keypair,
    pub outsider: Keypair,
    pub mint: Keypair,
    pub global: Pubkey,
    pub fee: Pubkey,
    pub curve: Pubkey,
    pub vault: Pubkey,
    pub curve_tokens: Pubkey,
    pub reward_authority: Pubkey,
    pub reward_tokens: Pubkey,
    pub creator_tokens: Pubkey,
    pub trader_tokens: Pubkey,
    pub config: Config,
}

impl Fixture {
    pub async fn minted(config: Config) -> Self {
        Self::minted_with_vault_prefunding(config, true).await
    }
    /// Test the public contract path independently of the frontend's extra SOL transfer.
    pub async fn minted_with_vault_prefunding(config: Config, prefund: bool) -> Self {
        let admin = Keypair::new();
        let creator = Keypair::new();
        let trader = Keypair::new();
        let outsider = Keypair::new();
        let mint = Keypair::new();
        let fee = Pubkey::new_unique();
        let global = Pubkey::find_program_address(&[GLOBAL_STATE_SEED], &contract::ID).0;
        let curve =
            Pubkey::find_program_address(&[BONDING_CURVE, mint.pubkey().as_ref()], &contract::ID).0;
        let vault =
            Pubkey::find_program_address(&[SOL_VAULT_SEED, mint.pubkey().as_ref()], &contract::ID)
                .0;
        let curve_tokens =
            Pubkey::find_program_address(&[VAULT_SEED, mint.pubkey().as_ref()], &contract::ID).0;
        let reward_authority = Pubkey::find_program_address(
            &[REWARD_STATE_SEED, mint.pubkey().as_ref()],
            &contract::ID,
        )
        .0;
        let reward_tokens = Pubkey::find_program_address(
            &[REWARD_VAULT_SEED, mint.pubkey().as_ref()],
            &contract::ID,
        )
        .0;
        let creator_tokens = spl_associated_token_account::address::get_associated_token_address(
            &creator.pubkey(),
            &mint.pubkey(),
        );
        let trader_tokens = spl_associated_token_account::address::get_associated_token_address(
            &trader.pubkey(),
            &mint.pubkey(),
        );
        let g = Global {
            initialized: true,
            authority: admin.pubkey(),
            fee_recipient: fee,
            owner_wallet: admin.pubkey(),
            initial_virtual_token_reserves: SUPPLY,
            initial_virtual_sol_reserves: 30 * SOL,
            initial_real_token_reserves: SUPPLY,
            token_total_supply: SUPPLY,
            fee_basis_points: 1,
            create_fee: 6_900_000,
            staging_pool_fee: 1,
            backend_wallet: Pubkey::default(),
            protocol_version: contract::PROTOCOL_VERSION,
            stage_unit_seconds: contract::STAGE_UNIT_SECONDS,
            migration_grace_seconds: contract::MIGRATION_GRACE_SECONDS,
            minimum_migration_sol: config
                .minimum_migration_sol
                .max(contract::NEW_LAUNCH_MIN_MIGRATION_SOL),
        };
        assert!(
            std::env::var_os("HOLDNOW_TEST_SBF").is_some(),
            "Run bash scripts/test.sh: transaction tests execute the compiled SBPF program"
        );
        let mut pt = ProgramTest::default();
        if config.current_cluster_rent {
            // Agave 4.3.2 feature ID. Activate the real runtime feature so the bank's
            // rent collector AND Rent sysvar change together; set_sysvar alone is invalid.
            let rent_feature: Pubkey = "61BtM7BkDEE8Yq5fskEVAQT9mYA8qCejJWoLe5apqg81"
                .parse()
                .unwrap();
            pt.deactivate_feature(rent_feature);
            // Bincode Feature { activated_at: None }, padded to Feature::size_of().
            pt.add_account(
                rent_feature,
                Account {
                    lamports: Rent::default().minimum_balance(9),
                    data: vec![0; 9],
                    owner: "Feature111111111111111111111111111111111111"
                        .parse()
                        .unwrap(),
                    executable: false,
                    rent_epoch: 0,
                },
            );
        }
        pt.prefer_bpf(true);
        pt.add_program("holdnow", contract::ID, None);
        pt.prefer_bpf(false);
        // Agave supplies the real SPL Token and associated-token SBF programs.
        pt.add_program("attacker_amm", MOCK_AMM, processor!(attacker_amm));
        if let Some(directory) = std::env::var_os("HOLDNOW_EXTERNAL_PROGRAMS") {
            let cluster = if cfg!(feature = "devnet") {
                "devnet"
            } else {
                "mainnet-beta"
            };
            let dir = std::path::PathBuf::from(directory);
            pt.prefer_bpf(true);
            for (name, id) in [
                ("raydium", contract::AMM_PROGRAM),
                ("openbook", contract::MARKET_PROGRAM),
            ] {
                let path = dir.join(format!("{cluster}-{name}"));
                pt.add_program(
                    Box::leak(path.to_string_lossy().into_owned().into_boxed_str()),
                    id,
                    None,
                );
            }
            pt.prefer_bpf(false);
        } else {
            pt.add_program(
                "trusted_amm",
                contract::AMM_PROGRAM,
                processor!(trusted_amm),
            );
            pt.add_program(
                "market_stub",
                contract::MARKET_PROGRAM,
                processor!(market_stub),
            );
        }
        pt.add_account(global, state_account(&g, 256));
        for key in [
            admin.pubkey(),
            creator.pubkey(),
            trader.pubkey(),
            outsider.pubkey(),
            fee,
        ] {
            pt.add_account(key, Account::new(1_000 * SOL, 0, &system_program::id()));
        }
        let mut ctx = pt.start_with_context().await;
        if config.current_cluster_rent {
            ctx.warp_to_epoch(1).unwrap();
            let rent: Rent = ctx.banks_client.get_sysvar().await.unwrap();
            assert_eq!(rent.minimum_balance(0), 650_240);
            assert_eq!(rent.minimum_balance(3228), 17_048_480);
        }
        clock(&mut ctx, START).await;
        let creator_key = creator.pubkey();
        let mut setup = vec![
            system_instruction::create_account(
                &creator.pubkey(),
                &mint.pubkey(),
                Rent::default().minimum_balance(Mint::LEN),
                Mint::LEN as u64,
                &spl_token::id(),
            ),
            spl_token::instruction::initialize_mint2(
                &spl_token::id(),
                &mint.pubkey(),
                &creator.pubkey(),
                if config.freeze_authority {
                    Some(&creator_key)
                } else {
                    None
                },
                6,
            )
            .unwrap(),
            spl_associated_token_account::instruction::create_associated_token_account(
                &creator.pubkey(),
                &creator.pubkey(),
                &mint.pubkey(),
                &spl_token::id(),
            ),
            spl_associated_token_account::instruction::create_associated_token_account(
                &creator.pubkey(),
                &trader.pubkey(),
                &mint.pubkey(),
                &spl_token::id(),
            ),
            spl_token::instruction::mint_to(
                &spl_token::id(),
                &mint.pubkey(),
                &creator_tokens,
                &creator.pubkey(),
                &[],
                SUPPLY,
            )
            .unwrap(),
        ];
        if !config.mint_authority {
            setup.push(
                spl_token::instruction::set_authority(
                    &spl_token::id(),
                    &mint.pubkey(),
                    None,
                    spl_token::instruction::AuthorityType::MintTokens,
                    &creator.pubkey(),
                    &[],
                )
                .unwrap(),
            );
        }
        if prefund {
            setup.push(system_instruction::transfer(
                &creator.pubkey(),
                &vault,
                Rent::default().minimum_balance(0),
            ));
        }
        send(&mut ctx, &setup, &[&creator, &mint]).await.unwrap();
        Self {
            ctx,
            admin,
            creator,
            trader,
            outsider,
            mint,
            global,
            fee,
            curve,
            vault,
            curve_tokens,
            reward_authority,
            reward_tokens,
            creator_tokens,
            trader_tokens,
            config,
        }
    }
    pub async fn initialized(config: Config) -> Self {
        let mut f = Self::minted(config).await;
        let init = f.initialize_curve_ix(f.creator.pubkey(), f.config.stages);
        send(&mut f.ctx, &[init], &[&f.creator]).await.unwrap();
        f
    }
    pub async fn launched(config: Config) -> Self {
        let mut f = Self::initialized(config).await;
        let create = f.create_ix();
        send(&mut f.ctx, &[create], &[&f.creator]).await.unwrap();
        f.replay_legacy_minimum().await;
        f
    }
    /// Existing regression scenarios replay launches made under the former 1 SOL
    /// floor. New-policy scenarios pass the profile floor explicitly and never
    /// rewrite their state. Account layout and every other launch field are unchanged.
    pub async fn replay_legacy_minimum(&mut self) {
        if self.config.minimum_migration_sol < contract::NEW_LAUNCH_MIN_MIGRATION_SOL {
            let mut c = self.curve().await;
            c.minimum_migration_sol = self.config.minimum_migration_sol;
            let mut account = self
                .ctx
                .banks_client
                .get_account(self.curve)
                .await
                .unwrap()
                .unwrap();
            c.try_serialize(&mut account.data.as_mut_slice()).unwrap();
            self.ctx
                .set_account(&self.curve, &AccountSharedData::from(account));
        }
    }
    pub fn initialize_curve_ix(&self, user: Pubkey, stages: u64) -> Instruction {
        ix(
            contract::accounts::InitializeCurve {
                creator_token_account: self.creator_tokens,
                user,
                mint: self.mint.pubkey(),
                bonding_curve: self.curve,
                vault: self.vault,
                associated_bonding_curve: self.curve_tokens,
                global: self.global,
                system_program: system_program::id(),
                token_program: spl_token::id(),
                rent: sysvar::rent::id(),
            },
            contract::instruction::InitializeCurve {
                number_stages: stages,
            },
        )
    }
    pub fn create_ix(&self) -> Instruction {
        let c = &self.config;
        ix(
            contract::accounts::Create {
                user: self.creator.pubkey(),
                global: self.global,
                mint: self.mint.pubkey(),
                bonding_curve: self.curve,
                associated_bonding_curve: self.curve_tokens,
                associated_user_account: self.creator_tokens,
                fee_recipient: self.fee,
                reward_recipient: self.reward_authority,
                associated_reward_recipient: self.reward_tokens,
                system_program: system_program::id(),
                token_program: spl_token::id(),
            },
            contract::instruction::Create {
                number_stage: c.stages,
                stage_duration: c.duration,
                claim_duration: c.claim,
                sell_tax_min: c.tax_min,
                sell_tax_max: c.tax_max,
                sell_tax_decay: c.decay,
                token_pool_destination: c.destination,
                amount: SUPPLY,
                airdrop_amount: c.airdrop,
                merkle_root: c.root,
                minimum_migration_sol: c
                    .minimum_migration_sol
                    .max(contract::NEW_LAUNCH_MIN_MIGRATION_SOL),
                migration_grace_seconds: contract::MIGRATION_GRACE_SECONDS,
            },
        )
    }
    pub fn buy_ix(&self, amount: u64, minimum: u64) -> Instruction {
        ix(
            contract::accounts::Buy {
                global: self.global,
                fee_recipient: self.fee,
                reward_recipient: self.reward_authority,
                associated_reward_recipient: self.reward_tokens,
                mint: self.mint.pubkey(),
                vault: self.vault,
                bonding_curve: self.curve,
                associated_bonding_curve: self.curve_tokens,
                associated_user: self.trader_tokens,
                user: self.trader.pubkey(),
                system_program: system_program::id(),
                token_program: spl_token::id(),
                clock: sysvar::clock::id(),
            },
            contract::instruction::Buy {
                sol_amount: amount,
                min_tokens: minimum,
            },
        )
    }
    pub fn sell_ix(&self, user: Pubkey, account: Pubkey, amount: u64, minimum: u64) -> Instruction {
        ix(
            contract::accounts::Sell {
                user_sale_state: Pubkey::find_program_address(
                    &[
                        contract::USER_SALE_STATE_SEED,
                        self.mint.pubkey().as_ref(),
                        user.as_ref(),
                    ],
                    &contract::ID,
                )
                .0,
                global: self.global,
                fee_recipient: self.fee,
                reward_recipient: self.reward_authority,
                associated_reward_recipient: self.reward_tokens,
                mint: self.mint.pubkey(),
                vault: self.vault,
                bonding_curve: self.curve,
                associated_bonding_curve: self.curve_tokens,
                associated_user: account,
                user,
                system_program: system_program::id(),
                token_program: spl_token::id(),
                clock: sysvar::clock::id(),
            },
            contract::instruction::Sell {
                token_amount: amount,
                min_sol_amount: minimum,
            },
        )
    }
    pub fn stage_ix(&self) -> Instruction {
        ix(
            contract::accounts::StageCompleted {
                mint: self.mint.pubkey(),
                bonding_curve: self.curve,
                user: self.outsider.pubkey(),
                system_program: system_program::id(),
            },
            contract::instruction::StageCompleted {},
        )
    }
    pub fn root_ix(&self, owner: Pubkey, stage: u8, root: Pubkey) -> Instruction {
        ix(
            contract::accounts::UpdateMerkleRoot {
                reward_recipient: self.reward_authority,
                reward_vault: self.reward_tokens,
                token_program: spl_token::id(),
                bonding_curve: self.curve,
                mint: self.mint.pubkey(),
                global: self.global,
                owner_wallet: owner,
                clock: sysvar::clock::id(),
            },
            contract::instruction::UpdateMerkleRoot {
                stage,
                merkle_root: root,
                total_eligible_supply: SUPPLY,
            },
        )
    }
    pub fn initial_ix(
        &self,
        creator: Pubkey,
        backend: Pubkey,
        root: Pubkey,
        stages: u8,
    ) -> Instruction {
        ix(
            contract::accounts::SetInitialAllocations {
                global: self.global,
                bonding_curve: self.curve,
                mint: self.mint.pubkey(),
                creator,
                backend_wallet: backend,
                clock: sysvar::clock::id(),
            },
            contract::instruction::SetInitialAllocations {
                merkle_root: root,
                total_stages: stages,
            },
        )
    }
    pub fn claim_ix(
        &self,
        stage: u8,
        amount: u64,
        proof: Vec<[u8; 32]>,
        initial: Option<u64>,
        initial_proof: Option<Vec<[u8; 32]>>,
    ) -> Instruction {
        let user = self.trader.pubkey();
        let mint = self.mint.pubkey();
        ix(
            contract::accounts::ClaimRewards {
                mint,
                bonding_curve: self.curve,
                user_claim_state: Pubkey::find_program_address(
                    &[USER_CLAIM_STATE_SEED, mint.as_ref(), user.as_ref()],
                    &contract::ID,
                )
                .0,
                user_vesting_state: Pubkey::find_program_address(
                    &[USER_VESTING_STATE_SEED, mint.as_ref(), user.as_ref()],
                    &contract::ID,
                )
                .0,
                reward_recipient: self.reward_authority,
                reward_vault: self.reward_tokens,
                user_token_account: self.trader_tokens,
                user,
                system_program: system_program::id(),
                token_program: spl_token::id(),
                clock: sysvar::clock::id(),
                proof_buffer: None,
            },
            contract::instruction::ClaimRewards {
                stage,
                cumulative_regular_allocation: amount,
                regular_proof: proof,
                initial_allocation_amount: initial,
                initial_proof,
            },
        )
    }
    pub fn params_ix(
        &self,
        user: Pubkey,
        fee: u64,
        staging: u64,
        tokens: u64,
        sol: u64,
        supply: u64,
    ) -> Instruction {
        ix(
            contract::accounts::SetParams {
                global: self.global,
                user,
            },
            contract::instruction::SetParams {
                fee_recipient: self.fee,
                owner_wallet: self.admin.pubkey(),
                initial_virtual_token_reserves: tokens,
                initial_virtual_sol_reserves: sol,
                initial_real_token_reserves: SUPPLY,
                token_total_supply: supply,
                fee_basis_points: fee,
                create_fee: 6_900_000,
                staging_pool_fee: staging,
                minimum_migration_sol: contract::NEW_LAUNCH_MIN_MIGRATION_SOL,
            },
        )
    }
    pub fn activate_redemption_ix(&self) -> Instruction {
        ix(
            contract::accounts::ActivateRedemption {
                mint: self.mint.pubkey(),
                bonding_curve: self.curve,
                vault: self.vault,
                curve_tokens: self.curve_tokens,
                amm_config: Pubkey::find_program_address(
                    &[b"amm_config_account_seed"],
                    &contract::AMM_PROGRAM,
                )
                .0,
                user: self.trader.pubkey(),
            },
            contract::instruction::ActivateRedemption {},
        )
    }
    pub fn redeem_ix(&self, amount: u64, minimum: u64) -> Instruction {
        ix(
            contract::accounts::Redeem {
                mint: self.mint.pubkey(),
                bonding_curve: self.curve,
                vault: self.vault,
                curve_tokens: self.curve_tokens,
                user_tokens: self.trader_tokens,
                user: self.trader.pubkey(),
                token_program: spl_token::id(),
                system_program: system_program::id(),
            },
            contract::instruction::Redeem {
                token_amount: amount,
                min_sol: minimum,
            },
        )
    }
    pub async fn curve(&mut self) -> BondingCurve {
        state(&mut self.ctx, self.curve).await
    }
    pub async fn set_curve(&mut self, c: &BondingCurve) {
        let old = self
            .ctx
            .banks_client
            .get_account(self.curve)
            .await
            .unwrap()
            .unwrap();
        self.ctx.set_account(
            &self.curve,
            &AccountSharedData::from(state_account(c, old.data.len())),
        );
    }
    pub async fn buy(&mut self, amount: u64) {
        let i = self.buy_ix(amount, 0);
        send(&mut self.ctx, &[i], &[&self.trader]).await.unwrap();
    }
    pub async fn end_stage(&mut self) {
        let c = self.curve().await;
        clock(&mut self.ctx, c.at_start_stage + c.stage_duration).await;
        let i = self.stage_ix();
        send(&mut self.ctx, &[i], &[&self.outsider]).await.unwrap();
    }
    pub async fn settle_stage(&mut self) {
        let c = self.curve().await;
        let stage = c.current_stage as u8;
        if !c.stage_settled[stage as usize] {
            // Standard fixtures publish their sole trader's cumulative earned allocation.
            // Tests of missing/empty settlement use stage_ix directly instead.
            let total: u64 = c.stage_reward_pools[..stage as usize].iter().sum();
            let root = Pubkey::new_from_array(regular_leaf(
                self.trader.pubkey(),
                self.mint.pubkey(),
                stage,
                total,
            ));
            let i = self.root_ix(self.admin.pubkey(), stage, root);
            send(&mut self.ctx, &[i], &[&self.admin]).await.unwrap();
        }
    }
    pub async fn next_stage(&mut self) {
        self.settle_stage().await;
        let c = self.curve().await;
        clock(&mut self.ctx, c.at_start_stage + c.claim_duration).await;
        let i = self.stage_ix();
        send(&mut self.ctx, &[i], &[&self.outsider]).await.unwrap();
    }
}

pub fn regular_leaf(user: Pubkey, mint: Pubkey, stage: u8, amount: u64) -> [u8; 32] {
    solana_sdk::keccak::hashv(&[
        user.as_ref(),
        mint.as_ref(),
        &[stage],
        &amount.to_le_bytes(),
        b"CUMULATIVE",
    ])
    .to_bytes()
}
pub fn initial_leaf(user: Pubkey, mint: Pubkey, amount: u64) -> [u8; 32] {
    solana_sdk::keccak::hashv(&[
        user.as_ref(),
        mint.as_ref(),
        &amount.to_le_bytes(),
        b"INITIAL",
    ])
    .to_bytes()
}

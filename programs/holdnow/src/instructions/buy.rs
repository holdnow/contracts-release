use anchor_lang::prelude::*;

use anchor_spl::token::{transfer, Mint, Token, TokenAccount, Transfer};

use crate::{
    constants::{
        BONDING_CURVE, GLOBAL_STATE_SEED, REWARD_STATE_SEED, REWARD_VAULT_SEED, SOL_VAULT_SEED,
        VAULT_SEED,
    },
    error::*,
    events::*,
    state::{BondingCurve, Global},
};
use solana_program::program::invoke;
use solana_system_interface::instruction as system_instruction;

#[derive(Accounts)]
pub struct Buy<'info> {
    #[account(
        seeds = [GLOBAL_STATE_SEED],
        bump
    )]
    pub global: Box<Account<'info, Global>>,
    /// CHECK:` doc comment explaining why no checks through types are necessary.
    #[account(mut)]
    pub fee_recipient: AccountInfo<'info>, // wallet address to receive the fee as SOL

    #[account(
        mut,
        seeds = [REWARD_STATE_SEED, mint.key().as_ref()],
        bump
    )]
    /// CHECK:` doc comment explaining why no checks through types are necessary.
    pub reward_recipient: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [REWARD_VAULT_SEED, mint.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = reward_recipient,
    )]
    pub associated_reward_recipient: Box<Account<'info, TokenAccount>>,

    #[account(mut)]
    pub mint: Box<Account<'info, Mint>>, // the mint address of token

    #[account(
        mut,
        seeds = [SOL_VAULT_SEED, mint.key().as_ref()],
        bump
    )]
    /// CHECK: this should be set by admin
    pub vault: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [BONDING_CURVE, mint.key().as_ref()],
        bump
    )]
    pub bonding_curve: Box<Account<'info, BondingCurve>>,

    #[account(
        mut,
        seeds = [VAULT_SEED, mint.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = vault,
    )]
    pub associated_bonding_curve: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = user
    )]
    pub associated_user: Box<Account<'info, TokenAccount>>,

    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub clock: Sysvar<'info, Clock>,
}

pub fn buy(ctx: Context<Buy>, sol_amount: u64, min_tokens: u64) -> Result<()> {
    let a = ctx.accounts;
    a.global.assert_current()?;
    a.bonding_curve.assert_current()?;
    let stage = a.bonding_curve.assert_trading(a.clock.unix_timestamp)?;
    require!(sol_amount > 0, HoldNowCode::ZeroAmount);
    require_keys_eq!(
        a.fee_recipient.key(),
        a.global.fee_recipient,
        HoldNowCode::InValidFeeRecipient
    );
    require!(
        a.global.fee_basis_points <= crate::MAX_FEE_PERCENT
            && a.global.staging_pool_fee <= crate::MAX_FEE_PERCENT,
        HoldNowCode::InvalidFeeBasisPoints
    );
    let fee = crate::utils::percentage(sol_amount, a.global.fee_basis_points)?;
    let net_sol = sol_amount
        .checked_sub(fee)
        .ok_or(HoldNowCode::MathOverflow)?;
    let gross = calculate_tokens_from_sol(&a.bonding_curve, net_sol)?;
    let reward = crate::utils::percentage(gross, a.global.staging_pool_fee)?;
    let user_tokens = gross.checked_sub(reward).ok_or(HoldNowCode::MathOverflow)?;
    require!(user_tokens > 0, HoldNowCode::ZeroAmount);
    require!(user_tokens >= min_tokens, HoldNowCode::InsufficientOutput);
    require!(
        gross <= a.bonding_curve.real_token_reserves && gross <= a.associated_bonding_curve.amount,
        HoldNowCode::InsufficientFunds
    );
    for (to, amount) in [
        (a.vault.to_account_info(), net_sol),
        (a.fee_recipient.to_account_info(), fee),
    ] {
        invoke(
            &system_instruction::transfer(&a.user.key(), to.key, amount),
            &[
                a.user.to_account_info(),
                to,
                a.system_program.to_account_info(),
            ],
        )?;
    }
    let mint = a.mint.key();
    let bump = Pubkey::find_program_address(&[SOL_VAULT_SEED, mint.as_ref()], ctx.program_id).1;
    let seeds: &[&[u8]] = &[SOL_VAULT_SEED, mint.as_ref(), &[bump]];
    for (to, amount) in [
        (a.associated_user.to_account_info(), user_tokens),
        (a.associated_reward_recipient.to_account_info(), reward),
    ] {
        transfer(
            CpiContext::new(
                a.token_program.key(),
                Transfer {
                    from: a.associated_bonding_curve.to_account_info(),
                    to,
                    authority: a.vault.to_account_info(),
                },
            )
            .with_signer(&[seeds]),
            amount,
        )?;
    }
    let c = &mut a.bonding_curve;
    c.stage_reward_pools[stage] = c.stage_reward_pools[stage]
        .checked_add(reward)
        .ok_or(HoldNowCode::MathOverflow)?;
    c.real_token_reserves = c
        .real_token_reserves
        .checked_sub(gross)
        .ok_or(HoldNowCode::MathOverflow)?;
    c.virtual_token_reserves = c
        .virtual_token_reserves
        .checked_sub(gross)
        .ok_or(HoldNowCode::MathOverflow)?;
    c.real_sol_reserves = c
        .real_sol_reserves
        .checked_add(net_sol)
        .ok_or(HoldNowCode::MathOverflow)?;
    c.virtual_sol_reserves = c
        .virtual_sol_reserves
        .checked_add(net_sol)
        .ok_or(HoldNowCode::MathOverflow)?;
    emit!(SwapEvent {
        stage: c.current_stage,
        mint,
        user: a.user.key(),
        bonding_curve: c.key(),
        reward_cap_addr: a.associated_reward_recipient.key(),
        sol_amount: net_sol,
        token_amount: user_tokens,
        reward_amount: reward,
        is_buy: true,
        token_supply: c.token_total_supply,
        timestamp: a.clock.unix_timestamp,
        reserve_lamport: c.virtual_sol_reserves,
        reserve_token: c.virtual_token_reserves,
        fee_amount: fee
    });
    Ok(())
}
fn calculate_tokens_from_sol(c: &Account<BondingCurve>, amount: u64) -> Result<u64> {
    crate::utils::curve_output(amount, c.virtual_sol_reserves, c.virtual_token_reserves)
}

#[cfg(test)]
mod audit_unit_tests {
    include!("../../tests/unit/buy.rs");
}

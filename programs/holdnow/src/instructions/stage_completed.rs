use crate::{constants::BONDING_CURVE, error::*, events::*, state::BondingCurve};
use anchor_lang::prelude::*;
use anchor_spl::token::Mint;
#[derive(Accounts)]
pub struct StageCompleted<'info> {
    pub mint: Box<Account<'info, Mint>>,
    #[account(mut,seeds=[BONDING_CURVE,mint.key().as_ref()],bump)]
    pub bonding_curve: Box<Account<'info, BondingCurve>>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}
pub fn stage_completed(ctx: Context<StageCompleted>) -> Result<()> {
    let a = ctx.accounts;
    let now = Clock::get()?.unix_timestamp;
    let c = &mut a.bonding_curve;
    require!(c.token_total_supply > 0, HoldNowCode::LaunchNotCreated);
    require!(
        !c.complete && !c.migrated,
        HoldNowCode::BondingCurveComplete
    );
    let i = c.stage_index(
        u8::try_from(c.current_stage).map_err(|_| error!(HoldNowCode::InvalidStage))?,
    )?;
    if c.current_stage_complete {
        // Trading remains paused until a durable publication (including an explicit
        // empty settlement) is confirmed. Claims have no expiry and remain open.
        require!(c.stage_settled[i], HoldNowCode::StageNotSettled);
        require!(
            now >= c
                .at_start_stage
                .checked_add(c.claim_duration)
                .ok_or(HoldNowCode::MathOverflow)?,
            HoldNowCode::NotMoveNextStage
        );
        c.current_stage = c
            .current_stage
            .checked_add(1)
            .ok_or(HoldNowCode::MathOverflow)?;
        c.at_start_stage = now;
        if c.current_stage > c.number_stage {
            c.complete = true;
            emit!(CompleteEvent {
                user: a.user.key(),
                mint: a.mint.key(),
                bonding_curve: c.key(),
                timestamp: now
            });
        } else {
            c.current_stage_complete = false;
            emit!(MoveStageEvent {
                user: a.user.key(),
                mint: a.mint.key(),
                bonding_curve: c.key(),
                current_stage: c.current_stage,
                reserve_token: c.virtual_token_reserves,
                reserve_lamport: c.virtual_sol_reserves,
                timestamp: now,
            });
        }
    } else {
        require!(
            now >= c
                .at_start_stage
                .checked_add(c.stage_duration)
                .ok_or(HoldNowCode::MathOverflow)?,
            HoldNowCode::NotStageComplete
        );
        c.stage_completion_times[i] = now;
        c.current_stage_complete = true;
        c.at_start_stage = now;
        emit!(CompleteStageEvent {
            user: a.user.key(),
            mint: a.mint.key(),
            bonding_curve: c.key(),
            current_stage: c.current_stage,
            timestamp: now
        });
    }
    Ok(())
}

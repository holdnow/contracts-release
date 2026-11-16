use crate::*;
use solana_program::program::invoke;

//  transfer sol from user
pub fn sol_transfer_user<'a>(
    source: AccountInfo<'a>,
    destination: AccountInfo<'a>,
    system_program: AccountInfo<'a>,
    amount: u64,
) -> Result<()> {
    let ix = solana_system_interface::instruction::transfer(source.key, destination.key, amount);
    invoke(&ix, &[source, destination, system_program])?;
    Ok(())
}
/// Exact-input constant-product output, rounded DOWN in the recipient's units.
pub fn curve_output(input: u64, reserve_in: u64, reserve_out: u64) -> Result<u64> {
    require!(
        reserve_in > 0 && reserve_out > 0,
        error::HoldNowCode::InvalidAmount
    );
    let denominator = reserve_in as u128 + input as u128;
    Ok(((input as u128 * reserve_out as u128) / denominator) as u64)
}
pub fn percentage(amount: u64, rate: u64) -> Result<u64> {
    require!(rate <= 100, error::HoldNowCode::InvalidTaxPercentage);
    Ok((amount as u128 * rate as u128 / 100) as u64)
}
pub fn current_tax(curve: &BondingCurve, now: i64) -> Result<u64> {
    require!(
        curve.sell_tax_min <= curve.sell_tax_max
            && curve.sell_tax_max <= 100
            && curve.sell_tax_decay <= 100
            && curve.stage_duration > 0,
        error::HoldNowCode::InvalidConfiguration
    );
    let elapsed = now.saturating_sub(curve.at_start_stage).max(0) as u128;
    let decay = curve.stage_duration as u128 * curve.sell_tax_decay as u128 / 100;
    if decay == 0 || elapsed >= decay {
        return Ok(curve.sell_tax_min as u64);
    }
    let range = (curve.sell_tax_max - curve.sell_tax_min) as u128;
    Ok(curve.sell_tax_max as u64 - (range * elapsed / decay) as u64)
}

/// Preserve the launch invariant without truncating the adjusted u128 reserve.
pub fn launch_reserves(tokens: u64, sol: u64, airdrop: u64) -> Result<(u64, u64)> {
    let remaining = tokens
        .checked_sub(airdrop)
        .ok_or(error::HoldNowCode::MathOverflow)?;
    require!(remaining > 0 && sol > 0, error::HoldNowCode::InvalidAmount);
    let adjusted = u64::try_from((sol as u128 * tokens as u128) / remaining as u128)
        .map_err(|_| error!(error::HoldNowCode::MathOverflow))?;
    Ok((remaining, adjusted))
}

/// The create argument remains a count of stage units, preserving production day semantics.
pub fn stage_duration_seconds(stage_units: u64) -> Result<u64> {
    require!(
        (1..=crate::MAX_STAGE_UNITS).contains(&stage_units),
        error::HoldNowCode::InvalidConfiguration
    );
    stage_units
        .checked_mul(crate::STAGE_UNIT_SECONDS)
        .ok_or_else(|| error!(error::HoldNowCode::MathOverflow))
}

use anchor_lang::solana_program::system_program;
// Minimal AMM v4 Initialize2 wire encoder. Account order and tag are public ABI.
// Reference: raydium-io/raydium-contract-instructions commit
// 8c98edf604172b3b796d256271e8560c4a3bcc9f, lib/src/amm_instruction.rs.
// Tests compare the entire instruction against that pinned upstream encoder.
use solana_program::{
    instruction::{AccountMeta, Instruction},
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar,
};
#[allow(clippy::too_many_arguments)]
pub(crate) fn initialize2(
    program: &Pubkey,
    pool: &Pubkey,
    authority: &Pubkey,
    orders: &Pubkey,
    lp: &Pubkey,
    coin_mint: &Pubkey,
    pc_mint: &Pubkey,
    coin_vault: &Pubkey,
    pc_vault: &Pubkey,
    target: &Pubkey,
    config: &Pubkey,
    fee: &Pubkey,
    market_program: &Pubkey,
    market: &Pubkey,
    wallet: &Pubkey,
    coin: &Pubkey,
    pc: &Pubkey,
    lp_account: &Pubkey,
    nonce: u8,
    open_time: u64,
    pc_amount: u64,
    coin_amount: u64,
) -> Result<Instruction, ProgramError> {
    let mut data = vec![1, nonce];
    for n in [open_time, pc_amount, coin_amount] {
        data.extend_from_slice(&n.to_le_bytes());
    }
    let accounts = vec![
        AccountMeta::new_readonly(anchor_spl::token::ID, false),
        AccountMeta::new_readonly(anchor_spl::associated_token::ID, false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(sysvar::rent::ID, false),
        AccountMeta::new(*pool, false),
        AccountMeta::new_readonly(*authority, false),
        AccountMeta::new(*orders, false),
        AccountMeta::new(*lp, false),
        AccountMeta::new_readonly(*coin_mint, false),
        AccountMeta::new_readonly(*pc_mint, false),
        AccountMeta::new(*coin_vault, false),
        AccountMeta::new(*pc_vault, false),
        AccountMeta::new(*target, false),
        AccountMeta::new_readonly(*config, false),
        AccountMeta::new(*fee, false),
        AccountMeta::new_readonly(*market_program, false),
        AccountMeta::new_readonly(*market, false),
        AccountMeta::new(*wallet, true),
        AccountMeta::new(*coin, false),
        AccountMeta::new(*pc, false),
        AccountMeta::new(*lp_account, false),
    ];
    Ok(Instruction {
        program_id: *program,
        accounts,
        data,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initialize2_matches_pinned_upstream_abi_byte_for_byte() {
        let k: Vec<_> = (1u8..=18)
            .map(|i| Pubkey::new_from_array([i; 32]))
            .collect();
        macro_rules! build {
            ($f:path) => {
                $f(
                    &k[0], &k[1], &k[2], &k[3], &k[4], &k[5], &k[6], &k[7], &k[8], &k[9], &k[10],
                    &k[11], &k[12], &k[13], &k[14], &k[15], &k[16], &k[17], 17, 123, 999, 777,
                )
                .unwrap()
            };
        }
        let instruction = build!(initialize2);
        let mut bytes = instruction.program_id.to_bytes().to_vec();
        bytes.extend_from_slice(&instruction.data);
        for meta in &instruction.accounts {
            bytes.extend_from_slice(meta.pubkey.as_ref());
            bytes.push(meta.is_signer as u8);
            bytes.push(meta.is_writable as u8);
        }
        assert_eq!(
            bytes.as_slice(),
            include_bytes!("../tests/fixtures/raydium_initialize2.bin")
        );
    }
}

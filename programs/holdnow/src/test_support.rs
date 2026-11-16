//! In-memory account fixtures for pure unit tests; never compiled into the program.
use crate::BondingCurve;
use anchor_lang::prelude::*;

pub fn curve(tokens: u64, sol: u64) -> BondingCurve {
    let bytes = vec![0u8; 1_024];
    let mut c = BondingCurve::try_deserialize_unchecked(&mut bytes.as_slice()).unwrap();
    c.virtual_token_reserves = tokens;
    c.virtual_sol_reserves = sol;
    c.real_token_reserves = tokens;
    c.token_total_supply = tokens;
    c
}

pub fn account<T: AccountSerialize + AccountDeserialize + Owner + Clone>(
    value: T,
) -> Account<'static, T> {
    let key = Box::leak(Box::new(Pubkey::new_unique()));
    let owner = Box::leak(Box::new(T::owner()));
    let lamports = Box::leak(Box::new(10_000_000));
    let mut data = Vec::new();
    value.try_serialize(&mut data).unwrap();
    let data = Box::leak(data.into_boxed_slice());
    let info = Box::leak(Box::new(AccountInfo::new(
        key, false, true, lamports, data, owner, false,
    )));
    Account::try_from(info).unwrap()
}

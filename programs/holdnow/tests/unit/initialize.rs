use super::*;
fn global() -> Global {
    Global::try_deserialize_unchecked(&mut vec![0u8; 512].as_slice()).unwrap()
}
#[test]
fn bootstrap_sets_authority_and_reinitialization_is_rejected() {
    let owner = crate::deployment::BOOTSTRAP_AUTHORITY;
    let mut global = global();
    initialize_global(&mut global, owner).unwrap();
    assert!(global.initialized);
    assert_eq!(global.authority, owner);
    assert_eq!(global.protocol_version, crate::PROTOCOL_VERSION);
    assert_eq!(global.stage_unit_seconds, crate::STAGE_UNIT_SECONDS);
    assert_eq!(global.minimum_migration_sol, crate::NEW_LAUNCH_MIN_MIGRATION_SOL);
    assert!(initialize_global(&mut global, owner).is_err());
}
#[test]
fn unauthorized_bootstrap_does_not_change_state() {
    let mut global = global();
    assert!(initialize_global(&mut global, Pubkey::new_unique()).is_err());
    assert!(!global.initialized);
    assert_eq!(global.authority, Pubkey::default());
}

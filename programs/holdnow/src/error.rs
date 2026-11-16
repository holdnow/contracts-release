use anchor_lang::prelude::*;

#[error_code]
pub enum HoldNowCode {
    #[msg("The given account is not authorized to execute this instruction.")]
    NotAuthorized,

    #[msg("The given account is not valid fee recipient.")]
    InValidFeeRecipient,

    #[msg("The program is already initialized.")]
    AlreadyInitialized,

    #[msg("Slippage: Tokens you'd receive does not meet the minimum tokens you requested")]
    InsufficientOutput,

    #[msg("slippage: Too much SOL required to buy the given amount of tokens.")]
    TooMuchSolRequired,

    #[msg("slippage: Too little SOL received to sell the given amount of tokens.")]
    TooLittleSolReceived,

    #[msg("The mint does not match the bonding curve.")]
    MintDoesNotMatchBondingCurve,

    #[msg("The bonding curve has completed and liquidity migrated to raydium.")]
    BondingCurveComplete,

    #[msg("The bonding curve has not completed.")]
    BondingCurveNotComplete,

    #[msg("The program is not initialized.")]
    NotInitialized,

    #[msg("Math operation overflow.")]
    MathOverflow,

    #[msg("Amount should be bigger than 0.")]
    ZeroAmount,

    #[msg("Amount is invalid to create the pool.")]
    InvalidAmount,

    #[msg("Supply is invalid to create the pool.")]
    InvalidSupply,

    #[msg("Freeze authority enabled.")]
    FreezeAuthorityEnabled,

    #[msg("Mint authority enabled.")]
    MitAuthorityEnabled,

    #[msg("TaxPercentage invalid.")]
    InvalidTaxPercentage,

    #[msg("FeeBasisPoints invalid.")]
    InvalidFeeBasisPoints,

    #[msg("The Current stage has completed.")]
    CurrentStageComplete,

    #[msg("The Token is insufficient.")]
    InsufficientFunds,

    #[msg("Now, Next stage doesn't start.")]
    NotMoveNextStage,

    #[msg("Now, current stage has not completeted.")]
    NotStageComplete,

    #[msg("Now, Token pool destination is incorrect.")]
    IncorrectPoolDestination,

    #[msg("Invalid airdrop allocation specified.")]
    InvalidAirdropAmount,

    #[msg("Invalid Claim Amount.")]
    InvalidClaimAmount,

    #[msg("No Claimable Amount")]
    InvalidNoClaimAmount,

    #[msg("No Airdrop Amount.")]
    InvalidNoAirdropAmount,

    #[msg("Arithmetic underflow occurred.")]
    ArithmeticUnderflow,

    #[msg("Price calculation would result in zero or negative value.")]
    InvalidPriceCalculation,

    #[msg("Invalid Merkle proof provided")]
    InvalidMerkleProof,

    #[msg("Claim amount exceeds allocated amount")]
    ExcessiveClaimAmount,

    #[msg("Merkle root not set for this stage")]
    MerkleRootNotSet,

    #[msg("Insufficient reward pool balance")]
    InsufficientRewardPool,

    #[msg("Nothing to claim")]
    NothingToClaim,

    #[msg("Stage not complete")]
    StageNotComplete,

    #[msg("Invalid stage")]
    InvalidStage,

    #[msg("Initial allocations already set")]
    InitialAllocationsAlreadySet,

    #[msg("Invalid stage count")]
    InvalidStageCount,

    #[msg("Insufficient funds in the token vault")]
    InsufficientVaultFunds,

    #[msg("Migration fee must equal 100 basis points")]
    InvalidFeeRate,

    #[msg("Claim duration is too short")]
    ClaimWindowTooShort,

    #[msg("Claim duration is too long")]
    ClaimWindowTooLong,

    #[msg("Launch configuration is invalid")]
    InvalidConfiguration,
    #[msg("Launch has not been created")]
    LaunchNotCreated,
    #[msg("Trading deadline has elapsed; complete the stage first")]
    StageExpired,
    #[msg("The stage root is immutable once published")]
    MerkleRootAlreadySet,
    #[msg("Liquidity was already migrated")]
    AlreadyMigrated,
    #[msg("Untrusted migration program or account")]
    InvalidMigrationAccount,
    #[msg("Migration must mint LP tokens and burn the received balance")]
    InvalidLiquidity,
    #[msg("Reward category budget exceeded")]
    RewardBudgetExceeded,
    #[msg("Legacy accounts require an explicit, reconciled migration")]
    UnsupportedVersion,
    #[msg("Publish the stage reward settlement before advancing")]
    StageNotSettled,
    #[msg("The launch can fund migration; redemption is unavailable")]
    MigrationFunded,
    #[msg("The launch is permanently in redemption mode")]
    RedemptionActive,
    #[msg("Activate redemption before redeeming tokens")]
    RedemptionNotActive,
    #[msg("Migration deadline elapsed; holders may redeem remaining SOL")]
    MigrationExpired,
}

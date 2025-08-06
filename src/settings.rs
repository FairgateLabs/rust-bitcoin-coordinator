// Default Bitcoin Coordinator constants

// Context string for CPFP transactions.
pub const CPFP_TRANSACTION_CONTEXT: &str = "CPFP_TRANSACTION";
pub const RBF_TRANSACTION_CONTEXT: &str = "RBF_TRANSACTION";
pub const FUNDING_TRANSACTION_CONTEXT: &str = "FUNDING_TRANSACTION";

// Bitcoin Core has a mempool policy called the "chain limit":
// You can’t have more than 25 unconfirmed transactions chained together (i.e. one spending the other).
pub const MAX_LIMIT_UNCONFIRMED_PARENTS: u32 = 25;

// Minimum number of unconfirmed transactions required to dispatch a CPFP (Child Pays For Parent) transaction.
// This is due to Bitcoin's mempool chain limit policy, which restricts the number of unconfirmed transactions that can be chained together (default is 25).
// To create a valid CPFP, there must be at least one unconfirmed parent transaction and at least one unconfirmed output available to spend for the CPFP.
// This ensures that the CPFP transaction can be constructed and accepted by the mempool under Bitcoin's standardness rules.
pub const MIN_UNCONFIRMED_TXS_FOR_CPFP: u32 = 2;

// SETTINGS CONFIGURABLE:

// Maximum number of unconfirmed speedup transactions allowed before triggering a replacement speedup.
// If the number of unconfirmed speedups reaches this limit, the coordinator will attempt to replace them with a new speedup transaction.
pub const DEFAULT_MAX_UNCONFIRMED_SPEEDUPS: u32 = 10;

// Maximum transaction weight in bytes.
pub const DEFAULT_MAX_TX_WEIGHT: u64 = 400_000;

// Maximum number of RBF attempts for a single transaction
pub const DEFAULT_MAX_RBF_ATTEMPTS: u32 = 10;

// Minimum funding amount in sats to ensure sufficient funds for speedups
pub const DEFAULT_MIN_FUNDING_AMOUNT_SATS: u64 = 10000;

// Fee percentage increase for RBF (150% of original fee)
pub const DEFAULT_RBF_FEE_PERCENTAGE: f64 = 1.5;

// Minimum blocks to wait before attempting to resend a speedup transaction (CPFP or RBF)
pub const DEFAULT_MIN_BLOCKS_BEFORE_RESEND_SPEEDUP: u32 = 1;

// Maximum feerate sat/vbyte allowed for speedups
pub const DEFAULT_MAX_FEERATE_SAT_VB: u64 = 1000;

// Fee multiplier for base fee multiplier
pub const DEFAULT_BASE_FEE_MULTIPLIER: f64 = 1.0;

// Bump fee percentage
pub const DEFAULT_BUMP_FEE_PERCENTAGE: f64 = 1.5;

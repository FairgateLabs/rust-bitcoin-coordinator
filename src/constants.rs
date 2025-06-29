// Context string for CPFP transactions.
pub const CPFP_TRANSACTION_CONTEXT: &str = "CPFP_TRANSACTION";
pub const RBF_TRANSACTION_CONTEXT: &str = "RBF_TRANSACTION";
pub const FUNDING_TRANSACTION_CONTEXT: &str = "FUNDING_TRANSACTION";

// Maximum number of unconfirmed speedup transactions allowed before triggering a replacement speedup.
// If the number of unconfirmed speedups reaches this limit, the coordinator will attempt to replace them with a new speedup transaction.
pub const MAX_UNCONFIRMED_SPEEDUPS: usize = 10;

// Stop monitoring a transaction after 100 confirmations.
// In case of a reorganization bigger than 100 blocks, we have to do a rework in the coordinator.
pub const MAX_MONITORING_CONFIRMATIONS: u32 = 100;

// Maximum transaction weight in bytes.
pub const MAX_TX_WEIGHT: u64 = 400_000;

// Maximum number of RBF attempts for a single transaction
pub const MAX_RBF_ATTEMPTS: u32 = 10;

// Minimum funding amount in sats to ensure sufficient funds for speedups
pub const MIN_FUNDING_AMOUNT_SATS: u64 = 10000;

// Fee percentage increase for RBF (150% of original fee)
pub const RBF_FEE_PERCENTAGE: f64 = 1.5;

// Minimum blocks to wait before attempting RBF
pub const MIN_BLOCKS_BEFORE_RBF: u32 = 1;

// Maximum feerate sat/vbyte allowed for speedups
pub const MAX_FEERATE_SAT_VB: u64 = 1000;

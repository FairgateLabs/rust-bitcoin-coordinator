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

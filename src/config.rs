use crate::constants::{
    DEFAULT_MAX_FEERATE_SAT_VB, DEFAULT_MAX_RBF_ATTEMPTS, DEFAULT_MAX_TX_WEIGHT,
    DEFAULT_MAX_UNCONFIRMED_SPEEDUPS, DEFAULT_MIN_BLOCKS_BEFORE_RBF,
    DEFAULT_MIN_FUNDING_AMOUNT_SATS, DEFAULT_RBF_FEE_PERCENTAGE,
};
use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use bitvmx_transaction_monitor::config::MonitorConstants;
use key_manager::config::KeyManagerConfig;
use serde::Deserialize;
use storage_backend::storage_config::StorageConfig;
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)] // enforce strict field compliance
pub struct CoordinatorConfig {
    pub storage: StorageConfig,
    pub rpc: RpcConfig,
    pub key_manager: KeyManagerConfig,
    pub key_storage: StorageConfig,
    pub constants: Option<CoordinatorConstants>,
    pub log_level: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CoordinatorConstants {
    pub max_unconfirmed_speedups: u32,
    pub max_tx_weight: u64,
    pub max_rbf_attempts: u32,
    pub min_funding_amount_sats: u64,
    pub rbf_fee_percentage: f64,
    pub min_blocks_before_rbf: u32,
    pub max_feerate_sat_vb: u64,
    pub monitor_constants: MonitorConstants,
}

impl Default for CoordinatorConstants {
    fn default() -> Self {
        Self {
            max_unconfirmed_speedups: DEFAULT_MAX_UNCONFIRMED_SPEEDUPS,
            max_tx_weight: DEFAULT_MAX_TX_WEIGHT,
            max_rbf_attempts: DEFAULT_MAX_RBF_ATTEMPTS,
            min_funding_amount_sats: DEFAULT_MIN_FUNDING_AMOUNT_SATS,
            rbf_fee_percentage: DEFAULT_RBF_FEE_PERCENTAGE,
            min_blocks_before_rbf: DEFAULT_MIN_BLOCKS_BEFORE_RBF,
            max_feerate_sat_vb: DEFAULT_MAX_FEERATE_SAT_VB,
            monitor_constants: MonitorConstants::default(),
        }
    }
}

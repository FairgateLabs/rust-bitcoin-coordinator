use crate::settings::{
    DEFAULT_BASE_FEE_MULTIPLIER, DEFAULT_BUMP_FEE_PERCENTAGE, DEFAULT_MAX_FEERATE_SAT_VB,
    DEFAULT_MAX_RBF_ATTEMPTS, DEFAULT_MAX_TX_WEIGHT, DEFAULT_MAX_UNCONFIRMED_SPEEDUPS,
    DEFAULT_MIN_BLOCKS_BEFORE_RESEND_SPEEDUP, DEFAULT_MIN_FUNDING_AMOUNT_SATS,
    DEFAULT_RBF_FEE_PERCENTAGE, DEFAULT_RETRY_ATTEMPTS_SENDING_TX, DEFAULT_RETRY_INTERVAL_SECONDS,
};
use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use bitvmx_transaction_monitor::config::MonitorSettings;
use key_manager::config::KeyManagerConfig;
use serde::Deserialize;
use storage_backend::storage_config::StorageConfig;
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)] // enforce strict field compliance
pub struct CoordinatorConfig {
    pub storage: StorageConfig,
    pub rpc: RpcConfig,
    pub key_manager: KeyManagerConfig,
    pub key_storage: StorageConfig,
    pub settings: Option<CoordinatorSettings>,
    pub log_level: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CoordinatorSettings {
    pub max_unconfirmed_speedups: u32,
    pub max_tx_weight: u64,
    pub max_rbf_attempts: u32,
    pub min_funding_amount_sats: u64,
    pub rbf_fee_percentage: f64,
    pub min_blocks_before_resend_speedup: u32,
    pub max_feerate_sat_vb: u64,
    pub monitor_settings: MonitorSettings,
    pub base_fee_multiplier: f64,
    pub bump_fee_percentage: f64,
    pub retry_interval_seconds: u64,
    pub retry_attempts_sending_tx: u32,
}

impl Default for CoordinatorSettings {
    fn default() -> Self {
        Self {
            max_unconfirmed_speedups: DEFAULT_MAX_UNCONFIRMED_SPEEDUPS,
            max_tx_weight: DEFAULT_MAX_TX_WEIGHT,
            max_rbf_attempts: DEFAULT_MAX_RBF_ATTEMPTS,
            min_funding_amount_sats: DEFAULT_MIN_FUNDING_AMOUNT_SATS,
            rbf_fee_percentage: DEFAULT_RBF_FEE_PERCENTAGE,
            min_blocks_before_resend_speedup: DEFAULT_MIN_BLOCKS_BEFORE_RESEND_SPEEDUP,
            max_feerate_sat_vb: DEFAULT_MAX_FEERATE_SAT_VB,
            monitor_settings: MonitorSettings::default(),
            base_fee_multiplier: DEFAULT_BASE_FEE_MULTIPLIER,
            bump_fee_percentage: DEFAULT_BUMP_FEE_PERCENTAGE,
            retry_interval_seconds: DEFAULT_RETRY_INTERVAL_SECONDS,
            retry_attempts_sending_tx: DEFAULT_RETRY_ATTEMPTS_SENDING_TX,
        }
    }
}

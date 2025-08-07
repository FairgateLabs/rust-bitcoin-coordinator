use crate::settings::{
    DEFAULT_MAX_FEERATE_SAT_VB, DEFAULT_MAX_RBF_ATTEMPTS, DEFAULT_MAX_TX_WEIGHT,
    DEFAULT_MAX_UNCONFIRMED_SPEEDUPS, DEFAULT_MIN_BLOCKS_BEFORE_RESEND_SPEEDUP,
    DEFAULT_MIN_FUNDING_AMOUNT_SATS, DEFAULT_RBF_FEE_PERCENTAGE,
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
pub struct Settings {
    pub max_unconfirmed_speedups: u32,
    pub max_tx_weight: u64,
    pub max_rbf_attempts: u32,
    pub min_funding_amount_sats: u64,
    pub rbf_fee_percentage: f64,
    pub min_blocks_before_resend_speedup: u32,
    pub max_feerate_sat_vb: u64,
    pub monitor_settings: MonitorSettings,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CoordinatorSettings {
    pub max_unconfirmed_speedups: Option<u32>,
    pub max_tx_weight: Option<u64>,
    pub max_rbf_attempts: Option<u32>,
    pub min_funding_amount_sats: Option<u64>,
    pub rbf_fee_percentage: Option<f64>,
    pub min_blocks_before_resend_speedup: Option<u32>,
    pub max_feerate_sat_vb: Option<u64>,
    pub monitor_settings: Option<MonitorSettings>,
    pub base_fee_multiplier: Option<f64>,
    pub bump_fee_percentage: Option<f64>,
}

impl Default for CoordinatorSettings {
    fn default() -> Self {
        Self {
            max_unconfirmed_speedups: None,
            max_tx_weight: None,
            max_rbf_attempts: None,
            min_funding_amount_sats: None,
            rbf_fee_percentage: None,
            min_blocks_before_resend_speedup: None,
            max_feerate_sat_vb: None,
            monitor_settings: None,
            base_fee_multiplier: None,
            bump_fee_percentage: None,
        }
    }
}

impl From<CoordinatorSettings> for Settings {
    fn from(settings: CoordinatorSettings) -> Self {
        Self {
            max_unconfirmed_speedups: settings
                .max_unconfirmed_speedups
                .unwrap_or(DEFAULT_MAX_UNCONFIRMED_SPEEDUPS),

            max_tx_weight: settings.max_tx_weight.unwrap_or(DEFAULT_MAX_TX_WEIGHT),

            max_rbf_attempts: settings
                .max_rbf_attempts
                .unwrap_or(DEFAULT_MAX_RBF_ATTEMPTS),

            min_funding_amount_sats: settings
                .min_funding_amount_sats
                .unwrap_or(DEFAULT_MIN_FUNDING_AMOUNT_SATS),

            rbf_fee_percentage: settings
                .rbf_fee_percentage
                .unwrap_or(DEFAULT_RBF_FEE_PERCENTAGE),

            min_blocks_before_resend_speedup: settings
                .min_blocks_before_resend_speedup
                .unwrap_or(DEFAULT_MIN_BLOCKS_BEFORE_RESEND_SPEEDUP),

            max_feerate_sat_vb: settings
                .max_feerate_sat_vb
                .unwrap_or(DEFAULT_MAX_FEERATE_SAT_VB),

            monitor_settings: settings.monitor_settings.unwrap_or_default().into(),
        }
    }
}

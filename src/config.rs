use crate::settings::{
    DEFAULT_BASE_FEE_MULTIPLIER, DEFAULT_BUMP_FEE_PERCENTAGE, DEFAULT_MAX_FEERATE_SAT_VB,
    DEFAULT_MAX_RBF_ATTEMPTS, DEFAULT_MAX_TX_WEIGHT, DEFAULT_MAX_UNCONFIRMED_SPEEDUPS,
    DEFAULT_MIN_BLOCKS_BEFORE_RESEND_SPEEDUP, DEFAULT_MIN_FUNDING_AMOUNT_SATS,
    DEFAULT_MIN_NETWORK_FEE_RATE, DEFAULT_RBF_FEE_PERCENTAGE, DEFAULT_RETRY_ATTEMPTS_SENDING_TX,
    DEFAULT_RETRY_INTERVAL_SECONDS,
};
use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use bitvmx_transaction_monitor::config::{MonitorSettings, MonitorSettingsConfig};
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
    pub settings: Option<CoordinatorSettingsConfig>,
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
    pub min_network_fee_rate: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CoordinatorSettingsConfig {
    pub max_unconfirmed_speedups: Option<u32>,
    pub max_tx_weight: Option<u64>,
    pub max_rbf_attempts: Option<u32>,
    pub min_funding_amount_sats: Option<u64>,
    pub rbf_fee_percentage: Option<f64>,
    pub min_blocks_before_resend_speedup: Option<u32>,
    pub max_feerate_sat_vb: Option<u64>,
    pub monitor_settings: Option<MonitorSettingsConfig>,
    pub base_fee_multiplier: Option<f64>,
    pub bump_fee_percentage: Option<f64>,
    pub retry_interval_seconds: Option<u64>,
    pub retry_attempts_sending_tx: Option<u32>,
    pub min_network_fee_rate: Option<u64>,
}

impl Default for CoordinatorSettingsConfig {
    fn default() -> Self {
        Self {
            max_unconfirmed_speedups: Some(DEFAULT_MAX_UNCONFIRMED_SPEEDUPS),
            max_tx_weight: Some(DEFAULT_MAX_TX_WEIGHT),
            max_rbf_attempts: Some(DEFAULT_MAX_RBF_ATTEMPTS),
            min_funding_amount_sats: Some(DEFAULT_MIN_FUNDING_AMOUNT_SATS),
            rbf_fee_percentage: Some(DEFAULT_RBF_FEE_PERCENTAGE),
            min_blocks_before_resend_speedup: Some(DEFAULT_MIN_BLOCKS_BEFORE_RESEND_SPEEDUP),
            max_feerate_sat_vb: Some(DEFAULT_MAX_FEERATE_SAT_VB),
            monitor_settings: Some(MonitorSettingsConfig::default()),
            base_fee_multiplier: Some(DEFAULT_BASE_FEE_MULTIPLIER),
            bump_fee_percentage: Some(DEFAULT_BUMP_FEE_PERCENTAGE),
            retry_interval_seconds: Some(DEFAULT_RETRY_INTERVAL_SECONDS),
            retry_attempts_sending_tx: Some(DEFAULT_RETRY_ATTEMPTS_SENDING_TX),
            min_network_fee_rate: Some(DEFAULT_MIN_NETWORK_FEE_RATE),
        }
    }
}

impl From<CoordinatorSettingsConfig> for CoordinatorSettings {
    fn from(settings: CoordinatorSettingsConfig) -> Self {
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

            base_fee_multiplier: settings
                .base_fee_multiplier
                .unwrap_or(DEFAULT_BASE_FEE_MULTIPLIER),

            bump_fee_percentage: settings
                .bump_fee_percentage
                .unwrap_or(DEFAULT_BUMP_FEE_PERCENTAGE),

            retry_interval_seconds: settings
                .retry_interval_seconds
                .unwrap_or(DEFAULT_RETRY_INTERVAL_SECONDS),

            retry_attempts_sending_tx: settings
                .retry_attempts_sending_tx
                .unwrap_or(DEFAULT_RETRY_ATTEMPTS_SENDING_TX),

            min_network_fee_rate: settings
                .min_network_fee_rate
                .unwrap_or(DEFAULT_MIN_NETWORK_FEE_RATE),
        }
    }
}

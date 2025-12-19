use crate::errors::BitcoinCoordinatorError;
use crate::settings::{
    DEFAULT_BASE_FEE_MULTIPLIER, DEFAULT_BUMP_FEE_PERCENTAGE, DEFAULT_MAX_FEERATE_SAT_VB,
    DEFAULT_MAX_RBF_ATTEMPTS, DEFAULT_MAX_TX_WEIGHT, DEFAULT_MAX_UNCONFIRMED_SPEEDUPS,
    DEFAULT_MIN_BLOCKS_BEFORE_RESEND_SPEEDUP, DEFAULT_MIN_FUNDING_AMOUNT_SATS,
    DEFAULT_MIN_NETWORK_FEE_RATE, DEFAULT_RBF_FEE_MULTIPLIER, DEFAULT_RETRY_ATTEMPTS_SENDING_TX,
    DEFAULT_RETRY_INTERVAL_SECONDS, MAX_LIMIT_UNCONFIRMED_PARENTS,
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
    pub rbf_fee_multiplier: Option<f64>,
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
            rbf_fee_multiplier: Some(DEFAULT_RBF_FEE_MULTIPLIER),
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

impl CoordinatorSettingsConfig {
    pub fn validate(&self) -> Result<(), BitcoinCoordinatorError> {
        if let Some(max_unconfirmed_speedups) = self.max_unconfirmed_speedups {
            if max_unconfirmed_speedups == 0 {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "max_unconfirmed_speedups must be greater than 0, got {}",
                    max_unconfirmed_speedups
                )));
            }
            if max_unconfirmed_speedups > MAX_LIMIT_UNCONFIRMED_PARENTS {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(
                    format!(
                        "max_unconfirmed_speedups ({}) exceeds Bitcoin's chain limit of {} unconfirmed transactions",
                        max_unconfirmed_speedups, MAX_LIMIT_UNCONFIRMED_PARENTS
                    ),
                ));
            }
        }

        if let Some(max_tx_weight) = self.max_tx_weight {
            if max_tx_weight == 0 {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "max_tx_weight must be greater than 0, got {}",
                    max_tx_weight
                )));
            }
            if max_tx_weight > DEFAULT_MAX_TX_WEIGHT {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "max_tx_weight ({}) exceeds Bitcoin's maximum transaction weight of {}",
                    max_tx_weight, DEFAULT_MAX_TX_WEIGHT
                )));
            }
        }

        if let Some(max_rbf_attempts) = self.max_rbf_attempts {
            if max_rbf_attempts == 0 {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "max_rbf_attempts must be greater than 0, got {}",
                    max_rbf_attempts
                )));
            }
            if max_rbf_attempts > DEFAULT_MAX_RBF_ATTEMPTS {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "max_rbf_attempts ({}) exceeds reasonable limit of {}",
                    max_rbf_attempts, DEFAULT_MAX_RBF_ATTEMPTS
                )));
            }
        }

        if let Some(min_funding_amount_sats) = self.min_funding_amount_sats {
            if min_funding_amount_sats < DEFAULT_MIN_FUNDING_AMOUNT_SATS {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "min_funding_amount_sats ({}) is below Bitcoin's dust threshold of {} sats",
                    min_funding_amount_sats, DEFAULT_MIN_FUNDING_AMOUNT_SATS
                )));
            }
        }

        if let Some(rbf_fee_percentage) = self.rbf_fee_multiplier {
            if rbf_fee_percentage < 1.0 {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "rbf_fee_percentage ({}) must be at least 1.0 (100%) for RBF to be valid",
                    rbf_fee_percentage
                )));
            }
            if rbf_fee_percentage > 3.0 {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "rbf_fee_percentage ({}) exceeds reasonable limit of (300%)",
                    rbf_fee_percentage
                )));
            }
        }

        if let Some(min_blocks_before_resend_speedup) = self.min_blocks_before_resend_speedup {
            const MIN_BLOCKS: u32 = 1;
            const MAX_BLOCKS: u32 = 3;
            if min_blocks_before_resend_speedup < MIN_BLOCKS {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "min_blocks_before_resend_speedup ({}) must be at least {}",
                    min_blocks_before_resend_speedup, MIN_BLOCKS
                )));
            }
            if min_blocks_before_resend_speedup > MAX_BLOCKS {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "min_blocks_before_resend_speedup ({}) exceeds maximum allowed of {}",
                    min_blocks_before_resend_speedup, MAX_BLOCKS
                )));
            }
        }

        if let Some(max_feerate_sat_vb) = self.max_feerate_sat_vb {
            if max_feerate_sat_vb == 0 {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "max_feerate_sat_vb must be greater than 0, got {}",
                    max_feerate_sat_vb
                )));
            }
            if max_feerate_sat_vb > DEFAULT_MAX_FEERATE_SAT_VB {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "max_feerate_sat_vb ({}) exceeds reasonable limit of {} sat/vb",
                    max_feerate_sat_vb, DEFAULT_MAX_FEERATE_SAT_VB
                )));
            }
        }

        if let Some(base_fee_multiplier) = self.base_fee_multiplier {
            if base_fee_multiplier <= 0.0 {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "base_fee_multiplier must be greater than 0, got {}",
                    base_fee_multiplier
                )));
            }
            if base_fee_multiplier > 100.0 {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "base_fee_multiplier ({}) exceeds reasonable limit of 100.0",
                    base_fee_multiplier
                )));
            }
        }

        if let Some(bump_fee_percentage) = self.bump_fee_percentage {
            if bump_fee_percentage < 1.0 {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "bump_fee_percentage ({}) must be at least 1.0 (100%)",
                    bump_fee_percentage
                )));
            }
            if bump_fee_percentage > 100.0 {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "bump_fee_percentage ({}) exceeds reasonable limit of 100.0 (10000%)",
                    bump_fee_percentage
                )));
            }
        }

        if let Some(retry_interval_seconds) = self.retry_interval_seconds {
            if retry_interval_seconds == 0 {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "retry_interval_seconds must be greater than 0, got {}",
                    retry_interval_seconds
                )));
            }
            const MAX_RETRY_INTERVAL_SECONDS: u64 = 300; // 5 minutes
            if retry_interval_seconds > MAX_RETRY_INTERVAL_SECONDS {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "retry_interval_seconds ({}) exceeds maximum allowed of {} seconds (5 minutes)",
                    retry_interval_seconds, MAX_RETRY_INTERVAL_SECONDS
                )));
            }
        }

        if let Some(retry_attempts_sending_tx) = self.retry_attempts_sending_tx {
            if retry_attempts_sending_tx == 0 {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "retry_attempts_sending_tx must be greater than 0, got {}",
                    retry_attempts_sending_tx
                )));
            }
            const MAX_RETRY_ATTEMPTS: u32 = 10;
            if retry_attempts_sending_tx > MAX_RETRY_ATTEMPTS {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "retry_attempts_sending_tx ({}) exceeds maximum allowed of {}",
                    retry_attempts_sending_tx, MAX_RETRY_ATTEMPTS
                )));
            }
        }

        if let Some(min_network_fee_rate) = self.min_network_fee_rate {
            if min_network_fee_rate < 1 {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "min_network_fee_rate must be at least 1, got {}",
                    min_network_fee_rate
                )));
            }
        }

        // Cross-validation: min_network_fee_rate cannot exceed max_feerate_sat_vb
        if let (Some(min), Some(max)) = (self.min_network_fee_rate, self.max_feerate_sat_vb) {
            if min > max {
                return Err(BitcoinCoordinatorError::InvalidConfiguration(format!(
                    "min_network_fee_rate ({}) cannot exceed max_feerate_sat_vb ({})",
                    min, max
                )));
            }
        }

        Ok(())
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
                .rbf_fee_multiplier
                .unwrap_or(DEFAULT_RBF_FEE_MULTIPLIER),

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

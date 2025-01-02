use std::env;

use bitvmx_transaction_monitor::types::BlockHeight;
use config as settings;
use serde::Deserialize;
use tracing::warn;

use crate::errors::ConfigError;

static DEFAULT_ENV: &str = "development";
static CONFIG_PATH: &str = "config";

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)] // enforce strict field compliance
pub struct BlockchainConfig {
    pub database: DatabaseConfig,
    pub rpc: RpcConfig,
    pub monitor: MonitorConfig,
    pub dispatcher: DispatcherConfig,
    pub key_manager: KeyManagerConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RpcConfig {
    pub network: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub wallet: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MonitorConfig {
    pub checkpoint_height: Option<BlockHeight>,
    pub confirmation_threshold: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DispatcherConfig {
    // amount in sats of the output used to bump the fee of the DRP transaction
    pub cpfp_amount: u64,
    // fee in sats for the DRP transaction
    pub cpfp_fee: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct KeyManagerConfig {
    pub key_derivation_seed: String,
    pub key_derivation_path: String,
    pub winternitz_seed: String,
    pub storage: StorageConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct StorageConfig {
    pub password: String,
    pub path: String,
}

impl BlockchainConfig {
    pub fn load() -> Result<BlockchainConfig, ConfigError> {
        let env = BlockchainConfig::get_env();
        BlockchainConfig::parse_config(env)
    }

    fn get_env() -> String {
        env::var("BITVMX_ENV").unwrap_or_else(|_| {
            let default_env = DEFAULT_ENV.to_string();
            warn!(
                "BITVMX_ENV not set. Using default environment: {}",
                default_env
            );
            default_env
        })
    }

    fn parse_config(env: String) -> Result<BlockchainConfig, ConfigError> {
        let config_path = format!("{}/{}.yaml", CONFIG_PATH, env);

        let settings = settings::Config::builder()
            .add_source(config::File::with_name(&config_path))
            .build()
            .map_err(ConfigError::ConfigFileError)?;

        settings
            .try_deserialize::<BlockchainConfig>()
            .map_err(ConfigError::ConfigFileError)
    }
}

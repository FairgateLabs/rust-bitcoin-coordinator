use std::env;

use bitvmx_bitcoin_rpc::{rpc_config::RpcConfig, types::BlockHeight};
use config as settings;
use key_manager::config::{KeyManagerConfig, KeyStorageConfig};
use serde::Deserialize;
use tracing::warn;

use crate::errors::ConfigError;

static DEFAULT_ENV: &str = "development";
static CONFIG_PATH: &str = "config";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)] // enforce strict field compliance
pub struct Config {
    pub database: DatabaseConfig,
    pub rpc: RpcConfig,
    pub monitor: MonitorConfig,
    pub dispatcher: DispatcherConfig,
    pub key_manager: KeyManagerConfig,
    pub key_storage: KeyStorageConfig,
    pub log_level: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MonitorConfig {
    pub checkpoint_height: Option<BlockHeight>,
    pub confirmation_threshold: u32,
}

#[derive(Debug, Deserialize)]
pub struct DispatcherConfig {
    // amount in sats of the output used to bump the fee of the DRP transaction
    pub cpfp_amount: u64,
    // fee in sats for the DRP transaction
    pub cpfp_fee: u64,
}

impl Config {
    pub fn load() -> Result<Config, ConfigError> {
        let env = Config::get_env();
        Config::parse_config(env)
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

    fn parse_config(env: String) -> Result<Config, ConfigError> {
        let config_path = format!("{}/{}.yaml", CONFIG_PATH, env);

        let settings = settings::Config::builder()
            .add_source(config::File::with_name(&config_path))
            .build()
            .map_err(ConfigError::ConfigFileError)?;

        settings
            .try_deserialize::<Config>()
            .map_err(ConfigError::ConfigFileError)
    }
}

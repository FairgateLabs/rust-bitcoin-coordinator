use std::env;

use config as settings;
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
    pub dispatcher: DispatcherConfig,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct RpcConfig {
    pub network: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub wallet: String,
}

#[derive(Debug, Deserialize)]
pub struct  DispatcherConfig {
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
        env::var("BITVMX_ENV")
            .unwrap_or_else(|_| {
                let default_env = DEFAULT_ENV.to_string();
                warn!("BITVMX_ENV not set. Using default environment: {}", default_env);
                default_env
            }
        )
    }

    fn parse_config(env: String) -> Result<Config, ConfigError> {
        let config_path = format!("{}/{}.yaml", CONFIG_PATH, env);

        let settings = settings::Config::builder()
            .add_source(config::File::with_name(&config_path))
            .build()
            .map_err(ConfigError::ConfigFileError)?;

        settings.try_deserialize::<Config>()
            .map_err(ConfigError::ConfigFileError)
    }
}

use thiserror::Error;
use config as settings;


#[derive(Error, Debug)]
pub enum BitVMXError {
    #[error("Unexpected error: {0}")]
    Unexpected(String),
}
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Bad configuration: {0}")]
    BadConfig(String),
    #[error("while trying to build configuration")]
    ConfigFileError(#[from] settings::ConfigError),
}

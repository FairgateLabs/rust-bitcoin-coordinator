use thiserror::Error;
use config as settings;

#[derive(Error, Debug)]
pub enum BitVMXError {
    #[error("Unexpected error: {0}")]
    Unexpected(String),
}

#[derive(Error, Debug)]
pub enum BitcoinError {
    #[error("Error with Orchestrator: {0}")]
    OrchestratorError(#[from] OrchestratorError),
    #[error("Error with StepHandler: {0}")]
    StepHandlerError(#[from] StepHandlerError),
    #[error("Error with Storage: {0}")]
    StorageError(#[from] StorageError),
    #[error("Error with TxBuilderHelper: {0}")]
    TxBuilderHelperError(#[from] TxBuilderHelperError),
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Bad configuration: {0}")]
    BadConfig(String),
    #[error("while trying to build configuration")]
    ConfigFileError(#[from] settings::ConfigError),
}

#[derive(Error, Debug)]
pub enum OrchestratorError {
    #[error("Error with TransactionDispatcher: {0}")]
    DispatcherError(#[from] transaction_dispatcher::errors::DispatcherError),
    #[error("Error with TransactionMonitor: {0}")]
    MonitorError(String),
    #[error("Error with Storage: {0}")]
    StorageError(#[from] StorageError),
    #[error("Unexpected error: {0}")]
    Unexpected(String),
}

#[derive(Error, Debug)]
pub enum StepHandlerError {
    #[error("Error with Orchestrator: {0}")]
    OrchestratorError(#[from] OrchestratorError),
    #[error("Error with Storage: {0}")]
    StorageError(#[from] StorageError),
}

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Error with Storage: {0}")]
    StorageBackendError(#[from] storage_backend::error::StorageError),
    #[error("Unexpected error: {0}")]
    Unexpected(String),
}

#[derive(Error, Debug)]
pub enum TxBuilderHelperError {
    #[error("Failed to create sighash: {0}")]
    SighashError(#[from] bitcoin::sighash::P2wpkhError),
    #[error("Error while parsing network: {0}")]
    NetworkParseError(#[from] bitcoin::network::ParseNetworkError),
    #[error("Error while building Orchestrator: {0}")]
    OrchestratorError(String),
    #[error("Error while building KeyManager: {0}")]
    KeyManagerError(#[from] key_manager::errors::KeyManagerError),
    #[error("Error while building KeyStore: {0}")]
    KeyStoreError(#[from] key_manager::errors::KeyStoreError),
    #[error("Unexpected error: {0}")]
    Unexpected(String),
    #[error("Error with TransactionDispatcher: {0}")]
    DispatcherError(#[from] transaction_dispatcher::errors::DispatcherError),
    #[error("Error with Address parsing: {0}")]
    AddressParseError(#[from] bitcoin::address::ParseError),
}
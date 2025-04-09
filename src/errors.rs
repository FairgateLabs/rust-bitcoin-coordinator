use config as settings;
use thiserror::Error;

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

#[derive(Error, Debug)]
pub enum BitcoinCoordinatorStoreError {
    #[error("Error with Storage Backend: {0}")]
    StorageBackendError(#[from] storage_backend::error::StorageError),

    #[error("Error: {0}, {1}")]
    BitcoinCoordinatorStoreError(String, storage_backend::error::StorageError),

    #[error("Transaction not found: {0}")]
    TransactionNotFound(String),

    #[error("Invalid extra data")]
    InvalidExtraData,

    #[error("Funding key not found")]
    FundingKeyNotFound,

    #[error("Funding transaction not found")]
    FundingTransactionNotFound,
}

#[derive(Error, Debug)]
pub enum BitcoinCoordinatorError {
    #[error("Error with Bitcoin Coordinator Store: {0}")]
    BitcoinCoordinatorStoreError(#[from] BitcoinCoordinatorStoreError),

    #[error("Error with Monitor: {0}")]
    MonitorError(#[from] bitvmx_transaction_monitor::errors::MonitorError),

    #[error("Error with Dispatcher: {0}")]
    DispatcherError(#[from] transaction_dispatcher::errors::DispatcherError),

    #[error("Error with Bitcoin Coordinator: {0}")]
    BitcoinCoordinatorError(String),
}

#[derive(Error, Debug)]
pub enum TxBuilderHelperError {
    #[error("Hex Decoding error: {0}")]
    HexDecodingError(#[from] hex::FromHexError),

    #[error("{0} length must be {1} bytes")]
    LengthError(String, usize),

    #[error("Error while converting slice to array: {0}")]
    ConversionError(#[from] std::array::TryFromSliceError),

    #[error("Error while building KeyStore: {0}")]
    KeyStoreError(#[from] key_manager::errors::KeyStoreError),

    #[error("Error while building Dispatcher: {0}")]
    DispatcherError(#[from] transaction_dispatcher::errors::DispatcherError),

    #[error("Error while building BitcoinClient: {0}")]
    BitcoinClientError(#[from] bitcoincore_rpc::Error),

    #[error("Error while building parsing: {0}")]
    ParsingError(#[from] bitcoin::address::ParseError),

    #[error("Error while building KeyManager: {0}")]
    KeyManagerError(#[from] key_manager::errors::KeyManagerError),
}

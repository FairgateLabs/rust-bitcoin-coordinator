use bitvmx_bitcoin_rpc::errors::BitcoinClientError;
use config as settings;
use protocol_builder::errors::ProtocolBuilderError;
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

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Error: {0}, {1}")]
    BitcoinCoordinatorStoreError(String, storage_backend::error::StorageError),

    #[error("Transaction not found: {0}")]
    TransactionNotFound(String),

    #[error("Funding key not found")]
    FundingKeyNotFound,

    #[error("Funding transaction not found")]
    FundingNotFound,

    #[error("Funding transaction already exists")]
    FundingTransactionAlreadyExists,

    #[error("Speed up transaction not found")]
    SpeedupNotFound,

    #[error("Invalid transaction state")]
    InvalidTransactionState,

    #[error("Replace speedup not confirmed")]
    ReplaceSpeedupNotConfirmed,

    #[error("Transaction state transition invalid: from {0:?} to {1:?}")]
    InvalidStateTransition(TransactionState, TransactionState),

}

#[derive(Error, Debug)]
pub enum BitcoinCoordinatorError {
    #[error("Error with Bitcoin Coordinator Store: {0}")]
    BitcoinCoordinatorStoreError(#[from] BitcoinCoordinatorStoreError),

    #[error("Error with Monitor: {0}")]
    MonitorError(#[from] bitvmx_transaction_monitor::errors::MonitorError),

    #[error("Error with Bitcoin Coordinator: {0}")]
    BitcoinCoordinatorError(String),

    #[error("Transaction not found: {0}")]
    TransactionNotFound(String),

    #[error("Error with Bitcoin Client: {0}")]
    BitcoinClientError(#[from] BitcoinClientError),

    #[error("Rpc error: {0}")]
    RpcError(#[from] bitcoincore_rpc::Error),

    #[error("Protocol builder error: {0}")]
    ProtocolBuilderError(#[from] ProtocolBuilderError),

    #[error("Transaction too heavy: {0}, weight: {1}, max weight: {2}")]
    TransactionTooHeavy(String, u64, u64),
}

#[derive(Error, Debug)]
pub enum TxBuilderHelperError {
    #[error("Hex Decoding error: {0}")]
    HexDecodingError(#[from] hex::FromHexError),

    #[error("{0} length must be {1} bytes")]
    LengthError(String, usize),

    #[error("Error while converting slice to array: {0}")]
    ConversionError(#[from] std::array::TryFromSliceError),

    #[error("Error while building BitcoinClient: {0}")]
    BitcoinClientError(#[from] bitcoincore_rpc::Error),

    #[error("Error while building parsing: {0}")]
    ParsingError(#[from] bitcoin::address::ParseError),

    #[error("Error while building KeyManager: {0}")]
    KeyManagerError(#[from] key_manager::errors::KeyManagerError),
}

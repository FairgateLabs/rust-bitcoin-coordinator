use bitcoin::{Transaction, Txid};
use bitvmx_bitcoin_rpc::types::BlockHeight;
use bitvmx_transaction_monitor::types::{AckMonitorNews, MonitorNews};
use bitvmx_transaction_monitor::TransactionBlockchainStatus;
use protocol_builder::types::{output::SpeedupData, Utxo};
use serde::{Deserialize, Serialize};

use crate::settings::{
    CPFP_TRANSACTION_CONTEXT, FUNDING_TRANSACTION_CONTEXT, RBF_TRANSACTION_CONTEXT,
};

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum TransactionState {
    // The transaction is ready and queued to be sent.
    ToDispatch,

    // The transaction has been broadcast to the network and it is in the mempool. It is waiting for mining.
    InMempool,

    // The transaction has been mined and confirmed by the network.
    Confirmed,

    // The transaction has been successfully reach the target block height and it is considered finalized.
    Finalized,

    // The transaction has failed to be broadcasted and rejected by the network.
    Failed,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct CoordinatedTransaction {
    pub tx_id: Txid,
    pub tx: Transaction,
    // This is the utxo that will be used to pay for the transaction using CPFP (Child Pays For Parent)
    pub speedup_data: Option<SpeedupData>,
    pub broadcast_block_height: Option<BlockHeight>,
    pub target_block_height: Option<BlockHeight>,
    pub state: TransactionState,
    pub context: String,
    // Number of blocks to wait before considering the transaction stuck in mempool
    // None means this transaction doesn't have a stuck threshold
    pub stuck_in_mempool_blocks: Option<u32>,
}

impl CoordinatedTransaction {
    pub fn new(
        tx: Transaction,
        speedup_data: Option<SpeedupData>,
        state: TransactionState,
        target_block_height: Option<BlockHeight>,
        context: String,
        stuck_in_mempool_blocks: Option<u32>,
    ) -> Self {
        Self {
            tx_id: tx.compute_txid(),
            tx,
            speedup_data,
            broadcast_block_height: None,
            state,
            target_block_height,
            context,
            stuck_in_mempool_blocks,
        }
    }
}

// SpeedupState is now unified with TransactionState
pub type SpeedupState = TransactionState;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct CoordinatedSpeedUpTransaction {
    pub speedup_type: SpeedupType,

    pub tx_id: Txid,

    // The speedup transaction itself (needed for dispatch, None for funding transactions)
    pub tx: Option<Transaction>,

    // The previous funding utxo.
    pub prev_funding: Utxo,

    // The change funding utxo.
    pub next_funding: Utxo,

    // If Some(txid), this speedup has been replaced by the speedup transaction with this txid.
    // If None, this speedup is not replaced by any other speedup.
    pub replaced_by_tx_id: Option<Txid>,

    // If Some(txid), this speedup is a replacement (RBF) for the speedup transaction with this txid.
    // If None, it is a new speedup (CPFP)
    pub replaces_tx_id: Option<Txid>,

    pub broadcast_block_height: BlockHeight,

    pub state: SpeedupState,

    pub context: String,

    pub bump_fee_percentage_used: f64,

    pub speedup_tx_data: Vec<(SpeedupData, Transaction, String)>,

    pub network_fee_rate_used: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum SpeedupType {
    RBF,
    CPFP,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct RetryInfo {
    pub retries_count: u32,
    pub last_retry_timestamp: u64,
}

impl RetryInfo {
    pub fn new(count: u32, last_timestamp: u64) -> Self {
        Self {
            retries_count: count,
            last_retry_timestamp: last_timestamp,
        }
    }
}

#[allow(clippy::too_many_arguments)]
impl CoordinatedSpeedUpTransaction {
    pub fn new(
        tx_id: Txid,
        tx: Option<Transaction>,
        prev_funding: Utxo,
        next_funding: Utxo,
        replaces_tx_id: Option<Txid>,
        broadcast_block_height: BlockHeight,
        state: SpeedupState,
        bump_fee_percentage_used: f64,
        speedup_tx_data: Vec<(SpeedupData, Transaction, String)>,
        network_fee_rate_used: u64,
    ) -> Self {
        let is_rbf = replaces_tx_id.is_some();
        let mut context = if is_rbf {
            RBF_TRANSACTION_CONTEXT.to_string()
        } else {
            CPFP_TRANSACTION_CONTEXT.to_string()
        };

        if broadcast_block_height == 0
            && state == TransactionState::Finalized
            && speedup_tx_data.is_empty()
        {
            context = FUNDING_TRANSACTION_CONTEXT.to_string();
        }

        Self {
            speedup_type: if is_rbf {
                SpeedupType::RBF
            } else {
                SpeedupType::CPFP
            },
            tx_id,
            tx,
            prev_funding,
            next_funding,
            replaced_by_tx_id: None, // Initially, no transaction replaces this one
            replaces_tx_id,
            broadcast_block_height,
            state,
            context,
            bump_fee_percentage_used,
            speedup_tx_data,
            network_fee_rate_used,
        }
    }
}

impl CoordinatedSpeedUpTransaction {
    pub fn is_funding(&self) -> bool {
        self.broadcast_block_height == 0
            && self.state == TransactionState::Finalized
            && self.speedup_tx_data.is_empty()
    }

    /// Returns true if this speedup is replacing another speedup transaction
    pub fn is_replacing(&self) -> bool {
        self.replaces_tx_id.is_some()
    }

    /// Returns true if this speedup is being replaced by another speedup transaction
    pub fn is_being_replaced(&self) -> bool {
        self.replaced_by_tx_id.is_some()
    }

    pub fn get_tx_name(&self) -> String {
        if self.is_funding() {
            "FUNDING".to_string()
        } else if self.is_replacing() {
            "RBF".to_string()
        } else {
            "CPFP".to_string()
        }
    }
}

/// Direct, enum-to-enum mapping between the indexer blockchain status and our internal TransactionState.
impl From<TransactionBlockchainStatus> for TransactionState {
    fn from(status: TransactionBlockchainStatus) -> Self {
        match status {
            TransactionBlockchainStatus::InMempool => TransactionState::InMempool,
            TransactionBlockchainStatus::Confirmed => TransactionState::Confirmed,
            TransactionBlockchainStatus::Finalized => TransactionState::Finalized,
            TransactionBlockchainStatus::NotFound => TransactionState::ToDispatch,
            TransactionBlockchainStatus::Orphan => TransactionState::InMempool,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TransactionFullInfo {
    pub tx: Transaction,
}

#[derive(Debug, Clone, PartialEq)]
pub struct News {
    pub monitor_news: Vec<MonitorNews>,
    pub coordinator_news: Vec<CoordinatorNews>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoordinatorNews {
    /// Error when dispatching a transaction
    /// - Txid: The transaction ID that failed to dispatch
    /// - String: Context information about the transaction
    /// - String: Error message describing what went wrong
    DispatchTransactionError(Txid, String, String),

    /// Error when attempting to speed up a transaction
    /// - Vec<Txid>: The transaction IDs that failed to speed up
    /// - Vec<String>: Context information about the transactions that failed to be sent
    /// - Txid: The cpfp/rbf transaction ID that failed to be sent
    /// - String: Error message describing what went wrong
    DispatchSpeedUpError(Vec<Txid>, Vec<String>, Txid, String),

    /// Indicates insufficient funds for a funding transaction
    /// - Txid: The funding transaction ID that was insufficient
    /// - u64: The available funding amount
    /// - u64: The amount required for a speedup
    InsufficientFunds(Txid, u64, u64),

    /// Indicates that there are no funding utxo loaded
    FundingNotFound,

    /// Indicates that the estimate feerate is too high
    /// - u64: The estimate feerate from the node
    /// - u64: The max allowed feerate from settings
    EstimateFeerateTooHigh(u64, u64),

    /// Transaction is already in mempool (treated as success)
    /// - Txid: The transaction ID that is already in mempool
    /// - String: Context information about the transaction
    TransactionAlreadyInMempool(Txid, String),

    /// Mempool rejection (retryable error)
    /// - Txid: The transaction ID that was rejected
    /// - String: Context information about the transaction
    /// - String: Error message describing the rejection
    MempoolRejection(Txid, String, String),

    /// Network or connection error (retryable error)
    /// - Txid: The transaction ID that failed due to network issues
    /// - String: Context information about the transaction
    /// - String: Error message describing the network error
    NetworkError(Txid, String, String),

    /// Transaction is stuck in mempool for too long
    /// - Txid: The transaction ID that is stuck
    /// - String: Context information about the transaction
    TransactionStuckInMempool(Txid, String),
}

impl News {
    pub fn new(monitor_news: Vec<MonitorNews>, coordinator_news: Vec<CoordinatorNews>) -> Self {
        Self {
            monitor_news,
            coordinator_news,
        }
    }
}

pub enum AckCoordinatorNews {
    InsufficientFunds(Txid),
    DispatchTransactionError(Txid),
    DispatchSpeedUpError(Txid),
    EstimateFeerateTooHigh(u64, u64),
    FundingNotFound,
    TransactionAlreadyInMempool(Txid),
    MempoolRejection(Txid),
    NetworkError(Txid),
    TransactionStuckInMempool(Txid),
}

pub enum AckNews {
    Monitor(AckMonitorNews),
    Coordinator(AckCoordinatorNews),
}

pub type TransactionNewsType = MonitorNews;

pub use bitvmx_transaction_monitor::types::FullBlock;

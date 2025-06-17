use bitcoin::{Transaction, Txid};
use bitvmx_bitcoin_rpc::types::BlockHeight;
use bitvmx_transaction_monitor::types::{
    AckMonitorNews, BlockInfo, MonitorNews, TransactionBlockchainStatus,
};
use protocol_builder::types::Utxo;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum TransactionState {
    // The transaction is ready and queued to be sent.
    ToDispatch,

    // The transaction has been broadcast to the network and is waiting for confirmations.
    Dispatched,

    Confirmed,

    // The transaction has been successfully confirmed by the network.
    Finalized,

    // The transaction has failed to be broadcasted.
    Failed,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct CoordinatedTransaction {
    pub tx_id: Txid,
    pub tx: Transaction,
    // This is the utxo that will be used to pay for the transaction using CPFP (Child Pays For Parent)
    pub cpfp_utxo: Option<Utxo>,
    pub broadcast_block_height: Option<BlockHeight>,
    pub target_block_height: Option<BlockHeight>,
    pub state: TransactionState,
    pub context: String,
}

impl CoordinatedTransaction {
    pub fn new(
        tx: Transaction,
        cpfp_utxo: Option<Utxo>,
        state: TransactionState,
        target_block_height: Option<BlockHeight>,
        context: String,
    ) -> Self {
        Self {
            tx_id: tx.compute_txid(),
            tx,
            cpfp_utxo,
            broadcast_block_height: None,
            state,
            target_block_height,
            context,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct TransactionNew {
    pub tx_id: Txid,
    pub tx: Transaction,
    pub block_info: BlockInfo,
    pub confirmations: u32,
    pub status: TransactionBlockchainStatus,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum SpeedupState {
    Dispatched,
    Confirmed,
    Finalized,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct CoordinatedSpeedUpTransaction {
    pub tx_id: Txid,

    // The child tx ids that are being speeded up.
    pub child_tx_ids: Vec<Txid>,

    // The fee used when tx was sent.
    pub fee: u64,

    // The change funding utxo.
    pub funding: Utxo,

    // If true, this speed is is a replacement (RBF) for a previous speedup.
    // Otherwise, it is a new speedup (CPFP)
    pub is_replace_speedup: bool,

    pub broadcast_block_height: BlockHeight,

    pub state: SpeedupState,

    pub context: String,
}

#[allow(clippy::too_many_arguments)]
impl CoordinatedSpeedUpTransaction {
    pub fn new(
        tx_id: Txid,
        child_tx_ids: Vec<Txid>,
        fee: u64,
        funding: Utxo,
        is_replace_speedup: bool,
        broadcast_block_height: BlockHeight,
        state: SpeedupState,
        context: String,
    ) -> Self {
        Self {
            tx_id,
            child_tx_ids,
            fee,
            funding,
            is_replace_speedup,
            broadcast_block_height,
            state,
            context,
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
    /// - Txid: The transaction ID that failed to speed up
    /// - String: Context information about the transaction
    /// - Txid: The funding transaction ID that was insufficient
    /// - String: Error message describing what went wrong
    DispatchSpeedUpError(Vec<Txid>, Vec<String>, Txid, String),

    /// Indicates insufficient funds for a  funding transaction
    /// - Txid: The funding transaction ID that was insufficient
    InsufficientFunds(Txid),

    /// Indicates that there are no funds for a funding transaction
    FundingNotFound(),

    /// Notification of a new speed-up transaction
    /// - Txid: The transaction ID that was sped up
    /// - String: Context information about the transaction
    /// - u32: Counter indicating how many times this transaction has been sped up
    NewSpeedUp(Txid, String, u32),
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
    NewSpeedUp(Txid),
    DispatchTransactionError(Txid),
    DispatchSpeedUpError(Txid),
}

pub enum AckNews {
    Monitor(AckMonitorNews),
    Coordinator(AckCoordinatorNews),
}

pub type TransactionNewsType = MonitorNews;

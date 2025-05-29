use crate::{coordinator::BitcoinCoordinator, storage::BitcoinCoordinatorStore};
use bitcoin::{Transaction, Txid};
use bitvmx_bitcoin_rpc::{bitcoin_client::BitcoinClient, types::BlockHeight};
use bitvmx_transaction_monitor::types::{
    AckMonitorNews, BlockInfo, MonitorNews, MonitorType, TransactionBlockchainStatus,
};
use protocol_builder::types::Utxo;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum TransactionDispatchState {
    // The transaction is ready and queued to be sent.
    PendingDispatch,
    // The transaction has been broadcast to the network and is waiting for confirmations.
    BroadcastPendingConfirmation,
    // The transaction has been successfully confirmed by the network.
    Finalized,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct CoordinatedTransaction {
    pub tx_id: Txid,
    pub tx: Transaction,
    // This is the utxo that will be used to speed up the transaction
    pub speedup_utxo: Option<Utxo>,
    pub broadcast_block_height: Option<BlockHeight>,
    pub target_block_height: Option<BlockHeight>,
    pub state: TransactionDispatchState,
    pub context: String,
}

impl CoordinatedTransaction {
    pub fn new(
        tx: Transaction,
        speedup_utxo: Option<Utxo>,
        state: TransactionDispatchState,
        target_block_height: Option<BlockHeight>,
        context: String,
    ) -> Self {
        Self {
            tx_id: tx.compute_txid(),
            tx,
            speedup_utxo,
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
pub struct SpeedUpTx {
    pub tx_id: Txid,
    pub deliver_block_height: BlockHeight,
    pub child_tx_ids: Vec<Txid>,
    pub utxo: Utxo,
}

impl SpeedUpTx {
    pub fn new(
        tx_id: Txid,
        deliver_block_height: BlockHeight,
        child_tx_ids: Vec<Txid>,
        utxo: Utxo,
    ) -> Self {
        Self {
            tx_id,
            deliver_block_height,
            child_tx_ids,
            utxo,
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

pub type BitcoinCoordinatorType =
    BitcoinCoordinator<MonitorType, BitcoinCoordinatorStore, BitcoinClient>;

pub type TransactionNewsType = MonitorNews;

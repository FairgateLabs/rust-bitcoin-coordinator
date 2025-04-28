use bitcoin::{Amount, Transaction, TxOut, Txid};
use bitvmx_bitcoin_rpc::types::BlockHeight;
use bitvmx_transaction_monitor::types::{
    AckMonitorNews, BlockInfo, MonitorNews, MonitorType, TransactionBlockchainStatus,
};
use serde::{Deserialize, Serialize};
use transaction_dispatcher::DispatcherType;

use crate::{coordinator::BitcoinCoordinator, storage::BitcoinCoordinatorStore};

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct FundingTransaction {
    pub tx_id: Txid,
    pub utxo_index: u32,
    pub utxo_output: TxOut,
}

impl FundingTransaction {
    pub fn new(tx_id: Txid, utxo_index: u32, utxo_output: TxOut) -> Self {
        Self {
            tx_id,
            utxo_index,
            utxo_output,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum TransactionDispatchState {
    // The transaction is ready and queued to be sent.
    PendingDispatch,
    // The transaction has been broadcast to the network and is waiting for confirmations.
    BroadcastPendingConfirmation,
    // The transaction has been successfully confirmed by the network.
    Finalized,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct CoordinatedTransaction {
    pub tx_id: Txid,
    pub tx: Transaction,
    pub broadcast_block_height: Option<BlockHeight>,
    pub state: TransactionDispatchState,
    pub target_block_height: Option<BlockHeight>,
    pub context: String,
}

impl CoordinatedTransaction {
    pub fn new(
        tx: Transaction,
        state: TransactionDispatchState,
        target_block_height: Option<BlockHeight>,
        context: String,
    ) -> Self {
        Self {
            tx_id: tx.compute_txid(),
            tx,
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
    pub deliver_fee_rate: Amount,
    pub child_tx_id: Txid,
    pub utxo_index: u32,
    pub utxo_output: TxOut,
    //TODO: maybe we need to add status.
}

impl SpeedUpTx {
    pub fn new(
        tx_id: Txid,
        deliver_block_height: BlockHeight,
        deliver_fee_rate: Amount,
        child_tx_id: Txid,
        utxo_index: u32,
        utxo_output: TxOut,
    ) -> Self {
        Self {
            tx_id,
            deliver_block_height,
            deliver_fee_rate,
            child_tx_id,
            utxo_index,
            utxo_output,
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
    /// - String: Error message describing what went wrong
    DispatchSpeedUpError(Txid, String, String),

    /// Indicates insufficient funds for a transaction
    /// - Txid: The transaction ID that needs funds
    /// - String: Context information about the transaction
    /// - Txid: The funding transaction ID that was insufficient
    /// - String: Context information about the funding transaction
    InsufficientFunds(Txid, String, Txid, String),

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
    BitcoinCoordinator<MonitorType, DispatcherType, BitcoinCoordinatorStore>;

pub type TransactionNewsType = MonitorNews;

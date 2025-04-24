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
}

impl CoordinatedTransaction {
    pub fn new(
        tx: Transaction,
        state: TransactionDispatchState,
        target_block_height: Option<BlockHeight>,
    ) -> Self {
        Self {
            tx_id: tx.compute_txid(),
            tx,
            broadcast_block_height: None,
            state,
            target_block_height,
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

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct News {
    pub monitor_news: Vec<MonitorNews>,
    pub insufficient_funds: Vec<(Txid, String)>,
}

impl News {
    pub fn new(txs: Vec<MonitorNews>, insufficient_funds: Vec<(Txid, String)>) -> Self {
        Self {
            monitor_news: txs,
            insufficient_funds,
        }
    }
}

pub enum AckNews {
    Transaction(AckMonitorNews),
    InsufficientFunds(Txid),
    NewBlock,
}

pub type BitcoinCoordinatorType =
    BitcoinCoordinator<MonitorType, DispatcherType, BitcoinCoordinatorStore>;

pub type TransactionNewsType = MonitorNews;

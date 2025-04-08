use bitcoin::{Amount, Transaction, TxOut, Txid};
use bitvmx_bitcoin_rpc::types::BlockHeight;
use bitvmx_transaction_monitor::{
    store::TransactionMonitoredType,
    types::{
        AcknowledgeTransactionNews, BlockInfo, Id, MonitorType, TransactionBlockchainStatus,
        TransactionNews,
    },
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
pub enum TransactionState {
    // Represents a transaction that has been chosen by the protocol to be sent.
    ReadyToSend,
    // Represents a transaction that has been broadcast to the network and is waiting for confirmations.
    Sent,
    // Represents a transaction that has been successfully confirmed by the network but a reorganizacion move it out of the chain.
    Orphan,
    // Represents a transaction that has been successfully confirmed by the network
    Confirmed,
    // Represents when the transaction was confirmed an amount of blocks
    Finalized,
    // Represents a transaction that has been acknowledged or recognized by the system.
    Acknowledged,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct CoordinatedTransaction {
    pub group_id: Option<Id>,
    pub tx_id: Txid,
    pub tx: Transaction,
    pub deliver_block_height: Option<BlockHeight>,
    pub state: TransactionState,
}

impl CoordinatedTransaction {
    pub fn new(group_id: Option<Id>, tx: Transaction, state: TransactionState) -> Self {
        Self {
            group_id,
            tx_id: tx.compute_txid(),
            tx,
            deliver_block_height: None,
            state,
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

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum TransactionDispatch {
    GroupTransaction(Id, Transaction),
    SingleTransaction(Transaction),
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum TransactionFund {
    GroupTransaction(Id, FundingTransaction),
    SingleTransaction(Txid, FundingTransaction),
}

/// News represents new events that need to be processed
/// - instance_txs: New transactions found for specific instance IDs
/// - single_txs: Transactions are detected that are monitored by the system (for now pegins)
/// - funds_requests: Instance IDs that need additional funding
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct News {
    pub txs: Vec<TransactionNews>,
    pub funds_requests: Vec<Id>,
}

impl News {
    pub fn new(txs: Vec<TransactionNews>, funds_requests: Vec<Id>) -> Self {
        Self {
            txs,
            funds_requests,
        }
    }
}

pub enum AcknowledgeNews {
    Transaction(AcknowledgeTransactionNews),
    FundingRequest(Id),
}

pub type BitcoinCoordinatorType =
    BitcoinCoordinator<MonitorType, DispatcherType, BitcoinCoordinatorStore>;

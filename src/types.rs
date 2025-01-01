use bitcoin::{Address, Amount, Transaction, TxOut, Txid};
use bitvmx_transaction_monitor::types::{BlockHeight, BlockInfo};
use serde::{Deserialize, Serialize};

pub type InstanceId = u32;
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct FundingTx {
    pub tx_id: Txid,
    pub utxo_index: u32,
    pub utxo_output: TxOut,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum TransactionBlockchainStatus {
    // Represents a transaction that has been successfully confirmed by the network but a reorganizacion move it out of the chain.
    Orphan,
    // Represents a transaction that has been successfully confirmed by the network
    Confirmed,
    // Represents when the transaction was confirmed an amount of blocks
    Finalized,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum TransactionState {
    // Represents a transaction that is being monitored.
    New,
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
pub struct TransactionInfo {
    pub tx_id: Txid,
    // Represents the transaction itself, which is added when the transaction is ready to be sent.
    pub tx: Option<Transaction>,
    // Represents the hexadecimal representation of the transaction, which is added when the transaction is seen on the blockchain and confirmed.
    pub tx_hex: Option<String>,
    pub deliver_block_height: Option<BlockHeight>,
    pub state: TransactionState,
}

impl TransactionInfo {
    pub fn is_transaction_owned(&self) -> bool {
        // This method provides a simple way to determine if this transaction belongs to the current operator.
        // A transaction is considered owned if it has been sent by the operator.
        self.tx.is_some() && self.deliver_block_height.is_some()
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct TransactionNew {
    pub tx: Transaction,
    pub block_info: BlockInfo,
    pub confirmations: u32,
    pub status: TransactionBlockchainStatus,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct AddressNew {
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
    //TODO we need to add status.
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct TransactionPartialInfo {
    pub tx_id: Txid,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TransactionFullInfo {
    pub tx: Transaction,
}

//TODO Change the way we store data in the storage. BitvmxInstance should be different.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BitvmxInstance<T> {
    pub instance_id: InstanceId,
    pub txs: Vec<T>,
    // TODO: The instance could receive the txid of the funding or the transaction to be mined.
    // If the txid is sent, it is necessary to check that this transaction is mined.
    // On the other hand, if the transaction to be mined is sent, it is necessary to dispatch
    // this transaction and wait for it to be mined in order to start sending speed up transactions.
    pub funding_tx: FundingTx,
}

impl BitvmxInstance<TransactionFullInfo> {
    pub fn map_partial_info(&self) -> BitvmxInstance<TransactionPartialInfo> {
        let partial_info_txs = self
            .txs
            .iter()
            .map(|tx_info| TransactionPartialInfo {
                tx_id: tx_info.tx.compute_txid(),
            })
            .collect();

        BitvmxInstance::<TransactionPartialInfo> {
            instance_id: self.instance_id,
            txs: partial_info_txs,
            funding_tx: self.funding_tx.clone(),
        }
    }
}

/// News represents new events that need to be processed
/// - txs_by_id: New transactions found for specific instance IDs
/// - txs_by_address: New transactions found for monitored addresses
/// - funds_requests: Instance IDs that need additional funding
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct News {
    pub txs_by_id: Vec<(InstanceId, Vec<TransactionNew>)>,
    pub txs_by_address: Vec<(Address, Vec<AddressNew>)>,
    pub funds_requests: Vec<InstanceId>,
}
pub struct ProcessedNews {
    pub txs_by_id: Vec<(InstanceId, Vec<Txid>)>,
    pub txs_by_address: Vec<Address>,
    pub funds_requests: Vec<InstanceId>,
}

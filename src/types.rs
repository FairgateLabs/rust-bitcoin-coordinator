use bitcoin::{Amount, Transaction, TxOut, Txid};
use bitvmx_transaction_monitor::types::BlockHeight;
use serde::{Deserialize, Serialize};

pub type InstanceId = u32;
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct FundingTx {
    pub tx_id: Txid,
    pub utxo_index: u32,
    pub utxo_output: TxOut,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum TransactionStatus {
    // Represents a transaction that is being monitored.
    New,
    // Represents a transaction that has been chosen by the protocol to be sent.
    ReadyToSend,
    // Represents a transaction that has been broadcast to the network and is waiting for confirmations.
    Sent,
    // Represents a transaction that has been successfully confirmed by the network (confirmed: #blocks pass).
    Confirmed,
    // Represents a transaction that has been acknowledged or recognized by the system.
    Acknowledged,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct TransactionInfo {
    // Represents the transaction itself, which is added when the transaction is ready to be sent.
    pub tx: Option<Transaction>,
    // Represents the hexadecimal representation of the transaction, which is added when the transaction is seen on the blockchain and confirmed.
    pub tx_hex: Option<String>,
    pub tx_id: Txid,
    pub owner_operator_id: u32,
    pub deliver_block_height: Option<BlockHeight>,
    pub status: TransactionStatus,
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
    pub owner_operator_id: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TransactionFullInfo {
    pub tx: Transaction,
    pub owner_operator_id: u32,
}

//TODO Change the way we store data in the storage. BitvmxInstance should be different.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BitvmxInstance<T> {
    pub instance_id: InstanceId,
    pub txs: Vec<T>,
    pub funding_tx: FundingTx,
}

impl BitvmxInstance<TransactionFullInfo> {
    pub fn map_partial_info(&self) -> BitvmxInstance<TransactionPartialInfo> {
        let partial_info_txs = self
            .txs
            .iter()
            .map(|tx_info| TransactionPartialInfo {
                tx_id: tx_info.tx.compute_txid(),
                owner_operator_id: tx_info.owner_operator_id,
            })
            .collect();

        BitvmxInstance::<TransactionPartialInfo> {
            instance_id: self.instance_id,
            txs: partial_info_txs,
            funding_tx: self.funding_tx.clone(),
        }
    }
}

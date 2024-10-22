use bitcoin::{Amount, Transaction, TxOut, Txid};
use bitvmx_transaction_monitor::types::BlockHeight;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DeliverData {
    // Fee rate was used to send the transacion
    pub fee_rate: Amount,

    // Block height when transaction was sent
    pub block_height: BlockHeight,
}

pub type InstanceId = u32;
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct FundingTx {
    pub tx_id: Txid,
    pub utxo_index: u32,
    pub utxo_output: TxOut,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum TransactionStatus {
    // Indicates a transaction that is in a queue, awaiting dispatch by the protocol.
    Waiting,
    // Indicates a transaction that has been selected by the protocol for sending.
    Pending,
    // Indicates a transaction that has been sent and is currently awaiting confirmations.
    InProgress,
    // Indicates a transaction that has successfully completed and has received sufficient confirmations.
    Completed,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct TransactionInfo {
    pub tx: Option<Transaction>,
    pub tx_id: Txid,
    pub owner_operator_id: u32,
    pub deliver_data: Option<DeliverData>,
    pub status: TransactionStatus,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct SpeedUpTx {
    pub tx_id: Txid,
    pub deliver_data: DeliverData,
    pub child_tx_id: Txid,
    pub utxo_index: u32,
    pub utxo_output: TxOut,
    //TODO we need to add status.
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TransactionInfoSummary {
    pub tx_id: Txid,
    pub owner_operator_id: u32,
}

//TODO Change the way we store data in the storage. BitvmxInstance should be different.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct BitvmxInstance<T> {
    pub instance_id: InstanceId,
    pub txs: Vec<T>,
    pub funding_tx: FundingTx,
}

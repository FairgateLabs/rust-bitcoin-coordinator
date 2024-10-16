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
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct FundingTx {
    pub tx_id: Txid,
    pub utxo_index: u32,
    pub utxo_output: TxOut,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct TransactionInfo {
    pub tx: Option<Transaction>,
    pub tx_id: Txid,
    pub owner_operator_id: u32,
    pub deliver_data: Option<DeliverData>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct SpeedUpTx {
    pub tx_id: Txid,
    pub deliver_data: DeliverData,
    pub child_tx_id: Txid,
    pub utxo_index: u32,
    pub utxo_output: TxOut,
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

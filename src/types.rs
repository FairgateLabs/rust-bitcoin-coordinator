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
pub struct SpeedUpData {
    pub is_speed_up_tx: bool,
    pub child_tx_id: Txid,
    pub utxo_index: u32,
    pub utxo_output: TxOut,
}
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct TransactionInstance {
    // Transaction will be added when is send.
    pub tx: Option<Transaction>,
    pub tx_id: Txid,
    pub owner_operator_id: u32,
    pub deliver_data: Option<DeliverData>,
    pub speed_up_data: Option<SpeedUpData>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TransactionInstanceSummary {
    pub tx_id: Txid,
    pub owner_operator_id: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct BitvmxInstance<T> {
    pub instance_id: InstanceId,
    pub txs: Vec<T>,
    pub funding_tx: FundingTx,
}

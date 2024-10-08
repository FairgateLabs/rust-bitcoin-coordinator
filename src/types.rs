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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct InProgressSpeedUpTx {
    // Information about dispatch tx
    pub deliver_data: DeliverData,

    pub child_tx_id: Txid,
}

pub type InstanceId = u32;
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct InProgressTx {
    // main transaction
    pub tx_id: Transaction,

    // Information about send tx
    pub deliver_data: DeliverData,

    // Stores information about the transaction child to speed up the main txs.
    pub speed_up_txs: Vec<InProgressSpeedUpTx>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct FundingTx {
    pub child_tx_id: Txid,
    pub utxo_index: u32,
    pub utxo_output: TxOut,
}
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TxInstance {
    pub tx_id: Txid,
    pub owner: bool,
}
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct BitvmxInstance {
    pub instance_id: InstanceId,
    pub txs: Vec<TxInstance>,
    pub funding_tx: FundingTx,
}

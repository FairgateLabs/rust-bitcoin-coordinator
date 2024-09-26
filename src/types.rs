use bitcoin::{Amount, Transaction, TxOut, Txid};
use bitvmx_transaction_monitor::types::BlockHeight;
use serde::{Deserialize, Serialize};

pub type InstanceId = u32;
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PendingTx {
    pub tx: Transaction,
    pub fee_rate: Amount,
    pub block_height: BlockHeight,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct FundingTx {
    pub tx_id: Txid,
    pub utxo_index: u32,
    pub utxo_output: TxOut,
}

/*
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct BitvmxInstance {
    pub instance_id: InstanceId,
    pub txs: Vec<FundingTx>,
    pub start_height: BlockHeight,
}
*/

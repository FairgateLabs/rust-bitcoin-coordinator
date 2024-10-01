use bitcoin::{Amount, Transaction, TxOut, Txid};
use bitvmx_transaction_monitor::types::BlockHeight;
use serde::{Deserialize, Serialize};

pub type InstanceId = u32;
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct InProgressTx {
    pub tx: Transaction,

    //Fee rate was used to send the transacion
    pub fee_rate: Amount,

    // Block height when transaction was sent
    pub block_height: BlockHeight,

    // If transaction was speed up then we save that information
    pub was_speed_up: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct FundingTx {
    pub tx_id: Txid,
    pub utxo_index: u32,
    pub utxo_output: TxOut,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct BitvmxInstance {
    pub instance_id: InstanceId,
    pub txs: Vec<Txid>,
    pub funding_tx: FundingTx,
}

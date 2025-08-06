use bitcoin::{Transaction, Txid};
use bitvmx_bitcoin_rpc::types::BlockHeight;
use bitvmx_transaction_monitor::types::{
    AckMonitorNews, BlockInfo, MonitorNews, TransactionBlockchainStatus,
};
use protocol_builder::types::{output::SpeedupData, Utxo};
use serde::{Deserialize, Serialize};

use crate::settings::{
    CPFP_TRANSACTION_CONTEXT, FUNDING_TRANSACTION_CONTEXT, RBF_TRANSACTION_CONTEXT,
};

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum TransactionState {
    // The transaction is ready and queued to be sent.
    ToDispatch,

    // The transaction has been broadcast to the network and is waiting for confirmations.
    Dispatched,

    Confirmed,

    // The transaction has been successfully confirmed by the network.
    Finalized,

    // The transaction has failed to be broadcasted.
    Failed,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct CoordinatedTransaction {
    pub tx_id: Txid,
    pub tx: Transaction,
    // This is the utxo that will be used to pay for the transaction using CPFP (Child Pays For Parent)
    pub speedup_data: Option<SpeedupData>,
    pub broadcast_block_height: Option<BlockHeight>,
    pub target_block_height: Option<BlockHeight>,
    pub state: TransactionState,
    pub context: String,
}

impl CoordinatedTransaction {
    pub fn new(
        tx: Transaction,
        speedup_data: Option<SpeedupData>,
        state: TransactionState,
        target_block_height: Option<BlockHeight>,
        context: String,
    ) -> Self {
        Self {
            tx_id: tx.compute_txid(),
            tx,
            speedup_data,
            broadcast_block_height: None,
            state,
            target_block_height,
            context,
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
pub enum SpeedupState {
    Dispatched,
    Confirmed,
    Finalized,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct CoordinatedSpeedUpTransaction {
    pub tx_id: Txid,

    // The previous funding utxo.
    pub prev_funding: Utxo,

    // The change funding utxo.
    pub next_funding: Utxo,

    // If true, this speedup is a replacement (RBF) for a previous speedup.
    // Otherwise, it is a new speedup (CPFP)
    pub is_rbf: bool,

    pub broadcast_block_height: BlockHeight,

    pub state: SpeedupState,

    pub context: String,

    pub bump_fee_percentage_used: f64,

    pub speedup_tx_data: Vec<(SpeedupData, Transaction)>,

    pub network_fee_rate_used: u64,
}

#[allow(clippy::too_many_arguments)]
impl CoordinatedSpeedUpTransaction {
    pub fn new(
        tx_id: Txid,
        prev_funding: Utxo,
        next_funding: Utxo,
        is_rbf: bool,
        broadcast_block_height: BlockHeight,
        state: SpeedupState,
        bump_fee_percentage_used: f64,
        speedup_tx_data: Vec<(SpeedupData, Transaction)>,
        network_fee_used: u64,
    ) -> Self {
        let mut context = if is_rbf {
            RBF_TRANSACTION_CONTEXT.to_string()
        } else {
            CPFP_TRANSACTION_CONTEXT.to_string()
        };

        if broadcast_block_height == 0
            && state == SpeedupState::Finalized
            && speedup_tx_data.is_empty()
        {
            context = FUNDING_TRANSACTION_CONTEXT.to_string();
        }

        Self {
            tx_id,
            prev_funding,
            next_funding,
            is_rbf,
            broadcast_block_height,
            state,
            context,
            bump_fee_percentage_used,
            speedup_tx_data,
            network_fee_rate_used: network_fee_used,
        }
    }
}

impl CoordinatedSpeedUpTransaction {
    pub fn is_funding(&self) -> bool {
        self.broadcast_block_height == 0
            && self.state == SpeedupState::Finalized
            && self.speedup_tx_data.is_empty()
    }

    pub fn is_rbf(&self) -> bool {
        self.is_rbf
    }

    pub fn get_tx_name(&self) -> String {
        if self.is_funding() {
            "FUNDING".to_string()
        } else if self.is_rbf() {
            "RBF".to_string()
        } else {
            "CPFP".to_string()
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TransactionFullInfo {
    pub tx: Transaction,
}

#[derive(Debug, Clone, PartialEq)]
pub struct News {
    pub monitor_news: Vec<MonitorNews>,
    pub coordinator_news: Vec<CoordinatorNews>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoordinatorNews {
    /// Error when dispatching a transaction
    /// - Txid: The transaction ID that failed to dispatch
    /// - String: Context information about the transaction
    /// - String: Error message describing what went wrong
    DispatchTransactionError(Txid, String, String),

    /// Error when attempting to speed up a transaction
    /// - Txid: The transaction ID that failed to speed up
    /// - String: Context information about the transaction
    /// - Txid: The funding transaction ID that was insufficient
    /// - String: Error message describing what went wrong
    DispatchSpeedUpError(Vec<Txid>, Vec<String>, Txid, String),

    /// Indicates insufficient funds for a funding transaction
    /// - Txid: The funding transaction ID that was insufficient
    /// - u64: The available funding amount
    /// - u64: The amount required for a speedup
    InsufficientFunds(Txid, u64, u64),

    /// Indicates that there are no funding utxo loaded
    FundingNotFound,

    /// Notification of a new speed-up transaction
    /// - Txid: The transaction ID that was sped up
    /// - String: Context information about the transaction
    /// - u32: Counter indicating how many times this transaction has been sped up
    NewSpeedUp(Txid, String, u32),

    /// Indicates that the estimate feerate is too high
    /// - u64: The estimate feerate from the node
    /// - u64: The max allowed feerate from settings
    EstimateFeerateTooHigh(u64, u64),
}

impl News {
    pub fn new(monitor_news: Vec<MonitorNews>, coordinator_news: Vec<CoordinatorNews>) -> Self {
        Self {
            monitor_news,
            coordinator_news,
        }
    }
}

pub enum AckCoordinatorNews {
    InsufficientFunds(Txid),
    NewSpeedUp(Txid),
    DispatchTransactionError(Txid),
    DispatchSpeedUpError(Txid),
    EstimateFeerateTooHigh(u64, u64),
    FundingNotFound,
}

pub enum AckNews {
    Monitor(AckMonitorNews),
    Coordinator(AckCoordinatorNews),
}

pub type TransactionNewsType = MonitorNews;

pub use bitvmx_transaction_monitor::types::FullBlock;

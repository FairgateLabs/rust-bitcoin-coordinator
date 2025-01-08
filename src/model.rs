use std::fmt;

use bitcoin::Txid;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub enum TransactionDispatcherModel {
    Transaction,
    DispatcherTask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatcherTask {
    pub transaction_id: Txid,
    pub child_tx: Option<Txid>,
    pub kind: DispatcherTaskKind,
    pub status: DispatcherTaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DispatcherTaskKind {
    Send,
    Speedup,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DispatcherTaskStatus {
    None,
    Sent,
    Confirmed,
    Error,
}

impl fmt::Display for DispatcherTaskKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                DispatcherTaskKind::Send => "Send",
                DispatcherTaskKind::Speedup => "Speed Up",
            }
        )
    }
}

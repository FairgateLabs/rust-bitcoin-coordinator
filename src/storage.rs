use crate::{
    errors::BitcoinCoordinatorStoreError,
    types::{CoordinatedTransaction, FundingTransaction, SpeedUpTx, TransactionDispatchState},
};

use bitcoin::{Transaction, Txid};
use bitvmx_bitcoin_rpc::types::BlockHeight;
use mockall::automock;
use std::rc::Rc;
use storage_backend::storage::{KeyValueStore, Storage};
use uuid::Uuid;
pub struct BitcoinCoordinatorStore {
    store: Rc<Storage>,
}
enum StoreKey {
    Transaction(Txid),
    TransactionList,
    TransactionFundingId(Txid),
    FundingTransactions(Uuid),
    TransactionSpeedUpList(Txid),
    FundingRequestList,
}

#[automock]
pub trait BitcoinCoordinatorStoreApi {
    fn save_tx(
        &self,
        tx: Transaction,
        target_block_height: Option<BlockHeight>,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn remove_tx(&self, tx_id: Txid) -> Result<(), BitcoinCoordinatorStoreError>;

    fn get_txs(
        &self,
        state: TransactionDispatchState,
    ) -> Result<Vec<CoordinatedTransaction>, BitcoinCoordinatorStoreError>;

    fn update_tx(
        &self,
        tx_id: Txid,
        status: TransactionDispatchState,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn update_tx_to_dispatched(
        &self,
        tx_id: Txid,
        deliver_block_height: u32,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn save_speedup_tx(&self, speed_up_tx: &SpeedUpTx) -> Result<(), BitcoinCoordinatorStoreError>;

    fn get_last_speedup_tx(
        &self,
        child_tx_id: &Txid,
    ) -> Result<Option<SpeedUpTx>, BitcoinCoordinatorStoreError>;

    fn get_speedup_tx(
        &self,
        child_tx_id: &Txid,
        tx_id: &Txid,
    ) -> Result<SpeedUpTx, BitcoinCoordinatorStoreError>;

    fn get_funding(
        &self,
        tx_id: Txid,
    ) -> Result<Option<FundingTransaction>, BitcoinCoordinatorStoreError>;

    fn add_funding(
        &self,
        tx_ids: Vec<Txid>,
        funding_tx: FundingTransaction,
        context: String,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn remove_funding(
        &self,
        funding_tx_id: Txid,
        tx_id: Txid,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn update_funding(
        &self,
        child_tx_id: Txid,
        funding_tx: FundingTransaction,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn add_insufficient_funds_news(&self, tx_id: Txid) -> Result<(), BitcoinCoordinatorStoreError>;
    fn ack_insufficient_funds_news(&self, tx_id: Txid) -> Result<(), BitcoinCoordinatorStoreError>;
    fn get_insufficient_funds_news(
        &self,
    ) -> Result<Vec<(Txid, String)>, BitcoinCoordinatorStoreError>;
}

impl BitcoinCoordinatorStore {
    pub fn new(store: Rc<Storage>) -> Result<Self, BitcoinCoordinatorStoreError> {
        Ok(Self { store })
    }

    fn get_key(&self, key: StoreKey) -> String {
        let prefix = "bitcoin_coordinator";
        match key {
            StoreKey::TransactionList => format!("{prefix}/tx/list"),
            StoreKey::Transaction(tx_id) => format!("{prefix}/tx/{tx_id}"),
            StoreKey::TransactionFundingId(tx_id) => {
                // Given a tx_id, we can get the funding transactions
                format!("{prefix}/tx/{tx_id}/funding")
            }
            StoreKey::FundingTransactions(group_id) => {
                format!("{prefix}/tx/{group_id}/funding/txs/list")
            }
            StoreKey::TransactionSpeedUpList(tx_id) => {
                format!("{prefix}/speedup/{tx_id}/list")
            }
            StoreKey::FundingRequestList => format!("{prefix}/funding/request/list"),
        }
    }

    fn get_txs(&self) -> Result<Vec<Txid>, BitcoinCoordinatorStoreError> {
        let key = self.get_key(StoreKey::TransactionList);

        let all_txs = self.store.get::<&str, Vec<Txid>>(&key)?;

        match all_txs {
            Some(txs) => Ok(txs),
            None => Ok(vec![]),
        }
    }

    fn get_tx(&self, tx_id: Txid) -> Result<CoordinatedTransaction, BitcoinCoordinatorStoreError> {
        let key = self.get_key(StoreKey::Transaction(tx_id));
        let tx = self.store.get::<&str, CoordinatedTransaction>(&key)?;

        if tx.is_none() {
            let message = format!("Transaction not found: {}", tx_id);
            return Err(BitcoinCoordinatorStoreError::TransactionNotFound(message));
        }

        Ok(tx.unwrap())
    }

    fn get_fundings_key(&self, tx_id: Txid) -> Result<Option<Uuid>, BitcoinCoordinatorStoreError> {
        let id_key = self.get_key(StoreKey::TransactionFundingId(tx_id));
        let fundings_id = self.store.get::<&str, (String, Uuid)>(&id_key)?;

        if fundings_id.is_none() {
            return Ok(None);
        }

        Ok(Some(fundings_id.unwrap().1))
    }
}

impl BitcoinCoordinatorStoreApi for BitcoinCoordinatorStore {
    fn get_txs(
        &self,
        state: TransactionDispatchState,
    ) -> Result<Vec<CoordinatedTransaction>, BitcoinCoordinatorStoreError> {
        let txs = self.get_txs()?;
        let mut txs_filter = Vec::new();

        for tx_id in txs {
            let tx = self.get_tx(tx_id)?;

            if tx.state == state {
                txs_filter.push(tx);
            }
        }

        Ok(txs_filter)
    }
    fn save_tx(
        &self,
        tx: Transaction,
        target_block_height: Option<BlockHeight>,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let key = self.get_key(StoreKey::Transaction(tx.compute_txid()));

        let tx_info = CoordinatedTransaction::new(
            tx.clone(),
            TransactionDispatchState::PendingDispatch,
            target_block_height,
        );

        self.store.set(&key, &tx_info, None)?;

        let txs_key = self.get_key(StoreKey::TransactionList);
        let mut txs = self
            .store
            .get::<&str, Vec<Txid>>(&txs_key)?
            .unwrap_or_default();
        txs.push(tx.compute_txid());
        self.store.set(&txs_key, &txs, None)?;

        Ok(())
    }

    fn remove_tx(&self, tx_id: Txid) -> Result<(), BitcoinCoordinatorStoreError> {
        let tx_key = self.get_key(StoreKey::Transaction(tx_id));
        self.store.delete(&tx_key)?;

        let txs_key = self.get_key(StoreKey::TransactionList);
        let mut txs = self
            .store
            .get::<&str, Vec<Txid>>(&txs_key)?
            .unwrap_or_default();

        txs.retain(|id| *id != tx_id);
        self.store.set(&txs_key, &txs, None)?;

        Ok(())
    }

    fn get_funding(
        &self,
        tx_id: Txid,
    ) -> Result<Option<FundingTransaction>, BitcoinCoordinatorStoreError> {
        let funding_txs_id = self.get_fundings_key(tx_id)?;

        if funding_txs_id.is_none() {
            return Ok(None);
        }

        let funding_txs_key = self.get_key(StoreKey::FundingTransactions(funding_txs_id.unwrap()));
        let funding_txs = self
            .store
            .get::<&str, Vec<FundingTransaction>>(&funding_txs_key)?
            .unwrap_or_default();

        if let Some(last_funding_tx) = funding_txs.last() {
            // Funding transaction is the last one.
            Ok(Some(last_funding_tx.clone()))
        } else {
            Ok(None)
        }
    }

    fn add_funding(
        &self,
        tx_ids: Vec<Txid>,
        funding_tx: FundingTransaction,
        context: String,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let new_funding_txs_id = Uuid::new_v4(); // This id represent the array of funding transactions

        for tx_id in tx_ids.clone() {
            // For each transaction, we need to set the funding id
            let id_key = self.get_key(StoreKey::TransactionFundingId(tx_id));
            self.store
                .set(id_key, (context.clone(), new_funding_txs_id), None)?;
        }

        let fundings_txs_key = self.get_key(StoreKey::FundingTransactions(new_funding_txs_id));

        let mut funding_info = self
            .store
            .get::<&str, Vec<FundingTransaction>>(&fundings_txs_key)?
            .unwrap_or_default();

        funding_info.push(funding_tx);

        self.store.set(&fundings_txs_key, &funding_info, None)?;

        Ok(())
    }

    fn remove_funding(
        &self,
        funding_tx_id: Txid,
        tx_id: Txid,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let funding_txs_id = self.get_fundings_key(tx_id)?;

        if funding_txs_id.is_none() {
            return Err(BitcoinCoordinatorStoreError::FundingTransactionNotFound);
        }

        let fundings_txs_key = self.get_key(StoreKey::FundingTransactions(funding_txs_id.unwrap()));

        let mut funding_txs = self
            .store
            .get::<&str, Vec<FundingTransaction>>(&fundings_txs_key)?
            .unwrap_or_default();

        funding_txs.retain(|tx| tx.tx_id != funding_tx_id);

        self.store.set(&fundings_txs_key, &funding_txs, None)?;

        Ok(())
    }

    fn update_funding(
        &self,
        child_tx_id: Txid,
        funding_tx: FundingTransaction,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let funding_txs_id = self.get_fundings_key(child_tx_id)?;

        if funding_txs_id.is_none() {
            return Err(BitcoinCoordinatorStoreError::FundingTransactionNotFound);
        }

        let fundings_txs_key = self.get_key(StoreKey::FundingTransactions(funding_txs_id.unwrap()));

        let mut funding_txs = self
            .store
            .get::<&str, Vec<FundingTransaction>>(&fundings_txs_key)?
            .unwrap_or_default();

        // Check if the funding transaction already exists to avoid duplicates
        if funding_txs.iter().any(|tx| tx.tx_id == funding_tx.tx_id) {
            return Err(BitcoinCoordinatorStoreError::FundingTransactionAlreadyExists);
        }

        // Remove the existing funding transaction before adding the updated one
        funding_txs.retain(|tx| tx.tx_id != funding_tx.tx_id);
        funding_txs.push(funding_tx);

        self.store.set(&fundings_txs_key, &funding_txs, None)?;

        Ok(())
    }

    fn get_speedup_tx(
        &self,
        child_tx_id: &Txid,
        tx_id: &Txid,
    ) -> Result<SpeedUpTx, BitcoinCoordinatorStoreError> {
        let speed_up_tx_key = self.get_key(StoreKey::TransactionSpeedUpList(*child_tx_id));

        // Retrieve the list of speed up transactions from storage
        let speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)?
            .unwrap_or_default();

        // Find the specific speed up transaction that matches the given tx_id
        let speed_up_tx = speed_up_txs.into_iter().find(|t| t.tx_id == *tx_id);

        if speed_up_tx.is_none() {
            return Err(BitcoinCoordinatorStoreError::SpeedUpTransactionNotFound);
        }

        Ok(speed_up_tx.unwrap())
    }

    fn get_last_speedup_tx(
        &self,
        child_tx_id: &Txid,
    ) -> Result<Option<SpeedUpTx>, BitcoinCoordinatorStoreError> {
        let speed_up_tx_key = self.get_key(StoreKey::TransactionSpeedUpList(*child_tx_id));

        // Retrieve the list of speed up transactions from storage
        let speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)?
            .unwrap_or_default();

        // Get the last speed up transaction from the list
        let speed_up_tx = speed_up_txs.into_iter().last();

        Ok(speed_up_tx)
    }

    // This function adds a new speed up transaction to the list of speed up transactions associated with an child transaction.
    // Speed up transactions are stored in a list, with the most recent transaction added to the end of the list.
    // This design ensures that if the last transaction in the list is pending, there cannot be another pending speed up transaction
    // for the same group of transactions, except for one that is specifically related to the same child transaction.
    fn save_speedup_tx(&self, speed_up_tx: &SpeedUpTx) -> Result<(), BitcoinCoordinatorStoreError> {
        let speed_up_tx_key =
            self.get_key(StoreKey::TransactionSpeedUpList(speed_up_tx.child_tx_id));

        let mut speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)?
            .unwrap_or_default();

        speed_up_txs.push(speed_up_tx.clone());

        self.store.set(&speed_up_tx_key, &speed_up_txs, None)?;

        Ok(())
    }

    fn update_tx_to_dispatched(
        &self,
        tx_id: Txid,
        deliver_block_height: u32,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        //TODO: Implement transaction status transition validation to ensure the correct sequence:
        // Pending -> InProgress -> Completed, and in reorganization scenarios, do the reverse order.

        let mut tx = self.get_tx(tx_id)?;

        if tx.state != TransactionDispatchState::PendingDispatch {
            return Err(BitcoinCoordinatorStoreError::InvalidTransactionState);
        }

        tx.state = TransactionDispatchState::BroadcastPendingConfirmation;

        tx.broadcast_block_height = Some(deliver_block_height);

        let key = self.get_key(StoreKey::Transaction(tx_id));
        self.store.set(key, tx, None)?;

        Ok(())
    }

    fn update_tx(
        &self,
        tx_id: Txid,
        tx_state: TransactionDispatchState,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        //TODO: Implement transaction status transition validation to ensure the correct sequence:
        // PendingDispatch -> BroadcastPendingConfirmation -> Finalized, and in reorganization scenarios, do the reverse order.

        let mut tx = self.get_tx(tx_id)?;
        tx.state = tx_state.clone();

        let key = self.get_key(StoreKey::Transaction(tx_id));
        self.store.set(key, tx, None)?;

        Ok(())
    }

    fn add_insufficient_funds_news(&self, tx_id: Txid) -> Result<(), BitcoinCoordinatorStoreError> {
        let fundings = self.get_fundings_key(tx_id)?;

        if fundings.is_none() {
            return Err(BitcoinCoordinatorStoreError::FundingTransactionNotFound);
        }

        let fundings_txs_key = self.get_key(StoreKey::FundingTransactions(fundings.unwrap()));
        let fundings_data = self
            .store
            .get::<&str, (String, Vec<FundingTransaction>)>(&fundings_txs_key)?
            .unwrap_or_default();

        let context = fundings_data.0;

        let funding_request_key = self.get_key(StoreKey::FundingRequestList);
        let mut funding_requests = self
            .store
            .get::<&str, Vec<(Txid, String)>>(&funding_request_key)?
            .unwrap_or_default();

        funding_requests.push((tx_id, context));

        self.store
            .set(&funding_request_key, &funding_requests, None)?;
        Ok(())
    }

    fn ack_insufficient_funds_news(&self, tx_id: Txid) -> Result<(), BitcoinCoordinatorStoreError> {
        let funding_request_key = self.get_key(StoreKey::FundingRequestList);
        let mut funding_requests = self
            .store
            .get::<&str, Vec<(Txid, String)>>(&funding_request_key)?
            .unwrap_or_default();

        funding_requests.retain(|(id, _)| *id != tx_id);
        self.store
            .set(&funding_request_key, &funding_requests, None)?;
        Ok(())
    }

    fn get_insufficient_funds_news(
        &self,
    ) -> Result<Vec<(Txid, String)>, BitcoinCoordinatorStoreError> {
        let funding_request_key = self.get_key(StoreKey::FundingRequestList);
        let funding_requests = self
            .store
            .get::<&str, Vec<(Txid, String)>>(&funding_request_key)?
            .unwrap_or_default();
        Ok(funding_requests)
    }
}

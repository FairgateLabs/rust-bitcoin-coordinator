use crate::{
    errors::BitcoinCoordinatorStoreError,
    types::{CoordinatedTransaction, FundingTransaction, SpeedUpTx, TransactionState},
};

use bitcoin::{Transaction, Txid};
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
    fn save_tx(&self, tx: Transaction) -> Result<(), BitcoinCoordinatorStoreError>;

    fn get_tx(
        &self,
        state: TransactionState,
    ) -> Result<Vec<CoordinatedTransaction>, BitcoinCoordinatorStoreError>;

    fn update_tx(
        &self,
        tx_id: Txid,
        status: TransactionState,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    // SPEED UP TRANSACTIONS
    fn save_speedup_tx(&self, speed_up_tx: &SpeedUpTx) -> Result<(), BitcoinCoordinatorStoreError>;

    fn get_last_speedup_tx(
        &self,
        child_tx_id: &Txid,
    ) -> Result<Option<SpeedUpTx>, BitcoinCoordinatorStoreError>;

    fn get_speedup_tx(
        &self,
        child_tx_id: &Txid,
        tx_id: &Txid,
    ) -> Result<Option<SpeedUpTx>, BitcoinCoordinatorStoreError>;

    // FUNDING TRANSACTIONS
    // Funding transactions are used to provide capital to speed-up transactions
    // when fee acceleration is needed
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
        tx_id: Txid,
        child_txid: Txid,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn update_funding(
        &self,
        tx_id: Txid,
        funding_tx: FundingTransaction,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    // FUNDING TRANSACTIONS REQUESTS
    // Funding requests are created when an instance run out off funds
    // and requires additional funding to speed up transactions
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

    fn get_fundings_key(&self, tx_id: Txid) -> Result<Uuid, BitcoinCoordinatorStoreError> {
        let id_key = self.get_key(StoreKey::TransactionFundingId(tx_id));
        let fundings_id = self.store.get::<&str, Uuid>(&id_key)?;

        match fundings_id {
            Some(id) => Ok(id),
            None => Err(BitcoinCoordinatorStoreError::FundingKeyNotFound),
        }
    }
}

impl BitcoinCoordinatorStoreApi for BitcoinCoordinatorStore {
    fn get_tx(
        &self,
        state: TransactionState,
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
    fn save_tx(&self, tx: Transaction) -> Result<(), BitcoinCoordinatorStoreError> {
        let key = self.get_key(StoreKey::Transaction(tx.compute_txid()));

        let tx_info = CoordinatedTransaction::new(tx.clone(), TransactionState::ReadyToSend);

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

    fn get_funding(
        &self,
        tx_id: Txid,
    ) -> Result<Option<FundingTransaction>, BitcoinCoordinatorStoreError> {
        let funding_txs_id = self.get_fundings_key(tx_id)?;

        let funding_txs_key = self.get_key(StoreKey::FundingTransactions(funding_txs_id));

        let funding_txs = self
            .store
            .get::<&str, (String, Vec<FundingTransaction>)>(&funding_txs_key)?
            .unwrap_or_default();

        if let Some(last_funding_tx) = funding_txs.1.last() {
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
        let new_funding_txs_id = Uuid::new_v4();

        for tx_id in tx_ids.clone() {
            // For each transaction, we need to set the funding id
            let id_key = self.get_key(StoreKey::TransactionFundingId(tx_id));
            self.store.set(id_key, new_funding_txs_id, None)?;
        }

        let fundings_txs_key = self.get_key(StoreKey::FundingTransactions(new_funding_txs_id));

        let mut funding_txs = self
            .store
            .get::<&str, (String, Vec<FundingTransaction>)>(&fundings_txs_key)?
            .unwrap_or_default();

        funding_txs.1.push(funding_tx);

        self.store
            .set(&fundings_txs_key, (context, funding_txs), None)?;

        Ok(())
    }

    fn remove_funding(
        &self,
        tx_id: Txid,
        child_txid: Txid,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let funding_txs_id = self.get_fundings_key(child_txid)?;

        let fundings_txs_key = self.get_key(StoreKey::FundingTransactions(funding_txs_id));

        let mut funding_txs = self
            .store
            .get::<&str, (String, Vec<FundingTransaction>)>(&fundings_txs_key)?
            .unwrap_or_default();

        if funding_txs.1.is_empty() || funding_txs.1.last().unwrap().tx_id != tx_id {
            return Err(BitcoinCoordinatorStoreError::FundingTransactionNotFound);
        }

        funding_txs.1.retain(|tx| tx.tx_id != tx_id);

        self.store.set(&fundings_txs_key, &funding_txs, None)?;

        Ok(())
    }

    fn update_funding(
        &self,
        tx_id: Txid,
        funding_tx: FundingTransaction,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let funding_txs_id = self.get_fundings_key(tx_id)?;

        let fundings_txs_key = self.get_key(StoreKey::FundingTransactions(funding_txs_id));

        let mut funding_txs = self
            .store
            .get::<&str, (String, Vec<FundingTransaction>)>(&fundings_txs_key)?
            .unwrap_or_default();

        funding_txs.1.push(funding_tx);

        self.store.set(&fundings_txs_key, &funding_txs, None)?;

        Ok(())
    }

    fn get_speedup_tx(
        &self,
        tx_id: &Txid,
        child_tx_id: &Txid,
    ) -> Result<Option<SpeedUpTx>, BitcoinCoordinatorStoreError> {
        let speed_up_tx_key = self.get_key(StoreKey::TransactionSpeedUpList(*child_tx_id));

        // Retrieve the list of speed up transactions from storage
        let speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)?
            .unwrap_or_default();

        // Find the specific speed up transaction that matches the given tx_id
        let speed_up_tx = speed_up_txs.into_iter().find(|t| t.tx_id == *tx_id);

        Ok(speed_up_tx)
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

    // This function adds a new speed up transaction to the list of speed up transactions associated with an instance.
    // Speed up transactions are stored in a list, with the most recent transaction added to the end of the list.
    // This design ensures that if the last transaction in the list is pending, there cannot be another pending speed up transaction
    // for the same instance, except for one that is specifically related to the same child transaction.
    fn save_speedup_tx(&self, speed_up_tx: &SpeedUpTx) -> Result<(), BitcoinCoordinatorStoreError> {
        let speed_up_tx_key =
            self.get_key(StoreKey::TransactionSpeedUpList(speed_up_tx.child_tx_id));

        // Retrieve the current list of speed up transactions for the instance from storage.
        let mut speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)
            .map_err(|e| {
                BitcoinCoordinatorStoreError::BitcoinCoordinatorStoreError(
                    "Failed to retrieve funding transaction".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();

        // Add the newly created speed up transaction to the end of the list.
        speed_up_txs.push(speed_up_tx.clone());

        // Save the updated list of speed up transactions back to storage.
        self.store.set(&speed_up_tx_key, &speed_up_txs, None)?;

        Ok(())
    }

    fn update_tx(
        &self,
        tx_id: Txid,
        tx_state: TransactionState,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        //TODO: Implement transaction status transition validation to ensure the correct sequence:
        // Pending -> InProgress -> Completed, and in reorganization scenarios, do the reverse order.

        let mut tx = self.get_tx(tx_id)?;
        tx.state = tx_state;

        let key = self.get_key(StoreKey::Transaction(tx_id));
        self.store.set(key, tx, None)?;

        Ok(())
    }

    fn add_insufficient_funds_news(&self, tx_id: Txid) -> Result<(), BitcoinCoordinatorStoreError> {
        let fundings = self.get_fundings_key(tx_id)?;
        let fundings_txs_key = self.get_key(StoreKey::FundingTransactions(fundings));
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

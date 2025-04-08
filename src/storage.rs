use crate::{
    errors::BitcoinCoordinatorStoreError,
    types::{
        CoordinatedTransaction, FundingTransaction, SpeedUpTx, TransactionFund, TransactionState,
    },
};

use bitcoin::{Transaction, Txid};
use bitvmx_bitcoin_rpc::types::BlockHeight;
use bitvmx_transaction_monitor::types::{Id, TransactionMonitor};
use mockall::automock;
use std::rc::Rc;
use storage_backend::storage::{KeyValueStore, Storage};
pub struct BitcoinCoordinatorStore {
    store: Rc<Storage>,
}
enum StoreKey {
    Transaction(Txid),
    TransactionList,
    TransactionFundingList(Id),
    TransactionSpeedUpList(String),
    FundingRequestList,
}

#[automock]
pub trait BitcoinCoordinatorStoreApi {
    // fn coordinate(&self, data: &TransactionMonitor) -> Result<(), BitcoinCoordinatorStoreError>;

    fn save_tx(
        &self,
        group_id: Option<Id>,
        tx: Transaction,
    ) -> Result<(), BitcoinCoordinatorStoreError>;
    fn remove_coordinator(
        &self,
        data: &TransactionMonitor,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn get_txs_by_state(
        &self,
        state: TransactionState,
    ) -> Result<Vec<CoordinatedTransaction>, BitcoinCoordinatorStoreError>;

    fn update_tx_state(
        &self,
        tx_id: Txid,
        status: TransactionState,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    // SPEED UP TRANSACTIONS
    fn get_speed_up_txs_for_child(
        &self,
        id: String,
        child_tx_id: &Txid,
    ) -> Result<Vec<SpeedUpTx>, BitcoinCoordinatorStoreError>;

    fn add_speed_up_tx(
        &self,
        id: String,
        speed_up_tx: &SpeedUpTx,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn get_speed_up_tx(
        &self,
        id: String,
        tx_id: &Txid,
    ) -> Result<Option<SpeedUpTx>, BitcoinCoordinatorStoreError>;

    fn is_speed_up_tx(
        &self,
        id: String,
        tx_id: &Txid,
    ) -> Result<bool, BitcoinCoordinatorStoreError>;

    // FUNDING TRANSACTIONS
    // Funding transactions are used to provide capital to speed-up transactions
    // when fee acceleration is needed
    fn get_funding_tx(
        &self,
        data: &TransactionFund,
    ) -> Result<Option<FundingTransaction>, BitcoinCoordinatorStoreError>;
    fn fund_for_speedup(&self, data: &TransactionFund) -> Result<(), BitcoinCoordinatorStoreError>;
    fn remove_funding_tx(
        &self,
        instance_id: Id,
        tx: &Txid,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    // FUNDING TRANSACTIONS REQUESTS
    // Funding requests are created when an instance run out off funds
    // and requires additional funding to speed up transactions
    fn add_funding_request(&self, group_id: Id) -> Result<(), BitcoinCoordinatorStoreError>;
    fn acknowledge_funding_request(
        &self,
        instance_id: Id,
    ) -> Result<(), BitcoinCoordinatorStoreError>;
    fn get_funding_requests(&self) -> Result<Vec<Id>, BitcoinCoordinatorStoreError>;

    fn add_tx_news(
        &self,
        group_id: Option<Id>,
        tx_id: Txid,
    ) -> Result<(), BitcoinCoordinatorStoreError>;
    fn get_news(&self) -> Result<Vec<(Option<Id>, Vec<Txid>)>, BitcoinCoordinatorStoreError>;
    fn acknowledge_instance_tx_news(
        &self,
        instance_id: Id,
        tx_id: Txid,
    ) -> Result<(), BitcoinCoordinatorStoreError>;
}

impl BitcoinCoordinatorStore {
    pub fn new(store: Rc<Storage>) -> Result<Self, BitcoinCoordinatorStoreError> {
        Ok(Self { store })
    }

    fn get_key(&self, key: StoreKey) -> String {
        let prefix = "bitcoin_coordinator";
        match key {
            StoreKey::TransactionList => format!("{prefix}/instance/list"),
            StoreKey::Transaction(tx_id) => format!("{prefix}/tx/{tx_id}"),
            StoreKey::TransactionFundingList(instance_id) => {
                format!("{prefix}/instance/{instance_id}/funding/list")
            }
            StoreKey::TransactionSpeedUpList(id) => {
                format!("{prefix}/speed_up/{id}/list")
            }
            StoreKey::FundingRequestList => format!("{prefix}/funding/request/list"),
        }
    }

    fn get_txs_by_state(
        &self,
        status: TransactionState,
    ) -> Result<Vec<(Option<Id>, CoordinatedTransaction)>, BitcoinCoordinatorStoreError> {
        let txs = self.get_txs()?;
        let mut txs_filter = Vec::new();

        for tx_id in txs {
            let tx = self.get_tx(tx_id)?;

            if tx.state == status {
                txs_filter.push(tx);
            }
        }

        Ok(txs_filter)
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
}

impl BitcoinCoordinatorStoreApi for BitcoinCoordinatorStore {
    fn save_tx(
        &self,
        group_id: Option<Id>,
        tx: Transaction,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let key = self.get_key(StoreKey::Transaction(tx.compute_txid()));

        let tx_info =
            CoordinatedTransaction::new(group_id, tx.clone(), TransactionState::ReadyToSend);

        self.store.set(&key, &tx_info, None)?;

        let txs_key = self.get_key(StoreKey::TransactionList);
        let mut txs = self
            .store
            .get::<&str, Vec<(Option<Id>, Txid)>>(&txs_key)?
            .unwrap_or_default();
        txs.push((group_id, tx.compute_txid()));
        self.store.set(&txs_key, &txs, None)?;

        Ok(())
    }

    // fn coordinate(&self, data: &TransactionMonitor) -> Result<(), BitcoinCoordinatorStoreError> {
    //     let mut txs_to_insert: Vec<CoordinatedTransaction> = vec![];

    //     for tx in data.txs.iter() {
    //         let tx_info = CoordinatedTransaction {
    //             tx_id: tx.tx_id,
    //             deliver_block_height: None,
    //             tx: None,
    //             state: TransactionState::New,
    //         };
    //         txs_to_insert.push(tx_info);
    //     }

    //     let instance_key = self.get_key(StoreKey::Transaction(data.id));

    //     // Map BitvmxInstance
    //     // 1. Store the instance under its ID
    //     self.store.set(instance_key, txs_to_insert, None)?;

    //     // 2. Maintain the list of all instances (instance/list)
    //     let instances_key = self.get_key(StoreKey::TransactionList);

    //     let mut all_instances = self
    //         .store
    //         .get::<_, Vec<Id>>(&instances_key)?
    //         .unwrap_or_default();

    //     // Add the new instance ID to the list if it's not already present
    //     if !all_instances.contains(&data.id) {
    //         all_instances.push(data.id);
    //         self.store.set(&instances_key, &all_instances, None)?;
    //     }

    //     if data.funding_tx.is_some() {
    //         self.fund_for_speedup(data.id, data.funding_tx.as_ref().unwrap())?;
    //     }

    //     Ok(())
    // }

    // This method is currently used for testing purposes only and may not be necessary in the future.
    // It is intended to facilitate the testing of instance-related operations within the storage system.
    // fn remove_coordinator(&self, instance_id: Id) -> Result<(), BitcoinCoordinatorStoreError> {
    //     let instance_key = self.get_key(StoreKey::Transaction(instance_id));
    //     self.store.delete(&instance_key)?;

    //     let instances_key = self.get_key(StoreKey::TransactionList);

    //     let mut all_instance_ids = self
    //         .store
    //         .get::<_, Vec<Id>>(&instances_key)?
    //         .unwrap_or_default();

    //     all_instance_ids.retain(|&id| id != instance_id);
    //     self.store.set(&instances_key, &all_instance_ids, None)?;

    //     let speed_up_tx_key = self.get_key(StoreKey::TransactionSpeedUpList(instance_id));

    //     self.store.delete(&speed_up_tx_key)?;

    //     let speed_up_txs_key = self.get_key(StoreKey::TransactionSpeedUpList(instance_id));
    //     self.store.delete(&speed_up_txs_key)?;

    //     Ok(())
    // }

    fn update_instance_tx_as_sent(
        &self,
        instance_id: Id,
        tx_id: &Txid,
        block_height: BlockHeight,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let key = self.get_key(StoreKey::Transaction(instance_id));

        let mut txs = self
            .store
            .get::<&str, Vec<CoordinatedTransaction>>(&key)
            .map_err(|e| {
                BitcoinCoordinatorStoreError::BitcoinCoordinatorStoreError(
                    format!("Failed to retrieve instance with ID {}", instance_id),
                    e,
                )
            })?
            .unwrap_or_default();

        if let Some(tx) = txs.iter_mut().find(|x| x.tx_id == *tx_id) {
            tx.deliver_block_height = Some(block_height);
            tx.state = TransactionState::Sent;
        }

        self.store.set(key, txs, None)?;

        Ok(())
    }

    fn get_funding_tx(
        &self,
        instance_id: Id,
    ) -> Result<Option<FundingTransaction>, BitcoinCoordinatorStoreError> {
        let funding_tx_key = self.get_key(StoreKey::TransactionFundingList(instance_id));
        let funding_txs = self
            .store
            .get::<&str, Vec<FundingTransaction>>(&funding_tx_key)
            .map_err(|e| {
                BitcoinCoordinatorStoreError::BitcoinCoordinatorStoreError(
                    "Failed to retrieve funding transaction".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();

        if let Some(last_funding_tx) = funding_txs.last() {
            Ok(Some(last_funding_tx.clone()))
        } else {
            Ok(None)
        }
    }

    fn fund_for_speedup(&self, data: &TransactionFund) -> Result<(), BitcoinCoordinatorStoreError> {
        let funding_tx_key = self.get_key(StoreKey::TransactionFundingList(instance_id));

        let mut funding_txs = self
            .store
            .get::<&str, Vec<FundingTransaction>>(&funding_tx_key)
            .map_err(|e| {
                BitcoinCoordinatorStoreError::BitcoinCoordinatorStoreError(
                    "Failed to retrieve funding transaction".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();

        funding_txs.push(funding_tx.clone());

        self.store.set(&funding_tx_key, &funding_txs, None)?;

        Ok(())
    }

    fn get_speed_up_txs_for_child(
        &self,
        id: String,
        child_tx_id: &Txid,
    ) -> Result<Vec<SpeedUpTx>, BitcoinCoordinatorStoreError> {
        let speed_up_tx_key = self.get_key(StoreKey::TransactionSpeedUpList(group_id));

        // Retrieve the speed up transactions from the storage
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

        speed_up_txs.retain(|t| t.child_tx_id == *child_tx_id);

        Ok(speed_up_txs)
    }

    fn get_speed_up_tx(
        &self,
        id: String,
        tx_id: &Txid,
    ) -> Result<Option<SpeedUpTx>, BitcoinCoordinatorStoreError> {
        let speed_up_tx_key = self.get_key(StoreKey::TransactionSpeedUpList(id));

        // Retrieve the list of speed up transactions from storage
        let speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)
            .map_err(|e| {
                BitcoinCoordinatorStoreError::BitcoinCoordinatorStoreError(
                    "Failed to retrieve funding transaction".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();

        // Find the specific speed up transaction that matches the given tx_id
        let speed_up_tx = speed_up_txs.into_iter().find(|t| t.tx_id == *tx_id);

        Ok(speed_up_tx)
    }

    // This function adds a new speed up transaction to the list of speed up transactions associated with an instance.
    // Speed up transactions are stored in a list, with the most recent transaction added to the end of the list.
    // This design ensures that if the last transaction in the list is pending, there cannot be another pending speed up transaction
    // for the same instance, except for one that is specifically related to the same child transaction.
    fn add_speed_up_tx(
        &self,
        id: String,
        speed_up_tx: &SpeedUpTx,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let speed_up_tx_key = self.get_key(StoreKey::TransactionSpeedUpList(id));

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

    fn is_speed_up_tx(
        &self,
        id: String,
        tx_id: &Txid,
    ) -> Result<bool, BitcoinCoordinatorStoreError> {
        let speed_up_tx = self.get_speed_up_tx(id, tx_id)?;
        Ok(speed_up_tx.is_some())
    }

    fn update_tx_state(
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

    fn remove_funding_tx(
        &self,
        instance_id: Id,
        funding_tx_id: &Txid,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let funding_tx_key = self.get_key(StoreKey::TransactionFundingList(instance_id));

        // Retrieve the current list of funding transactions for the instance from storage.
        let mut funding_txs = self
            .store
            .get::<&str, Vec<FundingTransaction>>(&funding_tx_key)
            .map_err(|e| {
                BitcoinCoordinatorStoreError::BitcoinCoordinatorStoreError(
                    "Failed to retrieve funding transaction".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();

        // Remove the specified funding transaction from the list.
        funding_txs.retain(|t| t.tx_id != *funding_tx_id);

        // Save the updated list of funding transactions back to storage.
        self.store.set(&funding_tx_key, &funding_txs, None)?;

        Ok(())
    }

    fn add_funding_request(&self, group_id: Id) -> Result<(), BitcoinCoordinatorStoreError> {
        let funding_request_key = self.get_key(StoreKey::FundingRequestList);
        let mut funding_requests = self
            .store
            .get::<&str, Vec<Id>>(&funding_request_key)
            .map_err(|e| {
                BitcoinCoordinatorStoreError::BitcoinCoordinatorStoreError(
                    "Failed to retrieve funding transaction".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();

        funding_requests.push(group_id);
        self.store
            .set(&funding_request_key, &funding_requests, None)?;
        Ok(())
    }

    fn acknowledge_funding_request(
        &self,
        instance_id: Id,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let funding_request_key = self.get_key(StoreKey::FundingRequestList);
        let mut funding_requests = self
            .store
            .get::<&str, Vec<Id>>(&funding_request_key)
            .map_err(|e| {
                BitcoinCoordinatorStoreError::BitcoinCoordinatorStoreError(
                    "Failed to retrieve funding transaction".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();

        funding_requests.retain(|&id| id != instance_id);
        self.store
            .set(&funding_request_key, &funding_requests, None)?;
        Ok(())
    }

    fn get_funding_requests(&self) -> Result<Vec<Id>, BitcoinCoordinatorStoreError> {
        let funding_request_key = self.get_key(StoreKey::FundingRequestList);
        let funding_requests = self
            .store
            .get::<&str, Vec<Id>>(&funding_request_key)
            .map_err(|e| {
                BitcoinCoordinatorStoreError::BitcoinCoordinatorStoreError(
                    "Failed to retrieve funding transaction".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();
        Ok(funding_requests)
    }
}

use crate::{
    errors::OrchestratorStoreError,
    types::{
        BitvmxInstance, FundingTx, InstanceId, SpeedUpTx, TransactionInfo, TransactionPartialInfo,
        TransactionState,
    },
};

use bitcoin::{Transaction, Txid};
use bitvmx_transaction_monitor::types::BlockHeight;
use mockall::automock;
use std::rc::Rc;
use storage_backend::storage::{KeyValueStore, Storage};
pub struct OrchestratorStore {
    store: Rc<Storage>,
}

enum StoreKey {
    Instance(InstanceId),
    InstanceList,
    InstanceFundingList(InstanceId),
    InstanceSpeedUpList(InstanceId),
    FundingRequestList,
    InstanceTxNews,
}

#[automock]
pub trait OrchestratorStoreApi {
    fn tx_exists(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<bool, OrchestratorStoreError>;
    fn get_instance(
        &self,
        instance_id: InstanceId,
    ) -> Result<Vec<TransactionInfo>, OrchestratorStoreError>;
    fn get_instances(&self) -> Result<Vec<InstanceId>, OrchestratorStoreError>;
    fn get_instance_tx(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<TransactionInfo>, OrchestratorStoreError>;

    fn add_instance(
        &self,
        instance: &BitvmxInstance<TransactionPartialInfo>,
    ) -> Result<(), OrchestratorStoreError>;
    fn add_instance_tx_hex(
        &self,
        instance_id: InstanceId,
        tx_id: Txid,
        tx_hex: String,
    ) -> Result<(), OrchestratorStoreError>;

    fn add_tx_to_instance(
        &self,
        instance_id: InstanceId,
        tx: &Transaction,
    ) -> Result<(), OrchestratorStoreError>;
    fn remove_instance(&self, instance_id: InstanceId) -> Result<(), OrchestratorStoreError>;

    fn update_instance_tx_as_sent(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        block_height: BlockHeight,
    ) -> Result<(), OrchestratorStoreError>;

    fn get_txs_info(
        &self,
        tx_state: TransactionState,
    ) -> Result<Vec<(InstanceId, Vec<TransactionInfo>)>, OrchestratorStoreError>;

    fn update_instance_tx_status(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        status: TransactionState,
    ) -> Result<(), OrchestratorStoreError>;

    // SPEED UP TRANSACTIONS
    fn get_speed_up_txs_for_child(
        &self,
        instance_id: InstanceId,
        child_tx_id: &Txid,
    ) -> Result<Vec<SpeedUpTx>, OrchestratorStoreError>;

    fn add_speed_up_tx(
        &self,
        instance_id: InstanceId,
        speed_up_tx: &SpeedUpTx,
    ) -> Result<(), OrchestratorStoreError>;

    fn get_speed_up_tx(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<SpeedUpTx>, OrchestratorStoreError>;

    fn is_speed_up_tx(
        &self,
        instance_id: u32,
        tx_id: &Txid,
    ) -> Result<bool, OrchestratorStoreError>;

    // FUNDING TRANSACTIONS
    // Funding transactions are used to provide capital to speed-up transactions
    // when fee acceleration is needed
    fn get_funding_tx(
        &self,
        instance_id: InstanceId,
    ) -> Result<Option<FundingTx>, OrchestratorStoreError>;
    fn add_funding_tx(
        &self,
        instance_id: InstanceId,
        tx: &FundingTx,
    ) -> Result<(), OrchestratorStoreError>;
    fn remove_funding_tx(
        &self,
        instance_id: InstanceId,
        tx: &Txid,
    ) -> Result<(), OrchestratorStoreError>;

    // FUNDING TRANSACTIONS REQUESTS
    // Funding requests are created when an instance run out off funds
    // and requires additional funding to speed up transactions
    fn add_funding_request(&self, instance_id: InstanceId) -> Result<(), OrchestratorStoreError>;
    fn acknowledge_funding_request(
        &self,
        instance_id: InstanceId,
    ) -> Result<(), OrchestratorStoreError>;
    fn get_funding_requests(&self) -> Result<Vec<InstanceId>, OrchestratorStoreError>;

    fn add_instance_tx_news(
        &self,
        instance_id: InstanceId,
        tx_id: Txid,
    ) -> Result<(), OrchestratorStoreError>;
    fn get_instance_tx_news(&self) -> Result<Vec<(InstanceId, Vec<Txid>)>, OrchestratorStoreError>;
    fn acknowledge_instance_tx_news(
        &self,
        instance_id: InstanceId,
        tx_id: Txid,
    ) -> Result<(), OrchestratorStoreError>;
}

impl OrchestratorStore {
    pub fn new(store: Rc<Storage>) -> Result<Self, OrchestratorStoreError> {
        Ok(Self { store })
    }

    fn get_key(&self, key: StoreKey) -> String {
        match key {
            StoreKey::InstanceList => "instance/list".to_string(),
            StoreKey::Instance(instance_id) => format!("instance/{}", instance_id),
            StoreKey::InstanceFundingList(instance_id) => {
                format!("instance/{}/funding/list", instance_id)
            }
            StoreKey::InstanceSpeedUpList(instance_id) => {
                format!("instance/{}/list", instance_id)
            }
            StoreKey::FundingRequestList => "funding/request/list".to_string(),
            StoreKey::InstanceTxNews => "instance/news".to_string(),
        }
    }

    pub fn update_instance_tx_status(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        tx_state: TransactionState,
    ) -> Result<(), OrchestratorStoreError> {
        //TODO: Implement transaction status transition validation to ensure the correct sequence:
        // Pending -> InProgress -> Completed, and in reorganization scenarios, do the reverse order.
        let mut txs = self.get_instance(instance_id)?;
        let tx_index = txs
            .iter()
            .position(|tx| tx.tx_id == *tx_id)
            .expect("Transaction not found in instance");

        txs[tx_index].state = tx_state;

        let instance_key = self.get_key(StoreKey::Instance(instance_id));
        self.store.set(instance_key, txs, None)?;

        Ok(())
    }

    fn get_txs_info(
        &self,
        status: TransactionState,
    ) -> Result<Vec<(InstanceId, Vec<TransactionInfo>)>, OrchestratorStoreError> {
        let instances_ids = self.get_instances()?;
        let mut ret_instance_txs: Vec<(InstanceId, Vec<TransactionInfo>)> = Vec::new();

        for instance_id in instances_ids {
            let mut txs: Vec<TransactionInfo> = Vec::new();
            let instance_txs = self.get_instance(instance_id)?;

            for tx in instance_txs {
                if tx.state == status {
                    txs.push(tx);
                }
            }

            if !txs.is_empty() {
                ret_instance_txs.push((instance_id, txs));
            }
        }

        Ok(ret_instance_txs)
    }
}

impl OrchestratorStoreApi for OrchestratorStore {
    fn get_txs_info(
        &self,
        tx_state: TransactionState,
    ) -> Result<Vec<(InstanceId, Vec<TransactionInfo>)>, OrchestratorStoreError> {
        self.get_txs_info(tx_state)
    }

    fn get_instances(&self) -> Result<Vec<InstanceId>, OrchestratorStoreError> {
        let instances_list_key = self.get_key(StoreKey::InstanceList);

        let all_instance_ids = self
            .store
            .get::<&str, Vec<u32>>(&instances_list_key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
                    "Failed to retrieve instances".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();

        Ok(all_instance_ids)
    }

    fn get_instance(
        &self,
        instance_id: InstanceId,
    ) -> Result<Vec<TransactionInfo>, OrchestratorStoreError> {
        let key = self.get_key(StoreKey::Instance(instance_id));
        let txs = self
            .store
            .get::<&str, Vec<TransactionInfo>>(&key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
                    format!("Failed to retrieve instance with ID {}", instance_id),
                    e,
                )
            })?
            .unwrap_or_default();

        Ok(txs)
    }

    fn add_instance(
        &self,
        instance: &BitvmxInstance<TransactionPartialInfo>,
    ) -> Result<(), OrchestratorStoreError> {
        let mut txs_to_insert: Vec<TransactionInfo> = vec![];

        for tx in instance.txs.iter() {
            let tx_info = TransactionInfo {
                tx_id: tx.tx_id,
                deliver_block_height: None,
                tx: None,
                state: TransactionState::New,
                tx_hex: None,
            };
            txs_to_insert.push(tx_info);
        }

        let instance_key = self.get_key(StoreKey::Instance(instance.instance_id));

        // Map BitvmxInstance
        // 1. Store the instance under its ID
        self.store.set(instance_key, txs_to_insert, None)?;

        // 2. Maintain the list of all instances (instance/list)
        let instances_key = self.get_key(StoreKey::InstanceList);

        let mut all_instances = self
            .store
            .get::<_, Vec<u32>>(&instances_key)?
            .unwrap_or_default();

        // Add the new instance ID to the list if it's not already present
        if !all_instances.contains(&instance.instance_id) {
            all_instances.push(instance.instance_id);
            self.store.set(&instances_key, &all_instances, None)?;
        }

        self.add_funding_tx(instance.instance_id, &instance.funding_tx)?;

        Ok(())
    }

    // This method is currently used for testing purposes only and may not be necessary in the future.
    // It is intended to facilitate the testing of instance-related operations within the storage system.
    fn remove_instance(&self, instance_id: InstanceId) -> Result<(), OrchestratorStoreError> {
        let instance_key = self.get_key(StoreKey::Instance(instance_id));
        self.store.delete(&instance_key)?;

        let instances_key = self.get_key(StoreKey::InstanceList);

        let mut all_instance_ids = self
            .store
            .get::<_, Vec<u32>>(&instances_key)?
            .unwrap_or_default();

        all_instance_ids.retain(|&id| id != instance_id);
        self.store.set(&instances_key, &all_instance_ids, None)?;

        let speed_up_tx_key = self.get_key(StoreKey::InstanceSpeedUpList(instance_id));

        self.store.delete(&speed_up_tx_key)?;

        let speed_up_txs_key = self.get_key(StoreKey::InstanceSpeedUpList(instance_id));
        self.store.delete(&speed_up_txs_key)?;

        Ok(())
    }

    fn get_instance_tx(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<TransactionInfo>, OrchestratorStoreError> {
        let txs = self.get_instance(instance_id)?;

        for tx in &txs {
            if tx.tx_id == *tx_id {
                return Ok(Some(tx.clone()));
            }
        }

        Ok(None)
    }

    fn tx_exists(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<bool, OrchestratorStoreError> {
        let tx_instance = self.get_instance_tx(instance_id, tx_id)?;
        Ok(tx_instance.is_some())
    }

    fn add_tx_to_instance(
        &self,
        instance_id: InstanceId,
        tx: &Transaction,
    ) -> Result<(), OrchestratorStoreError> {
        let mut txs = self.get_instance(instance_id)?;

        let tx_id = tx.compute_txid();

        for tx_instance in txs.iter_mut() {
            if tx_instance.tx_id == tx_id {
                tx_instance.tx = Some(tx.clone());
                let key = self.get_key(StoreKey::Instance(instance_id));
                self.store.set(key, txs, None)?;
                break;
            }
        }

        Ok(())
    }

    fn update_instance_tx_as_sent(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        block_height: BlockHeight,
    ) -> Result<(), OrchestratorStoreError> {
        let key = self.get_key(StoreKey::Instance(instance_id));

        let mut txs = self
            .store
            .get::<&str, Vec<TransactionInfo>>(&key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
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

    fn add_instance_tx_hex(
        &self,
        instance_id: InstanceId,
        tx_id: Txid,
        tx_hex: String,
    ) -> Result<(), OrchestratorStoreError> {
        let mut txs = self.get_instance(instance_id)?;
        let tx_index = txs
            .iter_mut()
            .position(|tx| tx.tx_id == tx_id)
            .expect("Transaction not found in instance");
        txs[tx_index].tx_hex = Some(tx_hex);
        let instance_key = self.get_key(StoreKey::Instance(instance_id));
        self.store.set(instance_key, txs, None)?;
        Ok(())
    }

    fn get_funding_tx(
        &self,
        instance_id: InstanceId,
    ) -> Result<Option<FundingTx>, OrchestratorStoreError> {
        let funding_tx_key = self.get_key(StoreKey::InstanceFundingList(instance_id));
        let funding_txs = self
            .store
            .get::<&str, Vec<FundingTx>>(&funding_tx_key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
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

    fn add_funding_tx(
        &self,
        instance_id: InstanceId,
        funding_tx: &FundingTx,
    ) -> Result<(), OrchestratorStoreError> {
        let funding_tx_key = self.get_key(StoreKey::InstanceFundingList(instance_id));

        let mut funding_txs = self
            .store
            .get::<&str, Vec<FundingTx>>(&funding_tx_key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
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
        instance_id: InstanceId,
        child_tx_id: &Txid,
    ) -> Result<Vec<SpeedUpTx>, OrchestratorStoreError> {
        let speed_up_tx_key = self.get_key(StoreKey::InstanceSpeedUpList(instance_id));

        // Retrieve the speed up transactions from the storage
        let mut speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
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
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<SpeedUpTx>, OrchestratorStoreError> {
        let speed_up_tx_key = self.get_key(StoreKey::InstanceSpeedUpList(instance_id));

        // Retrieve the list of speed up transactions from storage
        let speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
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
        instance_id: InstanceId,
        speed_up_tx: &SpeedUpTx,
    ) -> Result<(), OrchestratorStoreError> {
        let speed_up_tx_key = self.get_key(StoreKey::InstanceSpeedUpList(instance_id));

        // Retrieve the current list of speed up transactions for the instance from storage.
        let mut speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
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
        instance_id: u32,
        tx_id: &Txid,
    ) -> Result<bool, OrchestratorStoreError> {
        let speed_up_tx = self.get_speed_up_tx(instance_id, tx_id)?;
        Ok(speed_up_tx.is_some())
    }

    fn update_instance_tx_status(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        tx_state: TransactionState,
    ) -> Result<(), OrchestratorStoreError> {
        self.update_instance_tx_status(instance_id, tx_id, tx_state)
    }

    fn remove_funding_tx(
        &self,
        instance_id: InstanceId,
        funding_tx_id: &Txid,
    ) -> Result<(), OrchestratorStoreError> {
        let funding_tx_key = self.get_key(StoreKey::InstanceFundingList(instance_id));

        // Retrieve the current list of funding transactions for the instance from storage.
        let mut funding_txs = self
            .store
            .get::<&str, Vec<FundingTx>>(&funding_tx_key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
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

    fn add_funding_request(&self, instance_id: InstanceId) -> Result<(), OrchestratorStoreError> {
        let funding_request_key = self.get_key(StoreKey::FundingRequestList);
        let mut funding_requests = self
            .store
            .get::<&str, Vec<InstanceId>>(&funding_request_key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
                    "Failed to retrieve funding transaction".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();

        funding_requests.push(instance_id);
        self.store
            .set(&funding_request_key, &funding_requests, None)?;
        Ok(())
    }

    fn acknowledge_funding_request(
        &self,
        instance_id: InstanceId,
    ) -> Result<(), OrchestratorStoreError> {
        let funding_request_key = self.get_key(StoreKey::FundingRequestList);
        let mut funding_requests = self
            .store
            .get::<&str, Vec<InstanceId>>(&funding_request_key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
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

    fn get_funding_requests(&self) -> Result<Vec<InstanceId>, OrchestratorStoreError> {
        let funding_request_key = self.get_key(StoreKey::FundingRequestList);
        let funding_requests = self
            .store
            .get::<&str, Vec<InstanceId>>(&funding_request_key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
                    "Failed to retrieve funding transaction".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();
        Ok(funding_requests)
    }

    fn add_instance_tx_news(
        &self,
        instance_id: InstanceId,
        tx_id: Txid,
    ) -> Result<(), OrchestratorStoreError> {
        let instance_tx_news_key = self.get_key(StoreKey::InstanceTxNews);
        let mut instance_tx_news = self
            .store
            .get::<&str, Vec<(InstanceId, Vec<Txid>)>>(&instance_tx_news_key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
                    "Failed to retrieve instance tx news".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();

        // create a new entry for the instance if it doesn't exist
        if !instance_tx_news.iter().any(|(id, _)| *id == instance_id) {
            instance_tx_news.push((instance_id, vec![]));
        }

        // add the tx_id to the instance's list of news
        if let Some(instance_txs) = instance_tx_news
            .iter_mut()
            .find(|(id, _)| *id == instance_id)
        {
            instance_txs.1.push(tx_id);
        }

        self.store
            .set(&instance_tx_news_key, &instance_tx_news, None)?;
        Ok(())
    }

    fn get_instance_tx_news(&self) -> Result<Vec<(InstanceId, Vec<Txid>)>, OrchestratorStoreError> {
        let instance_tx_news_key = self.get_key(StoreKey::InstanceTxNews);
        let instance_tx_news = self
            .store
            .get::<&str, Vec<(InstanceId, Vec<Txid>)>>(&instance_tx_news_key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
                    "Failed to retrieve instance tx news".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();
        Ok(instance_tx_news)
    }

    fn acknowledge_instance_tx_news(
        &self,
        instance_id: InstanceId,
        tx_id: Txid,
    ) -> Result<(), OrchestratorStoreError> {
        let instance_tx_news_key = self.get_key(StoreKey::InstanceTxNews);
        let mut instance_tx_news = self
            .store
            .get::<&str, Vec<(InstanceId, Vec<Txid>)>>(&instance_tx_news_key)
            .map_err(|e| {
                OrchestratorStoreError::OrchestratorStoreError(
                    "Failed to retrieve instance tx news".to_string(),
                    e,
                )
            })?
            .unwrap_or_default();

        // Find the instance's transaction and remove the tx_id
        if let Some(instance_txs) = instance_tx_news
            .iter_mut()
            .find(|(id, _)| *id == instance_id)
        {
            instance_txs.1.retain(|&t| t != tx_id);
        }

        // Remove any empty instance entries
        instance_tx_news.retain(|(_, txs)| !txs.is_empty());

        self.store
            .set(&instance_tx_news_key, &instance_tx_news, None)?;
        Ok(())
    }
}

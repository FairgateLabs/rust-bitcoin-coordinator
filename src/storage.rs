use crate::types::{
    BitvmxInstance, FundingTx, InstanceId, SpeedUpTx, TransactionInfo, TransactionPartialInfo,
    TransactionStatus,
};
use anyhow::{Context, Ok, Result};
use bitcoin::{Transaction, Txid};
use bitvmx_transaction_monitor::types::BlockHeight;
use mockall::automock;
use std::path::PathBuf;
use storage_backend::storage::{KeyValueStore, Storage};
pub struct BitvmxStore {
    store: Storage,
}

enum StoreKey {
    Instance(InstanceId),
    InstanceList,
    InstanceFundingList(InstanceId),
    InstanceSpeedUpList(InstanceId),
}

#[automock]
pub trait BitvmxStoreApi {
    fn tx_exists(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<bool>;
    fn get_instance(&self, instance_id: InstanceId) -> Result<Vec<TransactionInfo>>;
    fn get_instances(&self) -> Result<Vec<InstanceId>>;
    fn get_instance_tx(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<TransactionInfo>>;

    fn add_instance(&self, instance: &BitvmxInstance<TransactionPartialInfo>) -> Result<()>;
    fn add_instance_tx_hex(
        &self,
        instance_id: InstanceId,
        tx_id: Txid,
        tx_hex: String,
    ) -> Result<()>;
    fn add_tx_to_instance(&self, instance_id: InstanceId, tx: &Transaction) -> Result<()>;
    fn remove_instance(&self, instance_id: InstanceId) -> Result<()>;

    fn add_in_progress_instance_tx(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        block_height: BlockHeight,
    ) -> Result<()>;

    fn get_txs_info(
        &self,
        status: TransactionStatus,
    ) -> Result<Vec<(InstanceId, Vec<TransactionInfo>)>>;

    //FUNDING
    fn get_funding_tx(&self, instance_id: InstanceId) -> Result<Option<FundingTx>>;
    fn add_funding_tx(&self, instance_id: InstanceId, tx: &FundingTx) -> Result<()>;

    //SPEED UP
    fn get_speed_up_txs_for_child(
        &self,
        instance_id: InstanceId,
        child_tx_id: &Txid,
    ) -> Result<Vec<SpeedUpTx>>;

    fn add_speed_up_tx(&self, instance_id: InstanceId, speed_up_tx: &SpeedUpTx) -> Result<()>;

    fn get_speed_up_tx(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<Option<SpeedUpTx>>;

    fn is_speed_up_tx(&self, instance_id: u32, tx_id: Txid) -> Result<bool>;

    fn update_instance_tx_status(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        status: TransactionStatus,
    ) -> Result<()>;
}

impl BitvmxStore {
    pub fn new_with_path(store_path: &str) -> Result<Self> {
        let store = Storage::new_with_path(&PathBuf::from(store_path.to_string()))
            .context("There is an error creating storage in BitvmxStore")?;
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
        }
    }

    pub fn update_instance_tx_status(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        status: TransactionStatus,
    ) -> Result<()> {
        //TODO: Implement transaction status transition validation to ensure the correct sequence:
        // Pending -> InProgress -> Completed, and in reorganization scenarios, do the reverse order.
        let mut txs = self.get_instance(instance_id)?;
        let tx_index = txs
            .iter()
            .position(|tx| tx.tx_id == *tx_id)
            .expect("Transaction not found in instance");

        txs[tx_index].status = status;

        let instance_key = self.get_key(StoreKey::Instance(instance_id));
        self.store.set(instance_key, txs)?;

        Ok(())
    }

    fn get_txs_info(
        &self,
        status: TransactionStatus,
    ) -> Result<Vec<(InstanceId, Vec<TransactionInfo>)>> {
        let instances_ids = self.get_instances()?;
        let mut ret_instance_txs: Vec<(InstanceId, Vec<TransactionInfo>)> = Vec::new();

        for instance_id in instances_ids {
            let mut txs: Vec<TransactionInfo> = Vec::new();
            let instance_txs = self.get_instance(instance_id)?;

            for tx in instance_txs {
                if tx.status == status {
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

impl BitvmxStoreApi for BitvmxStore {
    fn get_txs_info(
        &self,
        status: TransactionStatus,
    ) -> Result<Vec<(InstanceId, Vec<TransactionInfo>)>> {
        self.get_txs_info(status)
    }

    fn get_instances(&self) -> Result<Vec<InstanceId>> {
        let instances_list_key = self.get_key(StoreKey::InstanceList);

        let all_instance_ids = self
            .store
            .get::<&str, Vec<u32>>(&instances_list_key)
            .context("Failed to retrieve instances")?
            .unwrap_or_default();

        Ok(all_instance_ids)
    }

    fn get_instance(&self, instance_id: InstanceId) -> Result<Vec<TransactionInfo>> {
        let key = self.get_key(StoreKey::Instance(instance_id));
        let txs = self
            .store
            .get::<&str, Vec<TransactionInfo>>(&key)
            .context(format!(
                "Failed to retrieve instance with ID {}",
                instance_id
            ))?
            .unwrap_or_default();

        Ok(txs)
    }

    fn add_instance(&self, instance: &BitvmxInstance<TransactionPartialInfo>) -> Result<()> {
        let mut txs_to_insert: Vec<TransactionInfo> = vec![];

        for tx in instance.txs.iter() {
            let tx_info = TransactionInfo {
                tx_id: tx.tx_id,
                owner_operator_id: tx.owner_operator_id,
                deliver_block_height: None,
                tx: None,
                status: TransactionStatus::New,
                tx_hex: None,
            };
            txs_to_insert.push(tx_info);
        }

        let instance_key = self.get_key(StoreKey::Instance(instance.instance_id));

        // Map BitvmxInstance
        // 1. Store the instance under its ID
        self.store
            .set(&instance_key, txs_to_insert)
            .context(format!(
                "Failed to store instance under key {}",
                instance_key
            ))?;

        // 2. Maintain the list of all instances (instance/list)
        let instances_key = self.get_key(StoreKey::InstanceList);

        let mut all_instances = self
            .store
            .get::<_, Vec<u32>>(&instances_key)?
            .unwrap_or_default();

        // Add the new instance ID to the list if it's not already present
        if !all_instances.contains(&instance.instance_id) {
            all_instances.push(instance.instance_id);
            self.store
                .set(&instances_key, &all_instances)
                .context("Failed to update instances list")?;
        }

        self.add_funding_tx(instance.instance_id, &instance.funding_tx)?;

        Ok(())
    }

    // This method is currently used for testing purposes only and may not be necessary in the future.
    // It is intended to facilitate the testing of instance-related operations within the storage system.
    fn remove_instance(&self, instance_id: InstanceId) -> Result<()> {
        let instance_key = self.get_key(StoreKey::Instance(instance_id));
        self.store
            .delete(&instance_key)
            .context("Failed to delete instance")?;

        let instances_key = self.get_key(StoreKey::InstanceList);

        let mut all_instance_ids = self
            .store
            .get::<_, Vec<u32>>(&instances_key)?
            .unwrap_or_default();

        all_instance_ids.retain(|&id| id != instance_id);
        self.store.set(&instances_key, &all_instance_ids)?;

        let speed_up_tx_key = self.get_key(StoreKey::InstanceSpeedUpList(instance_id));

        self.store
            .delete(&speed_up_tx_key)
            .context("Failed to delete speed up transactions for instance")?;

        let speed_up_txs_key = self.get_key(StoreKey::InstanceSpeedUpList(instance_id));
        self.store
            .delete(&speed_up_txs_key)
            .context("Failed to delete speed up transactions for instance")?;

        Ok(())
    }

    fn get_instance_tx(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<TransactionInfo>> {
        let txs = self.get_instance(instance_id)?;

        for tx in &txs {
            if tx.tx_id == *tx_id {
                return Ok(Some(tx.clone()));
            }
        }

        Ok(None)
    }

    fn tx_exists(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<bool> {
        let tx_instance = self.get_instance_tx(instance_id, tx_id)?;
        Ok(tx_instance.is_some())
    }

    fn add_tx_to_instance(&self, instance_id: InstanceId, tx: &Transaction) -> Result<()> {
        let mut txs = self.get_instance(instance_id)?;

        let tx_id = tx.compute_txid();

        for tx_instance in txs.iter_mut() {
            if tx_instance.tx_id == tx_id {
                tx_instance.tx = Some(tx.clone());
                let key = self.get_key(StoreKey::Instance(instance_id));
                self.store.set(key, txs)?;
                break;
            }
        }

        Ok(())
    }

    fn add_in_progress_instance_tx(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        block_height: BlockHeight,
    ) -> Result<()> {
        let key = self.get_key(StoreKey::Instance(instance_id));

        let mut txs = self
            .store
            .get::<&str, Vec<TransactionInfo>>(&key)
            .context(format!(
                "Failed to retrieve instance with ID {}",
                instance_id
            ))?
            .unwrap_or_default();

        if let Some(tx) = txs.iter_mut().find(|x| x.tx_id == *tx_id) {
            tx.deliver_block_height = Some(block_height);
            tx.status = TransactionStatus::Sent;
        }

        self.store.set(key, txs)?;

        Ok(())
    }

    fn add_instance_tx_hex(
        &self,
        instance_id: InstanceId,
        tx_id: Txid,
        tx_hex: String,
    ) -> Result<()> {
        let mut txs = self.get_instance(instance_id)?;
        let tx_index = txs
            .iter_mut()
            .position(|tx| tx.tx_id == tx_id)
            .expect("Transaction not found in instance");
        txs[tx_index].tx_hex = Some(tx_hex);
        let instance_key = self.get_key(StoreKey::Instance(instance_id));
        self.store.set(instance_key, txs)?;
        Ok(())
    }

    fn get_funding_tx(&self, instance_id: InstanceId) -> Result<Option<FundingTx>> {
        let funding_tx_key = self.get_key(StoreKey::InstanceFundingList(instance_id));
        let funding_txs = self
            .store
            .get::<&str, Vec<FundingTx>>(&funding_tx_key)
            .context("Failed to retrieve funding transaction")?
            .unwrap_or_default();

        if let Some(last_funding_tx) = funding_txs.last() {
            Ok(Some(last_funding_tx.clone()))
        } else {
            Ok(None)
        }
    }

    fn add_funding_tx(&self, instance_id: InstanceId, funding_tx: &FundingTx) -> Result<()> {
        let funding_tx_key = self.get_key(StoreKey::InstanceFundingList(instance_id));

        let mut funding_txs = self
            .store
            .get::<&str, Vec<FundingTx>>(&funding_tx_key)
            .context("Failed to retrieve funding transaction")?
            .unwrap_or_default();

        funding_txs.push(funding_tx.clone());

        self.store
            .set(&funding_tx_key, &funding_txs)
            .context("Failed to save funding transaction")?;

        Ok(())
    }

    fn get_speed_up_txs_for_child(
        &self,
        instance_id: InstanceId,
        child_tx_id: &Txid,
    ) -> Result<Vec<SpeedUpTx>> {
        let speed_up_tx_key = self.get_key(StoreKey::InstanceSpeedUpList(instance_id));

        // Retrieve the speed up transactions from the storage
        let mut speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)
            .context("Failed to retrieve speed up transactions")?
            .unwrap_or_default();

        speed_up_txs.retain(|t| t.child_tx_id == *child_tx_id);

        Ok(speed_up_txs)
    }

    fn get_speed_up_tx(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<Option<SpeedUpTx>> {
        let speed_up_tx_key = self.get_key(StoreKey::InstanceSpeedUpList(instance_id));

        // Retrieve the list of speed up transactions from storage
        let speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)
            .context("Failed to retrieve speed up transactions")?
            .unwrap_or_default();

        // Find the specific speed up transaction that matches the given tx_id
        let speed_up_tx = speed_up_txs.into_iter().find(|t| t.tx_id == *tx_id);

        Ok(speed_up_tx)
    }

    // This function adds a new speed up transaction to the list of speed up transactions associated with an instance.
    // Speed up transactions are stored in a list, with the most recent transaction added to the end of the list.
    // This design ensures that if the last transaction in the list is pending, there cannot be another pending speed up transaction
    // for the same instance, except for one that is specifically related to the same child transaction.
    fn add_speed_up_tx(&self, instance_id: InstanceId, speed_up_tx: &SpeedUpTx) -> Result<()> {
        let speed_up_tx_key = self.get_key(StoreKey::InstanceSpeedUpList(instance_id));

        // Retrieve the current list of speed up transactions for the instance from storage.
        let mut speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)
            .context("Failed to retrieve speed up transactions")?
            .unwrap_or_default();

        // Add the newly created speed up transaction to the end of the list.
        speed_up_txs.push(speed_up_tx.clone());

        // Save the updated list of speed up transactions back to storage.
        self.store.set(&speed_up_tx_key, &speed_up_txs)?;

        Ok(())
    }

    fn is_speed_up_tx(&self, instance_id: u32, tx_id: Txid) -> Result<bool> {
        let speed_up_tx = self.get_speed_up_tx(instance_id, &tx_id)?;
        Ok(speed_up_tx.is_some())
    }

    fn update_instance_tx_status(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        status: TransactionStatus,
    ) -> Result<()> {
        self.update_instance_tx_status(instance_id, tx_id, status)
    }
}
#[automock]
pub trait StepHandlerApi {
    fn get_tx_to_answer(&self, instance_id: InstanceId, tx_id: Txid)
        -> Result<Option<Transaction>>;

    fn set_tx_to_answer(&self, instance_id: InstanceId, tx_id: Txid, tx: Transaction)
        -> Result<()>;

    fn update_instance_tx_status(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        status: TransactionStatus,
    ) -> Result<()>;

    fn get_txs_info(
        &self,
        status: TransactionStatus,
    ) -> Result<Vec<(InstanceId, Vec<TransactionInfo>)>>;
}

impl StepHandlerApi for BitvmxStore {
    fn get_tx_to_answer(
        &self,
        instance_id: InstanceId,
        tx_id: Txid,
    ) -> Result<Option<Transaction>> {
        let key = format!("instance/{}/tx/{}", instance_id, tx_id);

        let tx = self
            .store
            .get::<&str, Transaction>(&key)
            .context("Failed to retrieve instance txs to send")?;

        Ok(tx)
    }

    fn set_tx_to_answer(
        &self,
        instance_id: InstanceId,
        tx_id: Txid,
        tx: Transaction,
    ) -> Result<()> {
        let key = format!("instance/{}/tx/{}", instance_id, tx_id);

        self.store
            .set::<&str, Transaction>(&key, tx)
            .context("Failed to save instance tx to answer")?;

        Ok(())
    }

    fn update_instance_tx_status(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        status: TransactionStatus,
    ) -> Result<()> {
        self.update_instance_tx_status(instance_id, tx_id, status)
    }

    fn get_txs_info(
        &self,
        status: TransactionStatus,
    ) -> Result<Vec<(InstanceId, Vec<TransactionInfo>)>> {
        self.get_txs_info(status)
    }
}

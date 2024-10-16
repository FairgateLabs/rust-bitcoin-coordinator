use crate::types::{
    BitvmxInstance, DeliverData, FundingTx, InstanceId, SpeedUpTx, TransactionInfo,
    TransactionInfoSummary,
};
use anyhow::{Context, Ok, Result};
use bitcoin::{Amount, Transaction, Txid};
use bitvmx_transaction_monitor::types::BlockHeight;
use std::path::PathBuf;
use storage_backend::storage::{KeyValueStore, Storage};
pub struct BitvmxStore {
    store: Storage,
}

enum StoreKey {
    Instance(InstanceId),
    InstanceList,
    PendingList,
    CompletedList,
    InProgressInstanceTx(InstanceId, Txid),
    InstanceFundingTxList(InstanceId),
    SpeedUpList(InstanceId),
}

pub trait BitvmxApi:
    InstanceApi + PendingApi + InProgressApi + CompletedApi + FundingApi + SpeedUpApi
{
}

pub trait InstanceApi {
    fn tx_exists(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<bool>;
    fn get_instances(&self) -> Result<Vec<BitvmxInstance<TransactionInfo>>>;
    fn get_instance(
        &self,
        instance_id: InstanceId,
    ) -> Result<Option<BitvmxInstance<TransactionInfo>>>;
    fn get_instance_tx(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<TransactionInfo>>;
    fn get_instance_txs(
        &self,
        instance_id: InstanceId,
        tx_id: Vec<Txid>,
    ) -> Result<Vec<TransactionInfo>>;
    fn add_instance(&self, instance: &BitvmxInstance<TransactionInfoSummary>) -> Result<()>;
    fn add_instance_tx(&self, instance_id: InstanceId, tx_info: &TransactionInfo) -> Result<()>;
    fn add_tx_to_instance(&self, instance_id: InstanceId, tx: &Transaction) -> Result<()>;
    fn remove_instance(&self, instance_id: InstanceId) -> Result<()>;
}

pub trait PendingApi {
    fn get_pending_instance_txs(&self) -> Result<Vec<(InstanceId, Transaction)>>;
    fn add_pending_instance_tx(&self, instance_id: InstanceId, tx: &Transaction) -> Result<()>;
    fn remove_pending_instance_tx(&self, instance_id: InstanceId, tx: &Txid) -> Result<()>;
}

pub trait InProgressApi {
    fn get_in_progress_txs(
        &self,
        instance: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<TransactionInfo>>;
    fn add_in_progress_instance_tx(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        fee_rate: Amount,
        block_height: BlockHeight,
    ) -> Result<()>;

    fn remove_in_progress_instance_tx(&self, instance_id: InstanceId, tx: &Txid) -> Result<()>;
}

pub trait CompletedApi {
    fn add_completed_instance_tx(&self, instance: InstanceId, tx: &Txid) -> Result<()>;
    fn get_completed_instance_txs(&self) -> Result<Vec<Txid>>;
}
pub trait FundingApi {
    fn get_funding_tx(&self, instance_id: InstanceId) -> Result<Option<FundingTx>>;
    fn add_funding_tx(&self, instance_id: InstanceId, tx: &FundingTx) -> Result<()>;
}

pub trait SpeedUpApi {
    fn get_speed_up_txs_for_child(
        &self,
        instance_id: InstanceId,
        child_tx_id: &Txid,
    ) -> Result<Vec<SpeedUpTx>>;

    fn add_speed_up_tx(&self, instance_id: InstanceId, speed_up_tx: &SpeedUpTx) -> Result<()>;

    fn get_speed_up_tx(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<Option<SpeedUpTx>>;

    fn is_tx_a_speed_up_tx(&self, instance_id: u32, tx_id: Txid) -> Result<bool>;
}

impl BitvmxStore {
    pub fn new_with_path(store_path: &str) -> Result<Self> {
        let store = Storage::new_with_path(&PathBuf::from(store_path.to_string()))
            .context("There is an error creating storage in BitvmxStore")?;
        Ok(Self { store })
    }

    fn get_key(&self, key: StoreKey) -> String {
        match key {
            StoreKey::Instance(instance_id) => format!("instance/{}", instance_id),
            StoreKey::InstanceList => "instance/list".to_string(),
            StoreKey::PendingList => "pending/list".to_string(),
            StoreKey::CompletedList => "completed/list".to_string(),

            StoreKey::InProgressInstanceTx(instance_id, tx_id) => {
                format!("in_progress/instance/{}/tx/{}", instance_id, tx_id)
            }
            StoreKey::InstanceFundingTxList(instance_id) => {
                format!("instance/{}/funding/list", instance_id)
            }
            StoreKey::SpeedUpList(instance_id) => {
                format!("instance/{}/list", instance_id)
            }
        }
    }
}

impl InstanceApi for BitvmxStore {
    fn get_instance(
        &self,
        instance_id: InstanceId,
    ) -> Result<Option<BitvmxInstance<TransactionInfo>>> {
        let instance_key = self.get_key(StoreKey::Instance(instance_id));
        let instance = self
            .store
            .get::<&str, BitvmxInstance<TransactionInfo>>(&instance_key)
            .context(format!(
                "Failed to retrieve instance with ID {}",
                instance_id
            ))?;

        Ok(instance)
    }

    fn get_instances(&self) -> Result<Vec<BitvmxInstance<TransactionInfo>>> {
        let instances_list_key = self.get_key(StoreKey::InstanceList);

        let all_instance_ids = self
            .store
            .get::<&str, Vec<u32>>(&instances_list_key)
            .context("Failed to retrieve instances")?
            .unwrap_or_default();

        let mut instances = Vec::<BitvmxInstance<TransactionInfo>>::new();

        for id in all_instance_ids {
            if let Some(instance) = self.get_instance(id)? {
                instances.push(instance);
            }
        }

        Ok(instances)
    }

    fn add_instance(&self, instance: &BitvmxInstance<TransactionInfoSummary>) -> Result<()> {
        // Construct a new BitvmxInstance with detailed transaction information.
        let full_instance = BitvmxInstance::<TransactionInfo> {
            instance_id: instance.instance_id,
            txs: instance
                .txs
                .iter()
                .map(|tx| TransactionInfo {
                    tx_id: tx.tx_id,
                    owner_operator_id: tx.owner_operator_id,
                    deliver_data: None,
                    tx: None,
                })
                .collect(),
            // Clone the funding transaction from the instance to associate it with the full instance.
            funding_tx: instance.funding_tx.clone(),
        };

        let instance_key = self.get_key(StoreKey::Instance(instance.instance_id));

        // Map BitvmxInstance
        // 1. Store the instance under its ID
        self.store
            .set(&instance_key, full_instance.clone())
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

        let speed_up_tx_key = self.get_key(StoreKey::SpeedUpList(instance_id));

        self.store
            .delete(&speed_up_tx_key)
            .context("Failed to delete speed up transactions for instance")?;

        let speed_up_txs_key = self.get_key(StoreKey::SpeedUpList(instance_id));
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
        let instance = self.get_instance(instance_id)?;

        if let Some(instance) = instance {
            for tx in &instance.txs {
                if tx.tx_id == *tx_id {
                    return Ok(Some(tx.clone()));
                }
            }
        }

        Ok(None)
    }

    fn add_instance_tx(&self, instance_id: InstanceId, tx_info: &TransactionInfo) -> Result<()> {
        let instance_data = self.get_instance(instance_id)?;

        match instance_data {
            Some(mut instance) => {
                // Check if the transaction already exists in the instance
                if instance.txs.iter().any(|tx| tx.tx_id == tx_info.tx_id) {
                    println!("asdadasdasdasdasdasdasd");
                    return Ok(()); // Transaction already exists, do not add again
                } else {
                    instance.txs.push(tx_info.clone());

                    // Update the instance data in storage with the new transaction
                    let key = self.get_key(StoreKey::Instance(instance_id));
                    self.store.set(key, instance)?;

                    return Ok(());
                }
            }
            None => {
                return Err(anyhow::anyhow!("Instance does not exist"));
            }
        }
    }

    fn get_instance_txs(
        &self,
        instance_id: InstanceId,
        tx_ids: Vec<Txid>,
    ) -> Result<Vec<TransactionInfo>> {
        let mut instance_txs = vec![];

        for tx_id in tx_ids {
            let tx_instance = self.get_instance_tx(instance_id, &tx_id)?;
            if let Some(tx_inst) = tx_instance {
                instance_txs.push(tx_inst)
            }
        }

        Ok(instance_txs)
    }

    fn tx_exists(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<bool> {
        let tx_instance = self.get_instance_tx(instance_id, tx_id)?;
        Ok(tx_instance.is_some())
    }

    fn add_tx_to_instance(&self, instance_id: InstanceId, tx: &Transaction) -> Result<()> {
        let mut instance = self
            .get_instance(instance_id)?
            .ok_or(anyhow::anyhow!("Instance does not exist"))?;

        let tx_id = tx.compute_txid();

        for tx_instance in instance.txs.iter_mut() {
            if tx_instance.tx_id == tx_id {
                tx_instance.tx = Some(tx.clone());
                let key = self.get_key(StoreKey::Instance(instance_id));
                println!("XXX {:?}", instance);
                self.store.set(key, instance)?;
                break;
            }
        }

        Ok(())
    }
}

impl InProgressApi for BitvmxStore {
    fn get_in_progress_txs(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<TransactionInfo>> {
        let pending_key = self.get_key(StoreKey::InProgressInstanceTx(instance_id, *tx_id));
        let pending_instance_tx = self.store.get::<&str, TransactionInfo>(&pending_key)?;
        Ok(pending_instance_tx)
    }

    fn add_in_progress_instance_tx(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        fee_rate: Amount,
        block_height: BlockHeight,
    ) -> Result<()> {
        // Transaction should exist in storage.
        let mut tx_instance = self.get_instance_tx(instance_id, tx_id)?.unwrap();

        tx_instance.deliver_data = Some(DeliverData {
            fee_rate,
            block_height,
        });

        let pending_tx_key = self.get_key(StoreKey::InProgressInstanceTx(instance_id, *tx_id));
        self.store.set(pending_tx_key, tx_instance)?;

        Ok(())
    }

    fn remove_in_progress_instance_tx(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<()> {
        // 1. Remove the tx from the specific instance's in-progress list
        let instance_tx_key = self.get_key(StoreKey::InProgressInstanceTx(instance_id, *tx_id));
        self.store.delete(&instance_tx_key)?;

        Ok(())
    }
}

impl PendingApi for BitvmxStore {
    fn get_pending_instance_txs(&self) -> Result<Vec<(InstanceId, Transaction)>> {
        let pending_list_key = self.get_key(StoreKey::PendingList);

        let pending_instance_tx = self
            .store
            .get::<&str, Vec<(InstanceId, Transaction)>>(&pending_list_key)
            .context("Failed to retrieve instance list")?;

        match pending_instance_tx {
            Some(pendings) => Ok(pendings),
            None => Ok(vec![]),
        }
    }

    fn add_pending_instance_tx(&self, instance_id: InstanceId, tx: &Transaction) -> Result<()> {
        let mut pending_list = self.get_pending_instance_txs()?;

        // Check if the instance and tx already exist in the pending list
        let existing_index = pending_list.iter().position(|(id, existing_tx)| {
            *id == instance_id && existing_tx.compute_txid() == tx.compute_txid()
        });

        if let Some(index) = existing_index {
            // If it exists, override the transaction
            pending_list[index] = (instance_id, tx.clone());
        } else {
            // If it doesn't exist, add it to the array
            pending_list.push((instance_id, tx.clone()));
        }

        let pending_list_key = self.get_key(StoreKey::PendingList);

        // Save the updated pending list back to the store
        self.store
            .set(pending_list_key, &pending_list)
            .context("Failed to update pending list")?;

        Ok(())
    }

    fn remove_pending_instance_tx(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<()> {
        // Retrieve the current pending list
        let mut pending_list = self.get_pending_instance_txs()?;

        // Remove the specified transaction
        pending_list.retain(|(inst_id, tx)| inst_id == &instance_id && tx.compute_txid() == *tx_id);

        // Save the updated pending list back to the store
        let pending_list_key = self.get_key(StoreKey::PendingList);

        self.store
            .set(pending_list_key, &pending_list)
            .context("Failed to update pending list after removal")?;

        Ok(())
    }
}

impl CompletedApi for BitvmxStore {
    fn get_completed_instance_txs(&self) -> Result<Vec<Txid>> {
        let instance_tx_key = self.get_key(StoreKey::CompletedList);

        let result = self
            .store
            .get::<&str, Vec<Txid>>(&instance_tx_key)
            .context("Failed to retrieve completed instance transactions")?;

        match result {
            Some(txids) => Ok(txids),
            None => Ok(Vec::new()),
        }
    }

    fn add_completed_instance_tx(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<()> {
        let mut completed_txs = self.get_completed_instance_txs()?;

        completed_txs.push(*tx_id);

        let instance_tx_key = self.get_key(StoreKey::CompletedList);

        self.store
            .set(instance_tx_key, &completed_txs)
            .context("Failed to add completed instance transaction")?;

        // Remove the transaction from the pending and in-progress lists
        self.remove_pending_instance_tx(instance_id, tx_id)?;
        self.remove_in_progress_instance_tx(instance_id, tx_id)?;

        Ok(())
    }
}

impl FundingApi for BitvmxStore {
    fn get_funding_tx(&self, instance_id: InstanceId) -> Result<Option<FundingTx>> {
        let funding_tx_key = self.get_key(StoreKey::InstanceFundingTxList(instance_id));
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
        let funding_tx_key = self.get_key(StoreKey::InstanceFundingTxList(instance_id));

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
}

impl SpeedUpApi for BitvmxStore {
    fn get_speed_up_txs_for_child(
        &self,
        instance_id: InstanceId,
        child_tx_id: &Txid,
    ) -> Result<Vec<SpeedUpTx>> {
        let speed_up_tx_key = self.get_key(StoreKey::SpeedUpList(instance_id));

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
        let speed_up_tx_key = self.get_key(StoreKey::SpeedUpList(instance_id));

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
        let speed_up_tx_key = self.get_key(StoreKey::SpeedUpList(instance_id));

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

    fn is_tx_a_speed_up_tx(&self, instance_id: u32, tx_id: Txid) -> Result<bool> {
        let speed_up_tx = self.get_speed_up_tx(instance_id, &tx_id)?;
        Ok(speed_up_tx.is_some())
    }
}

use crate::types::{
    BitvmxInstance, DeliverData, FundingTx, InstanceId, TransactionInstance,
    TransactionInstanceSummary,
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
    InstanceTx(InstanceId, Txid),
    InstanceList,
    PendingList,
    CompletedList,
    InProgressInstanceTx(InstanceId, Txid),
    InstanceFundingTxList(InstanceId),
    SpeedUpTxList(InstanceId, Txid),
}

pub trait BitvmxApi:
    InstanceApi + PendingApi + InProgressApi + CompletedApi + FundingApi + SpeedUpApi
{
}

pub trait InstanceApi {
    fn tx_exists(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<bool>;
    fn get_instances(&self) -> Result<Vec<BitvmxInstance<TransactionInstance>>>;
    fn get_instance(
        &self,
        instance_id: InstanceId,
    ) -> Result<Option<BitvmxInstance<TransactionInstance>>>;
    fn get_instance_tx(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<TransactionInstance>>;
    fn get_instance_txs(
        &self,
        instance_id: InstanceId,
        tx_id: Vec<Txid>,
    ) -> Result<Vec<TransactionInstance>>;
    fn add_instance(&self, instance: &BitvmxInstance<TransactionInstanceSummary>) -> Result<()>;
    fn add_instance_tx(&self, instance_id: InstanceId, tx_info: &TransactionInstance)
        -> Result<()>;
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
    ) -> Result<Option<TransactionInstance>>;
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
    fn get_speed_up_txs(
        &self,
        instance_id: InstanceId,
        child_tx_id: Txid,
    ) -> Result<Vec<TransactionInstance>>;

    fn add_speed_up_tx(
        &self,
        instance_id: InstanceId,
        speed_up_tx: &TransactionInstance,
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
            StoreKey::Instance(instance_id) => format!("instance/{}", instance_id),
            StoreKey::InstanceTx(instance_id, tx_id) => {
                format!("instance/{}/tx/{}", instance_id, tx_id)
            }
            StoreKey::InstanceList => "instance/list".to_string(),
            StoreKey::PendingList => "pending/list".to_string(),
            StoreKey::CompletedList => "completed/list".to_string(),

            StoreKey::InProgressInstanceTx(instance_id, tx_id) => {
                format!("in_progress/instance/{}/tx/{}", instance_id, tx_id)
            }
            StoreKey::InstanceFundingTxList(instance_id) => {
                format!("instance/{}/funding/list", instance_id)
            }
            StoreKey::SpeedUpTxList(instance_id, tx_id) => {
                format!("instance/{}/child_tx/{}/speed_up_list", instance_id, tx_id)
            }
        }
    }
}

impl InstanceApi for BitvmxStore {
    fn get_instance(
        &self,
        instance_id: InstanceId,
    ) -> Result<Option<BitvmxInstance<TransactionInstance>>> {
        let instance_key = self.get_key(StoreKey::Instance(instance_id));
        let instance = self
            .store
            .get::<&str, BitvmxInstance<TransactionInstance>>(&instance_key)
            .context(format!(
                "Failed to retrieve instance with ID {}",
                instance_id
            ))?;

        Ok(instance)
    }

    fn get_instances(&self) -> Result<Vec<BitvmxInstance<TransactionInstance>>> {
        let instances_list_key = self.get_key(StoreKey::InstanceList);

        let all_instance_ids = self
            .store
            .get::<&str, Vec<u32>>(&instances_list_key)
            .context("Failed to retrieve instances")?
            .unwrap_or_default();

        let mut instances = Vec::<BitvmxInstance<TransactionInstance>>::new();

        for id in all_instance_ids {
            if let Some(instance) = self.get_instance(id)? {
                instances.push(instance);
            }
        }

        Ok(instances)
    }

    fn add_instance(&self, instance: &BitvmxInstance<TransactionInstanceSummary>) -> Result<()> {
        // Construct a new BitvmxInstance with detailed transaction information.
        let full_instance = BitvmxInstance::<TransactionInstance> {
            instance_id: instance.instance_id,
            txs: instance
                .txs
                .iter()
                .map(|tx| TransactionInstance {
                    tx_id: tx.tx_id,
                    owner_operator_id: tx.owner_operator_id,
                    deliver_data: None,
                    speed_up_data: None,
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

        for tx in &full_instance.txs {
            let instance_tx_key =
                self.get_key(StoreKey::InstanceTx(instance.instance_id, tx.tx_id));
            self.store.set(&instance_tx_key, tx).context(format!(
                "Failed to store instance transaction {} under key {}",
                tx.tx_id, instance_tx_key
            ))?;
        }

        Ok(())
    }

    fn remove_instance(&self, instance_id: InstanceId) -> Result<()> {
        let instance_key = self.get_key(StoreKey::Instance(instance_id));
        self.store.delete(&instance_key)?;

        let instances_key = self.get_key(StoreKey::InstanceList);
        let mut all_instance_ids = self
            .store
            .get::<_, Vec<u32>>(&instances_key)?
            .unwrap_or_default();

        all_instance_ids.retain(|&id| id != instance_id);
        self.store.set(&instances_key, &all_instance_ids)?;

        Ok(())
    }

    fn get_instance_tx(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<TransactionInstance>> {
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

    fn add_instance_tx(
        &self,
        instance_id: InstanceId,
        tx_info: &TransactionInstance,
    ) -> Result<()> {
        let instance_data = self.get_instance(instance_id)?;

        match instance_data {
            Some(mut instance) => {
                // Check if the transaction already exists in the instance
                if instance.txs.iter().any(|tx| tx.tx_id == tx_info.tx_id) {
                    return Ok(()); // Transaction already exists, do not add again
                } else {
                    instance.txs.push(tx_info.clone());

                    // Update the instance data in storage with the new transaction
                    let key = self.get_key(StoreKey::Instance(instance_id));
                    self.store.set(key, instance)?;

                    // Store the transaction instance separately for easy retrieval
                    let key = self.get_key(StoreKey::InstanceTx(instance_id, tx_info.tx_id));
                    self.store.set(key, tx_info)?;

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
    ) -> Result<Vec<TransactionInstance>> {
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
        let mut tx_instance = self
            .get_instance_tx(instance_id, &tx.compute_txid())?
            .ok_or(anyhow::anyhow!(
                "No instance found for transaction ID: {} and instance ID: {}",
                tx.compute_txid(),
                instance_id
            ))?;

        tx_instance.tx = Some(tx.clone());

        let key = self.get_key(StoreKey::InstanceTx(instance_id, tx_instance.tx_id));
        self.store.set(key, tx_instance)?;

        Ok(())
    }
}

impl InProgressApi for BitvmxStore {
    fn get_in_progress_txs(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<TransactionInstance>> {
        let pending_key = self.get_key(StoreKey::InProgressInstanceTx(instance_id, *tx_id));
        let pending_instance_tx = self.store.get::<&str, TransactionInstance>(&pending_key)?;
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
    fn get_speed_up_txs(
        &self,
        instance_id: InstanceId,
        child_tx_id: Txid,
    ) -> Result<Vec<TransactionInstance>> {
        let speed_up_tx_key = self.get_key(StoreKey::SpeedUpTxList(instance_id, child_tx_id));

        // Retrieve the speed up transactions from the storage
        let speed_up_txs: Vec<Txid> = self
            .store
            .get::<&str, Vec<Txid>>(&speed_up_tx_key)
            .context("Failed to retrieve speed up transactions")?
            .unwrap_or_default();

        // Get the instances associated with the speed up transactions
        let instances = self.get_instance_txs(instance_id, speed_up_txs)?;

        Ok(instances)
    }

    fn add_speed_up_tx(
        &self,
        instance_id: InstanceId,
        speed_up_tx: &TransactionInstance,
    ) -> Result<()> {
        // First, add the speed up transaction to the instance's transaction list.
        self.add_instance_tx(instance_id, speed_up_tx)?;

        // Next, update the list of speed up transaction IDs associated with the child transaction.
        let child_tx_id = speed_up_tx.speed_up_data.clone().unwrap().child_tx_id;
        let speed_up_tx_key = self.get_key(StoreKey::SpeedUpTxList(instance_id, child_tx_id));
        let mut speed_up_txs: Vec<Txid> = self
            .store
            .get::<&str, Vec<Txid>>(&speed_up_tx_key)
            .context("Failed to retrieve speed up transactions")?
            .unwrap_or_default();

        // Append the ID of the newly added speed up transaction to the list.
        speed_up_txs.push(speed_up_tx.tx_id);

        // Save the updated list of speed up transaction IDs back to storage.
        self.store.set(&speed_up_tx_key, &speed_up_txs)?;

        Ok(())
    }
}

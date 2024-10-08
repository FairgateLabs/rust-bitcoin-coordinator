use crate::types::{
    BitvmxInstance, DeliverData, FundingTx, InProgressSpeedUpTx, InProgressTx, InstanceId,
};
use anyhow::{bail, Context, Ok, Result};
use bitcoin::{Amount, Transaction, Txid};
use bitvmx_transaction_monitor::types::BlockHeight;
use std::path::PathBuf;
use storage_backend::storage::{KeyValueStore, Storage};
pub struct BitvmxStore {
    store: Storage,
}

enum StoreKey<'a> {
    Instance(InstanceId),
    InstanceList,

    PendingList,

    InProgressInstanceTx(InstanceId, &'a Txid),

    FundingInstance(InstanceId),

    CompletedInstanceTxs(InstanceId),
}

pub trait BitvmxApi: InstanceApi + PendingApi + InProgressApi + CompletedApi + FundingApi {}

pub trait InstanceApi {
    fn get_instances(&self) -> Result<Vec<BitvmxInstance>>;
    fn get_instance(&self, instance_id: InstanceId) -> Result<Option<BitvmxInstance>>;
    fn add_instance(&self, instance: &BitvmxInstance) -> Result<()>;
    fn remove_instance(&self, instance_id: InstanceId) -> Result<()>;
}

pub trait PendingApi {
    fn get_pending_list(&self) -> Result<Vec<(InstanceId, Transaction)>>;
    fn add_pending_instance_tx(&self, instance_id: InstanceId, tx: Transaction) -> Result<()>;
    fn remove_pending_instance_tx(&self, instance_id: InstanceId, tx: &Txid) -> Result<()>;
}

pub trait InProgressApi {
    fn get_in_progress_instance_tx(
        &self,
        instance: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<InProgressTx>>;
    fn add_in_progress_instance_tx(
        &self,
        instance_id: InstanceId,
        tx: &Transaction,
        fee_rate: Amount,
        block_height: BlockHeight,
    ) -> Result<()>;
    fn update_in_progress_instance_tx_speed_up(
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
    fn get_completed_instance_txs(&self, instance_id: InstanceId) -> Result<Vec<Txid>>;
}
pub trait FundingApi {
    fn get_funding_tx(&self, instance_id: InstanceId) -> Result<Option<FundingTx>>;
    fn replace_funding_tx(&self, instance_id: InstanceId, tx: &FundingTx) -> Result<()>;
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

            StoreKey::PendingList => "pending".to_string(),

            StoreKey::InProgressInstanceTx(instance_id, tx_id) => {
                format!("in_progress/instance/{}/tx/{}", instance_id, tx_id)
            }
            StoreKey::FundingInstance(instance_id) => {
                format!("in_progress/instance/{}", instance_id)
            }
            StoreKey::CompletedInstanceTxs(instance_id) => {
                format!("completed/instance/{}/txs", instance_id)
            }
        }
    }
}

impl InstanceApi for BitvmxStore {
    fn get_instance(&self, instance_id: InstanceId) -> Result<Option<BitvmxInstance>> {
        let instance_key = self.get_key(StoreKey::Instance(instance_id));
        let instance = self
            .store
            .get::<&str, BitvmxInstance>(&instance_key)
            .context(format!(
                "Failed to retrieve instance with ID {}",
                instance_id
            ))?;

        Ok(instance)
    }

    fn get_instances(&self) -> Result<Vec<BitvmxInstance>> {
        let instances_list_key = self.get_key(StoreKey::InstanceList);

        let all_instance_ids = self
            .store
            .get::<&str, Vec<u32>>(&instances_list_key)
            .context("Failed to retrieve instances")?
            .unwrap_or_default();

        let mut instances = Vec::<BitvmxInstance>::new();

        for id in all_instance_ids {
            if let Some(instance) = self.get_instance(id)? {
                instances.push(instance);
            }
        }

        Ok(instances)
    }

    fn add_instance(&self, instance: &BitvmxInstance) -> Result<()> {
        let instance_key = self.get_key(StoreKey::Instance(instance.instance_id));

        // 1. Store the instance under its ID
        self.store.set(&instance_key, instance).context(format!(
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
}

impl InProgressApi for BitvmxStore {
    fn get_in_progress_instance_tx(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<Option<InProgressTx>> {
        let pending_key = self.get_key(StoreKey::InProgressInstanceTx(instance_id, tx_id));
        let pending_instance_tx = self.store.get::<&str, InProgressTx>(&pending_key)?;
        Ok(pending_instance_tx)
    }

    fn add_in_progress_instance_tx(
        &self,
        instance_id: InstanceId,
        tx: &Transaction,
        fee_rate: Amount,
        block_height: BlockHeight,
    ) -> Result<()> {
        let in_progress_tx = InProgressTx {
            tx_id: tx.clone(),
            deliver_data: DeliverData {
                fee_rate,
                block_height,
            },
            speed_up_txs: vec![],
        };
        let tx_id = tx.compute_txid();
        let pending_tx_key = self.get_key(StoreKey::InProgressInstanceTx(instance_id, &tx_id));
        self.store.set(pending_tx_key, in_progress_tx)?;

        Ok(())
    }

    fn remove_in_progress_instance_tx(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<()> {
        // 1. Remove the tx from the specific instance's in-progress list
        let instance_tx_key = self.get_key(StoreKey::InProgressInstanceTx(instance_id, tx_id));
        self.store.delete(&instance_tx_key)?;

        Ok(())
    }

    fn update_in_progress_instance_tx_speed_up(
        &self,
        instance_id: InstanceId,
        child_tx_id: &Txid,
        fee_rate: Amount,
        block_height: BlockHeight,
    ) -> Result<()> {
        let pending_tx_key = self.get_key(StoreKey::InProgressInstanceTx(instance_id, child_tx_id));

        if let Some(mut pending_tx) = self.store.get::<_, InProgressTx>(&pending_tx_key)? {
            // Every time we want to update an instance in progress means the transaction was speed up.

            // Check if speed_up_txs is empty and initialize it if so
            if pending_tx.speed_up_txs.is_empty() {
                pending_tx.speed_up_txs = Vec::new();
            }

            // Push the new InProgressSpeedUpTx to the speed_up_txs vector
            pending_tx.speed_up_txs.push(InProgressSpeedUpTx {
                deliver_data: DeliverData {
                    fee_rate,
                    block_height,
                },
                child_tx_id: *child_tx_id,
            });

            self.store
                .set(&pending_tx_key, &pending_tx)
                .context("Failed to update in-progress instance tx")?;

            Ok(())
        } else {
            bail!("In-progress transaction not found");
        }
    }
}

impl PendingApi for BitvmxStore {
    fn get_pending_list(&self) -> Result<Vec<(InstanceId, Transaction)>> {
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

    fn add_pending_instance_tx(&self, instance_id: InstanceId, tx: Transaction) -> Result<()> {
        let mut pending_list = self.get_pending_list()?;

        // Check if the instance and tx already exist in the pending list
        let existing_index = pending_list.iter().position(|(id, existing_tx)| {
            *id == instance_id && existing_tx.compute_txid() == tx.compute_txid()
        });

        if let Some(index) = existing_index {
            // If it exists, override the transaction
            pending_list[index] = (instance_id, tx);
        } else {
            // If it doesn't exist, add it to the array
            pending_list.push((instance_id, tx));
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
        let mut pending_list = self.get_pending_list()?;

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
    fn get_completed_instance_txs(&self, instance_id: InstanceId) -> Result<Vec<Txid>> {
        let instance_tx_key = self.get_key(StoreKey::CompletedInstanceTxs(instance_id));

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
        let mut completed_txs = self.get_completed_instance_txs(instance_id)?;

        completed_txs.push(*tx_id);

        let instance_tx_key = self.get_key(StoreKey::CompletedInstanceTxs(instance_id));

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
        let funding_tx_key = self.get_key(StoreKey::FundingInstance(instance_id));
        self.store
            .get::<&str, FundingTx>(&funding_tx_key)
            .context("Failed to retrieve funding transaction")
    }

    fn replace_funding_tx(&self, instance_id: InstanceId, funding_tx: &FundingTx) -> Result<()> {
        let funding_tx_key = self.get_key(StoreKey::FundingInstance(instance_id));

        self.store
            .set(funding_tx_key, funding_tx)
            .context("Failed to add funding transaction")?;

        Ok(())
    }
}

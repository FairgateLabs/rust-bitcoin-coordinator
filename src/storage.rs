use crate::types::{FundingTx, InstanceId, PendingTx};
use anyhow::{Context, Ok, Result};
use bitcoin::{absolute::LockTime, Amount, ScriptBuf, Transaction, TxOut, Txid};
use bitvmx_transaction_monitor::types::{BitvmxInstance, BlockHeight};
use rust_bitvmx_storage_backend::storage::{KeyValueStore, Storage};
use std::{path::PathBuf, str::FromStr};
pub struct BitvmxStore {
    store: Storage,
}

enum StoreKey<'a> {
    Instance(InstanceId),
    InstanceTx(InstanceId, &'a Txid),
    InstanceList,
    InstanceNews,
}

pub trait BitvmxApi {
    // INSTANCE API
    fn get_instances(&self) -> Result<Vec<BitvmxInstance>>;
    fn get_instance(&self, instance_id: InstanceId) -> Result<Option<BitvmxInstance>>;
    fn add_instance(&self, instance: &BitvmxInstance) -> Result<()>;
    fn add_instances(&self, instances: &Vec<BitvmxInstance>) -> Result<()>;
    fn remove_instance(&self, instance_id: InstanceId) -> Result<()>;
    fn remove_instances(&self, instance_ids: Vec<InstanceId>) -> Result<()>;

    // PENDING API
    fn get_pending_instance_txs(&self) -> Result<Vec<(InstanceId, Transaction)>>;
    fn add_pending_instance_tx(&self, instance_id: InstanceId, tx: Transaction) -> Result<()>;
    fn remove_pending_instance_tx(&self, instance_id: InstanceId, tx: &Txid) -> Result<()>;

    // IN PROGREESS API
    fn get_in_progress_txs(&self) -> Result<Vec<PendingTx>>;
    fn get_in_progress_tx(&self, tx_id: &Txid) -> Result<Option<PendingTx>>;
    fn add_in_progress_tx(
        &self,
        tx: &Transaction,
        fee_rate: Amount,
        block_height: BlockHeight,
    ) -> Result<()>;
    fn update_in_progress_tx(
        &self,
        tx_id: &Txid,
        fee_rate: Amount,
        block_height: BlockHeight,
    ) -> Result<()>;
    fn remove_in_progress_instance_tx(&self, instance_id: InstanceId, tx: &Txid) -> Result<()>;

    // COMPLETED API
    fn add_completed_instance_tx(&self, instance: InstanceId, tx: &Txid) -> Result<()>;

    // FUNDING API
    fn get_funding_tx(&self) -> Result<Option<FundingTx>>;
    fn add_funding_tx(&self, tx: &Transaction) -> Result<()>;
    // This endpoint is for accounting, to know which txs have been used for speed up
    fn mark_funding_tx_as_used(&self, tx: &Txid) -> Result<()>;
}

impl BitvmxStore {
    pub fn new_with_path(store_path: &str) -> Result<Self> {
        let store = Storage::new_with_path(&PathBuf::from(format!("{}/monitor", store_path)))?;
        Ok(Self { store })
    }

    fn get_instance_key(&self, key: StoreKey) -> String {
        match key {
            StoreKey::Instance(instance_id) => format!("instance/{}", instance_id),
            StoreKey::InstanceTx(instance_id, tx_id) => {
                format!("instance/{}/tx/{}", instance_id, tx_id)
            }
            StoreKey::InstanceList => "instance/list".to_string(),
            StoreKey::InstanceNews => "instance/news".to_string(),
        }
    }
}

impl BitvmxApi for BitvmxStore {
    fn get_instance(&self, instance_id: InstanceId) -> Result<Option<BitvmxInstance>> {
        let instance_key = self.get_instance_key(StoreKey::Instance(instance_id));
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
        let instances_key = "instance/list";

        let all_instance_ids = self
            .store
            .get::<&str, Vec<u32>>(instances_key)
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
        let instance_key = format!("instance/{}", instance.id);

        // 1. Store the instance under its ID
        self.store.set(&instance_key, instance).context(format!(
            "Failed to store instance under key {}",
            instance_key
        ))?;

        // 2. Maintain the list of all instances (instance/list)
        let instances_key = "instance/list";
        let mut all_instances = self
            .store
            .get::<_, Vec<u32>>(instances_key)?
            .unwrap_or_default();

        // Add the new instance ID to the list if it's not already present
        if !all_instances.contains(&instance.id) {
            all_instances.push(instance.id);
            self.store
                .set(instances_key, &all_instances)
                .context("Failed to update instances list")?;
        }

        Ok(())
    }

    fn add_instances(&self, instances: &Vec<BitvmxInstance>) -> Result<()> {
        for instance in instances {
            self.add_instance(&instance)?;
        }
        Ok(())
    }

    fn remove_instance(&self, instance_id: InstanceId) -> Result<()> {
        let instance_key = format!("instance/{}", instance_id);
        self.store.delete(&instance_key)?;

        let instances_key = "instance/list";
        let mut all_instance_ids = self
            .store
            .get::<_, Vec<u32>>(instances_key)?
            .unwrap_or_default();

        all_instance_ids.retain(|&id| id != instance_id);
        self.store.set(instances_key, &all_instance_ids)?;

        Ok(())
    }

    fn remove_instances(&self, instance_ids: Vec<InstanceId>) -> Result<()> {
        for instance_id in instance_ids {
            self.remove_instance(instance_id)?;
        }
        Ok(())
    }

    fn get_in_progress_tx(&self, tx_id: &Txid) -> Result<Option<PendingTx>> {
        let stalled_tx_key = format!("stalled_tx/{}", tx_id);
        let stalled_tx = self.store.get::<&str, PendingTx>(&stalled_tx_key)?;

        Ok(stalled_tx)
    }

    fn get_in_progress_txs(&self) -> Result<Vec<PendingTx>> {
        let stalled_txs = "stalled_tx/list";

        let stalled_ids = self
            .store
            .get::<&str, Vec<Txid>>(stalled_txs)
            .context("Failed to retrieve stalled transactions")?
            .unwrap_or_default();

        let mut stalled_txs = Vec::<PendingTx>::new();

        for id in stalled_ids {
            if let Some(tx) = self.get_in_progress_tx(&id)? {
                stalled_txs.push(tx);
            }
        }

        Ok(stalled_txs)
    }

    fn add_in_progress_tx(
        &self,
        tx: &Transaction,
        fee_rate: Amount,
        block_height: BlockHeight,
    ) -> Result<()> {
        let stalled_tx = PendingTx {
            tx: tx.clone(),
            fee_rate,
            block_height,
        };
        let tx_id = tx.compute_txid();
        let stalled_tx_key = format!("stalled_tx/{}", tx_id);
        self.store.set(&stalled_tx_key, stalled_tx)?;

        // 2. Maintain the list of all stalled txs
        let stalled_tx_list_key = "stalled_tx/list";
        let mut all = self
            .store
            .get::<_, Vec<Txid>>(stalled_tx_list_key)?
            .unwrap_or_default();

        // Add the new tx id to the list if it's not already present
        if !all.contains(&tx_id) {
            all.push(tx_id);
            self.store
                .set(stalled_tx_list_key, &all)
                .context("Failed to update stalled txs list")?;
        }

        Ok(())
    }

    fn remove_in_progress_instance_tx(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<()> {
        let stalled_tx_key = format!("stalled_tx/{}", tx_id);
        self.store.delete(&stalled_tx_key)?;

        Ok(())
    }

    fn get_pending_instance_txs(&self) -> Result<Vec<(InstanceId, Transaction)>> {
        todo!()
    }

    fn add_pending_instance_tx(&self, instance_id: InstanceId, txs: Transaction) -> Result<()> {
        todo!()
    }

    fn remove_pending_instance_tx(&self, instance_id: InstanceId, tx: &Txid) -> Result<()> {
        todo!()
    }

    fn get_funding_tx(&self) -> Result<Option<FundingTx>> {
        todo!()
    }

    fn add_funding_tx(&self, tx: &Transaction) -> Result<()> {
        todo!()
    }

    fn mark_funding_tx_as_used(&self, tx: &Txid) -> Result<()> {
        todo!()
    }

    fn update_in_progress_tx(
        &self,
        tx_id: &Txid,
        fee_rate: Amount,
        block_height: BlockHeight,
    ) -> Result<()> {
        todo!()
    }

    fn add_completed_instance_tx(&self, instance: InstanceId, tx: &Txid) -> Result<()> {
        todo!()
    }
}

#[test]
fn instances_store() -> Result<(), anyhow::Error> {
    let tx_id = Txid::from_str(&"e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200b")
        .unwrap();

    let tx_id_2 =
        Txid::from_str(&"3a3f8d147abf0b9b9d25b07de7a16a4db96bda3e474ceab4c4f9e8e107d5b02f")
            .unwrap();

    let bitvmx_store = BitvmxStore::new_with_path("testxx")?;

    let instance = BitvmxInstance {
        id: 1,
        txs: vec![
            // FundingTx {
            //     tx_id,
            //     utxo_index: 1,
            //     utxo_output: TxOut {
            //         value: Amount::default(),
            //         script_pubkey: ScriptBuf::new(),
            //     },
            // },
            // FundingTx {
            //     tx_id: tx_id_2,
            //     utxo_index: 0,
            //     utxo_output: TxOut {
            //         value: Amount::default(),
            //         script_pubkey: ScriptBuf::new(),
            //     },
            // },
        ],
        start_height: 0,
    };

    //add instance
    bitvmx_store.add_instance(&instance)?;

    //get instances
    let instances = bitvmx_store.get_instances()?;
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].txs.len(), 2);

    //get instance by id
    let instance = bitvmx_store.get_instance(1)?;
    assert_eq!(instance.unwrap().txs.len(), 2);

    //remove instance
    bitvmx_store.remove_instance(1)?;
    let instances = bitvmx_store.get_instances()?;
    assert_eq!(instances.len(), 0);

    // get instance by id
    let instance = bitvmx_store.get_instance(1)?;
    assert!(instance.is_none());

    Ok(())
}

#[test]
fn stalled_tx_store() -> Result<(), anyhow::Error> {
    let bitvmx_store = BitvmxStore::new_with_path("testxx")?;
    let tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195600).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx2 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195601).unwrap(),
        input: vec![],
        output: vec![],
    };

    //add stalled tx 2 times, should rewrite the tx because has the same tx_id
    bitvmx_store.add_in_progress_tx(&tx, Amount::default(), 2)?;
    bitvmx_store.add_in_progress_tx(&tx2, Amount::default(), 2)?;

    //get stalled txs
    let txs = bitvmx_store.get_in_progress_txs()?;
    assert_eq!(txs.len(), 2);

    //get stalled tx by id
    let instance = bitvmx_store.get_in_progress_tx(&tx.compute_txid())?;
    assert!(instance.is_some());

    //get stalled tx by id
    let instance = bitvmx_store.get_in_progress_tx(&tx2.compute_txid())?;
    assert!(instance.is_some());

    //remove stalled tx
    bitvmx_store.remove_in_progress_instance_tx(1, &tx.compute_txid())?;
    let txs = bitvmx_store.get_in_progress_txs()?;
    assert_eq!(txs.len(), 1);

    //remove stalled tx2
    bitvmx_store.remove_in_progress_instance_tx(2, &tx2.compute_txid())?;
    let txs = bitvmx_store.get_in_progress_txs()?;
    assert_eq!(txs.len(), 0);

    Ok(())
}

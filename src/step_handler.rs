use crate::{
    orchestrator::{Orchestrator, OrchestratorApi},
    types::InstanceId,
};
use anyhow::{Context, Ok, Result};
use bitcoin::{Transaction, Txid};
use console::style;
use log::{info, trace};
use storage_backend::storage::{KeyValueStore, Storage};

pub struct StepHandler {
    orchestrator: Orchestrator,
    store: Storage,
}

pub trait StepHandlerApi {
    fn tick(&mut self) -> Result<()>;
}

impl StepHandler {
    pub fn new(orchestrator: Orchestrator, store: Storage) -> Result<Self> {
        Ok(Self {
            orchestrator,
            store,
        })
    }

    pub fn get_next_step_tx(
        &self,
        instance_id: InstanceId,
        tx_id: Txid,
    ) -> Result<Option<Transaction>> {
        let key = format!("instance/{}/tx/{}/sent", instance_id, tx_id);
        let was_sent = self.store.get::<&str, bool>(&key)?;

        if was_sent.is_none() || was_sent.unwrap() {
            return Ok(None);
        }

        let key = format!("instance/{}/tx/{}", instance_id, tx_id);

        let tx = self
            .store
            .get::<&str, Transaction>(&key)
            .context("Failed to retrieve instance txs to send")?;

        Ok(tx)
    }

    pub fn mark_tx_as_send(&self, instance_id: InstanceId, tx_id: Txid) -> Result<()> {
        let key = format!("instance/{}/tx/{}/sent", instance_id, tx_id);

        self.store
            .set(key, true)
            .context("Failed to retrieve instance txs to send")?;

        Ok(())
    }

    fn send_next_step_tx(
        &mut self,
        instance_id: InstanceId,
        tx_id: Txid,
    ) -> Result<(), anyhow::Error> {
        let tx: Option<Transaction> = self.get_next_step_tx(instance_id, tx_id)?;

        if tx.is_none() {
            trace!(
                "{} Transaction ID {} for Instance ID {} ALREADY SENT \n",
                style("Info").green(),
                style(tx_id).blue(),
                style(instance_id).green()
            );
            return Ok(());
        }

        info!(
            "{} Transaction ID {} for Instance ID {} CONFIRMED!!! \n",
            style("StepHandler").green(),
            style(tx_id).blue(),
            style(instance_id).green()
        );

        let tx: Transaction = tx.unwrap();
        self.orchestrator.send_tx_instance(instance_id, &tx)?;
        self.mark_tx_as_send(instance_id, tx_id)?;

        info!(
            "{} Transaction ID {} for Instance ID {} SENDING.... \n",
            style("StepHandler").green(),
            style(tx.compute_txid()).blue(),
            style(instance_id).red()
        );

        Ok(())
    }
}

impl StepHandlerApi for StepHandler {
    fn tick(&mut self) -> Result<()> {
        self.orchestrator
            .tick()
            .context("Failed tick orchestrator")?;

        let confirmed_txs = self.orchestrator.get_confirmed_txs()?;

        for (instance_id, txs) in confirmed_txs {
            for tx in txs {
                self.send_next_step_tx(instance_id, tx.tx_id)?;
            }
        }

        Ok(())
    }
}

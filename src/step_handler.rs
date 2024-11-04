use crate::{
    orchestrator::{Orchestrator, OrchestratorApi},
    storage::{BitvmxStore, StepHandlerApi},
    types::InstanceId,
};
use anyhow::{Context, Ok, Result};
use bitcoin::{Transaction, Txid};
use console::style;
use log::info;

pub struct StepHandler<'a> {
    orchestrator: Orchestrator,
    store: &'a BitvmxStore,
}

pub trait StepHandlerTrait {
    fn tick(&mut self) -> Result<()>;
}

impl<'a> StepHandler<'a> {
    pub fn new(orchestrator: Orchestrator, store: &'a BitvmxStore) -> Result<Self> {
        Ok(Self {
            orchestrator,
            store,
        })
    }

    fn send_next_step_tx(&self, instance_id: InstanceId, tx_id: Txid) -> Result<(), anyhow::Error> {
        info!(
            "{} Transaction ID {} for Instance ID {} CONFIRMED!!! \n",
            style("StepHandler").green(),
            style(tx_id).blue(),
            style(instance_id).green()
        );

        let tx: Option<Transaction> = self.store.get_tx_to_answer(instance_id, tx_id)?;

        if tx.is_none() {
            info!(
                "{} Transaction ID {} for Instance ID {} NO ANSWER FOUND \n",
                style("Info").green(),
                style(tx_id).blue(),
                style(instance_id).green()
            );
            return Ok(());
        }

        let tx: Transaction = tx.unwrap();
        self.orchestrator.send_tx_instance(instance_id, &tx)?;

        self.orchestrator
            .acknowledged_instance_tx(instance_id, &tx_id)?;

        Ok(())
    }
}

impl<'a> StepHandlerTrait for StepHandler<'a> {
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

use crate::{
    orchestrator::OrchestratorApi,
    storage::StepHandlerApi,
    types::{InstanceId, TransactionStatus},
};
use anyhow::{Context, Ok, Result};
use bitcoin::{Transaction, Txid};
use console::style;
use log::info;

pub struct StepHandler<'s, O, S>
where
    O: OrchestratorApi,
    S: StepHandlerApi,
{
    orchestrator: O,
    store: &'s S,
}

pub trait StepHandlerTrait {
    fn tick(&mut self) -> Result<()>;
}

impl<'s, O, S> StepHandler<'s, O, S>
where
    O: OrchestratorApi,
    S: StepHandlerApi,
{
    pub fn new(orchestrator: O, store: &'s S) -> Result<Self> {
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

        Ok(())
    }
}

impl<'s, O, S> StepHandlerTrait for StepHandler<'s, O, S>
where
    O: OrchestratorApi,
    S: StepHandlerApi,
{
    fn tick(&mut self) -> Result<()> {
        self.orchestrator
            .tick()
            .context("Failed tick orchestrator")?;

        let confirmed_txs = self.store.get_txs_info(TransactionStatus::Finalized)?;

        for (instance_id, txs) in confirmed_txs {
            for tx in txs {
                self.send_next_step_tx(instance_id, tx.tx_id)?;

                self.store.update_instance_tx_status(
                    instance_id,
                    &tx.tx_id,
                    TransactionStatus::Acknowledged,
                )?;
            }
        }

        Ok(())
    }
}

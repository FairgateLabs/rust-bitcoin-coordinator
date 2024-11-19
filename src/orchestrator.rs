use crate::{
    storage::BitvmxStoreApi,
    types::{
        BitvmxInstance, FundingTx, InstanceId, SpeedUpTx, TransactionInfo, TransactionPartialInfo,
        TransactionStatus,
    },
};
use anyhow::{Context, Ok, Result};
use bitcoin::{PublicKey, Transaction, TxOut, Txid};
use bitvmx_transaction_monitor::{
    monitor::MonitorApi,
    types::{BlockHeight, InstanceData, TxStatusResponse},
};
use console::style;
use log::info;
use transaction_dispatcher::{
    dispatcher::TransactionDispatcherApi, errors::DispatcherError, signer::Account,
};

pub struct Orchestrator<'b, M, D, B>
where
    M: MonitorApi,
    D: TransactionDispatcherApi,
    B: BitvmxStoreApi,
{
    monitor: M,
    dispatcher: D,
    store: &'b B,
    current_height: BlockHeight,
    account: Account,
}

pub trait OrchestratorApi {
    fn monitor_instance(&self, instance: &BitvmxInstance<TransactionPartialInfo>) -> Result<()>;

    // Add a non-existent transaction for an existing instance.
    // This will be use in the final step.
    fn add_tx_to_instance(&self, instance_id: InstanceId, tx_id: &Txid) -> Result<()>;

    // The protocol requires delivering an existing transaction for an instance.
    // This is achieved by passing the full transaction.
    fn send_tx_instance(&self, instance_id: InstanceId, tx: &Transaction) -> Result<()>;

    fn is_ready(&mut self) -> Result<bool>;

    fn tick(&mut self) -> Result<()>;

    fn add_funding_tx(&self, instance_id: InstanceId, funding_tx: &FundingTx) -> Result<()>;

    fn notify_insufficient_funds(&self, instance_id: InstanceId) -> Result<()>;
}

impl<'b, M, D, B> Orchestrator<'b, M, D, B>
where
    M: MonitorApi,
    D: TransactionDispatcherApi,
    B: BitvmxStoreApi,
{
    pub fn new(monitor: M, store: &'b B, dispatcher: D, account: Account) -> Result<Self> {
        Ok(Self {
            monitor,
            dispatcher,
            store,
            current_height: 0,
            account: account.clone(),
        })
    }

    fn process_pending_txs(&mut self) -> Result<()> {
        // Get pending instance transactions to be send to the blockchain
        let pending_txs = self.store.get_txs_info(TransactionStatus::ReadyToSend)?;

        info!(
            "transactions pending to be sent #{}",
            style(pending_txs.len()).yellow()
        );

        // For each pending pair
        for (instance_id, txs) in pending_txs {
            for tx_info in txs {
                info!(
                    "{} Dispatching transaction ID: {}",
                    style("Orchastrator").green(),
                    style(tx_info.tx_id).blue()
                );

                self.dispatcher
                    .send(tx_info.tx.unwrap())
                    .context("Error dispatching transaction")?;

                // TODO: check atomics transactions. to perform add and remove.

                self.store.add_in_progress_instance_tx(
                    instance_id,
                    &tx_info.tx_id,
                    self.current_height,
                )?;
            }
        }

        Ok(())
    }

    fn speed_up(
        &self,
        instance_id: InstanceId,
        tx: &Transaction,
        funding_txid: Txid,
        tx_public_key: PublicKey,
        funding_utxo: (u32, TxOut, PublicKey),
    ) -> Result<()> {
        let dispatch_result =
            self.dispatcher
                .speed_up(tx, tx_public_key, funding_txid, funding_utxo.clone());

        if let Err(error) = dispatch_result {
            match error {
                DispatcherError::InsufficientFunds => {
                    self.notify_insufficient_funds(instance_id)?;
                    return Ok(());
                }
                e => return Err(e.into()),
            }
        }

        if dispatch_result.is_ok() {
            let (speed_up_tx_id, deliver_fee_rate) = dispatch_result.unwrap();

            let speed_up_tx = SpeedUpTx {
                tx_id: speed_up_tx_id,
                deliver_fee_rate,
                deliver_block_height: self.current_height,
                child_tx_id: tx.compute_txid(),
                utxo_index: funding_utxo.0,
                utxo_output: funding_utxo.1,
            };

            self.store.add_speed_up_tx(instance_id, &speed_up_tx)?;

            self.monitor
                .save_transaction_for_tracking(instance_id, speed_up_tx_id)?;
        }

        Ok(())
    }

    fn notify_protocol_tx_changes(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
        tx_hex: &str,
    ) -> Result<()> {
        // Implement the notification logic here
        info!(
            "Found tx sent to protocol for instance_id: {:?}  tx_id: {} tx_hex {}",
            instance_id,
            style(tx_id).blue(),
            tx_hex
        );
        Ok(())
    }

    fn process_in_progress_txs(&mut self) -> Result<()> {
        let instance_txs = self.store.get_txs_info(TransactionStatus::Sent)?;
        for (instance_id, txs) in instance_txs {
            for tx in txs {
                info!(
                    "{} Processing transaction: {} for instance: {}",
                    style("→").cyan(),
                    style(tx.tx_id).blue(),
                    style(instance_id).red()
                );
                self.process_instance_tx_change(instance_id, tx.tx_id)?;
            }
        }

        Ok(())
    }

    fn process_instance_news(&mut self) -> Result<()> {
        // Get any news in each instance that are being monitored.
        // Get instances news also returns the speed ups txs added for each instance.
        let news = self.monitor.get_instance_news()?;

        for (instance_id, tx_ids) in &news {
            info!(
                "{} Instance ID: {} has new transactions: {}",
                style("News").green(),
                style(instance_id).green(),
                style(tx_ids.len()).red()
            );
        }

        for (instance_id, tx_ids) in news {
            for tx_id in tx_ids {
                let is_speed_up = self.store.is_speed_up_tx(instance_id, tx_id)?;

                if is_speed_up {
                    self.process_speed_up_change(instance_id, &tx_id)?;
                } else {
                    self.process_instance_tx_change(instance_id, tx_id)?;
                }
            }
        }

        Ok(())
    }

    fn process_speed_up_change(&mut self, instance_id: InstanceId, tx_id: &Txid) -> Result<()> {
        // This indicates that this is a speed-up transaction that has been mined with 1 confirmation,
        // which means it should be treated as the new funding transaction. // Get the transaction's status from the monitor
        let tx_status = self
            .monitor
            .get_instance_tx_status(instance_id, *tx_id)?
            .ok_or(anyhow::anyhow!(
                "No transaction status found for transaction ID: {} and instance ID: {}",
                tx_id,
                instance_id
            ))?;

        if tx_status.was_seen() {
            if tx_status.confirmations == 1 {
                self.handle_confirmation_speed_up_transaction(instance_id, tx_id)?;
            }

            if tx_status.is_orphan() {
                self.handle_orphan_speed_up_transaction(instance_id, &tx_status.tx_id)?;
            }
        }

        if !tx_status.was_seen() {
            // If a speed-up transaction has not been seen (it has not been mined), no action is required.
            // The responsibility for creating a new speed-up transaction lies with the instance transaction that is delivered.
        }

        // In the event of a reorganization, we would need to do the opposite.
        // This involves removing the speed-up transaction and potentially replacing it with another transaction
        // that could take its place as the last speed-up transaction or become the new last funding transaction.

        Ok(())
    }

    fn process_instance_tx_change(&mut self, instance_id: InstanceId, tx_id: Txid) -> Result<()> {
        // Get the transaction's status from the monitor
        let tx_status = self
            .monitor
            .get_instance_tx_status(instance_id, tx_id)?
            .ok_or(anyhow::anyhow!(
                "No transaction status found for transaction ID: {} and instance ID: {}",
                tx_id,
                instance_id
            ))?;

        let was_seen = tx_status.was_seen();

        if was_seen {
            if tx_status.confirmations == 1 {
                // If tx is orphan then confirmations should be 0.  No need to process reorgs here.

                //Confirmation in 1 means is it already included in the block.
                self.handle_confirmation_transaction(instance_id, &tx_status)?;

                return Ok(());
            }

            //TODO: use confirmation_threshold from configuration
            if tx_status.confirmations >= self.monitor.get_confirmation_threshold() {
                // Transaction was mined and has sufficient confirmations for
                // move the transaction to complete.
                self.handle_complete_transaction(instance_id, &tx_status)?;
                return Ok(());
            }

            if tx_status.is_orphan() {
                self.handle_orphan_transaction(instance_id, &tx_status)?;
            }

            return Ok(());
        }

        if !was_seen {
            // Get information for the last time the transaction was sent
            let in_progress_tx = self.store.get_instance_tx(instance_id, &tx_id)?;

            if let Some(in_progress_tx) = in_progress_tx {
                // The transaction is in progress for us, and was not seen yet in the chain.
                // It means we have to resend or speed up the tx.
                self.handle_unseen_transaction(instance_id, &in_progress_tx)?;
            }
        }

        Ok(())
    }

    fn handle_confirmation_transaction(
        &mut self,
        instance_id: InstanceId,
        tx_status: &TxStatusResponse,
    ) -> Result<()> {
        // Update the transaction to completed given that transaction has more than the threshold confirmations
        self.store.update_instance_tx_status(
            instance_id,
            &tx_status.tx_id,
            TransactionStatus::Confirmed,
        )?;

        // Acknowledge the transaction news to the monitor to update its state.
        // This step ensures that the monitor is aware of the transaction's completion and can update its tracking accordingly.
        self.monitor
            .acknowledge_instance_tx_news(instance_id, &tx_status.tx_id)?;

        Ok(())
    }

    fn handle_confirmation_speed_up_transaction(
        &mut self,
        instance_id: InstanceId,
        speed_up_tx_id: &Txid,
    ) -> Result<()> {
        let speed_up_tx = self
            .store
            .get_speed_up_tx(instance_id, speed_up_tx_id)?
            .unwrap();

        //Confirmation in 1 means the transaction is already included in the block.
        //The new transaction funding is gonna be this a speed-up transaction.
        let funding_info = FundingTx {
            tx_id: speed_up_tx.tx_id,
            utxo_index: speed_up_tx.utxo_index,
            utxo_output: speed_up_tx.utxo_output.clone(),
        };

        //TODO: There is something missing here. We are moving a speed-up transaction to a funding transaction.
        // The inverse should also be supported.
        self.store.add_funding_tx(instance_id, &funding_info)?;

        // Acknowledge the transaction news to the monitor to update its state.
        // This step ensures that the monitor is aware of the transaction's completion and can update its tracking accordingly.
        self.monitor
            .acknowledge_instance_tx_news(instance_id, &speed_up_tx.tx_id)?;

        Ok(())
    }

    fn handle_orphan_speed_up_transaction(
        &mut self,
        instance_id: InstanceId,
        speed_up_tx_id: &Txid,
    ) -> Result<()> {
        //Speed up previouly was mined, now is orphan then, we have to remove it as a funding tx.
        self.store.remove_funding_tx(instance_id, speed_up_tx_id)?;

        // Acknowledge the transaction news to the monitor to update its state.
        // This step ensures that the monitor is aware of the transaction's completion and can update its tracking accordingly.
        self.monitor
            .acknowledge_instance_tx_news(instance_id, speed_up_tx_id)?;

        Ok(())
    }

    fn handle_orphan_transaction(
        &mut self,
        instance_id: InstanceId,
        tx_status: &TxStatusResponse,
    ) -> Result<()> {
        // Transaction was mined and now is orphan.

        // Notify the protocol about the transaction changes, specifically for confirmed transactions.
        // This step is crucial for the protocol to be aware of the transaction's status and proceed accordingly.
        self.notify_protocol_tx_changes(
            instance_id,
            &tx_status.tx_id,
            &tx_status.tx_hex.clone().unwrap(),
        )?;

        let tx = self
            .store
            .get_instance_tx(instance_id, &tx_status.tx_id)?
            .unwrap();

        if tx.is_transaction_owned() {
            // Assuming this transaction is in the mempool after a reorganization,
            // it is now waiting to be included in a block again.
            self.store.update_instance_tx_status(
                instance_id,
                &tx_status.tx_id,
                TransactionStatus::Sent,
            )?;
        } else {
            self.store.update_instance_tx_status(
                instance_id,
                &tx_status.tx_id,
                TransactionStatus::Orphan,
            )?;
        }

        self.monitor
            .acknowledge_instance_tx_news(instance_id, &tx_status.tx_id)?;

        Ok(())
    }

    fn handle_complete_transaction(
        &mut self,
        instance_id: InstanceId,
        tx_status: &TxStatusResponse,
    ) -> Result<()> {
        // Transaction was mined and has sufficient confirmations to mark it as complete.

        // Notify the protocol about the transaction changes, specifically for confirmed transactions.
        // This step is crucial for the protocol to be aware of the transaction's status and proceed accordingly.
        self.notify_protocol_tx_changes(
            instance_id,
            &tx_status.tx_id,
            &tx_status.tx_hex.clone().unwrap(),
        )?;

        // Update the transaction to completed given that transaction has more than the threshold confirmations
        self.store.update_instance_tx_status(
            instance_id,
            &tx_status.tx_id,
            TransactionStatus::Finalized,
        )?;

        // Acknowledge the transaction news to the monitor to update its state.
        // This step ensures that the monitor is aware of the transaction's completion and can update its tracking accordingly.
        self.monitor
            .acknowledge_instance_tx_news(instance_id, &tx_status.tx_id)?;

        Ok(())
    }

    fn handle_unseen_transaction(
        &mut self,
        instance_id: InstanceId,
        tx_data: &TransactionInfo,
    ) -> Result<()> {
        // We get all the existing speed up transaction for tx_id. Then we figure out if we should speed it up again.
        let speed_up_txs = self
            .store
            .get_speed_up_txs_for_child(instance_id, &tx_data.tx_id)?;

        // In case there are an existing speed up we have to check if a new speed up is needed.
        // Otherwise we always speed up the transaction
        if !speed_up_txs.is_empty() {
            //Last speed up transaction should be the last created.
            let prev_fee_rate = speed_up_txs.last().unwrap().deliver_fee_rate;

            // Check if the transaction should be speed up
            if !self.dispatcher.should_speed_up(prev_fee_rate)? {
                return Ok(());
            }
        };

        //TODO: Detect every change in speed up transaction to identify which is the funding transaction.
        //TODO: It is possible to speed up just one transaction at a time. Same tx could be speed up.

        //We are gonna have a funding transaction for each Bitvmx instance.
        let funding_tx = self
            .store
            .get_funding_tx(instance_id)?
            .ok_or(anyhow::anyhow!(
                "No funding transaction available for speed up"
            ))?;

        self.speed_up(
            instance_id,
            tx_data.tx.as_ref().unwrap(),
            funding_tx.tx_id,
            self.account.pk,
            (
                funding_tx.utxo_index,
                funding_tx.utxo_output,
                self.account.pk,
            ),
        )?;

        Ok(())
    }
}

impl<'b, M, D, B> OrchestratorApi for Orchestrator<'b, M, D, B>
where
    M: MonitorApi,
    D: TransactionDispatcherApi,
    B: BitvmxStoreApi,
{
    fn tick(&mut self) -> Result<()> {
        //TODO Question: Should we handle the scenario where there are more than one instance per operator running?
        // This scenario raises concerns that the protocol should be aware of a transaction that belongs to it but was not sent by itself (was seen in the blockchain)

        // The monitor is considered ready when it has fully indexed the blockchain and is up to date with the latest block.
        // Note that if there is a significant gap in the indexing process, it may take multiple ticks for the monitor to become ready.

        if !self.monitor.is_ready()? {
            info!("Monitor is not ready yet, continuing to index blockchain.");

            self.monitor.tick().context("Error detecting instances")?;

            return Ok(());
        }

        //TODO QUESTION?: I think we could not recieve a tx to be send for an instance that
        //  has a pending tx be dispatch. Otherwise we could add some warning..

        // Send pending transactions that were queued.
        self.process_pending_txs()
            .context("Error sending pending transactions")?;

        let last_block_height: u32 = self.current_height;
        self.current_height = self.monitor.get_current_height();

        // If the last block height is the same as the current one, there's no need to continue.
        if last_block_height == self.current_height {
            return Ok(());
        }

        // Handle any updates related to instances, including new information about transactions that have not been reviewed yet.
        self.process_instance_news()
            .context("Failed to process instance updates")?;

        // Handle any updates related to instances, including new information about transactions that have not been reviewed yet.
        self.process_in_progress_txs()
            .context("Failed to process instance updates")?;

        Ok(())
    }

    fn monitor_instance(&self, instance: &BitvmxInstance<TransactionPartialInfo>) -> Result<()> {
        if instance.txs.is_empty() {
            return Err(anyhow::anyhow!("Instance txs array is empty"));
        }

        //TODO: we could add some validation to check instance and txs existence in the storage

        self.store.add_instance(instance)?;

        let instance_new = InstanceData {
            instance_id: instance.instance_id,
            txs: instance.txs.iter().map(|tx| tx.tx_id).collect(),
        };

        // When an instance is saved in the monitor for tracking,
        // the current height of the indexer is used as the starting point for tracking.
        // This is not currently configurable.
        // It may change if we found a case where it should be configurable.

        self.monitor
            .save_instances_for_tracking(vec![instance_new])?;

        Ok(())
    }

    fn is_ready(&mut self) -> Result<bool> {
        //TODO: The orchestrator is currently considered ready when the monitor is ready.
        // However, we may decide to take into consideration pending and in progress transactions in the future.
        self.monitor.is_ready()
    }

    fn send_tx_instance(&self, instance_id: InstanceId, tx: &Transaction) -> Result<()> {
        // This section of code is responsible for adding a transaction to an instance and marking it as pending.
        // First, it adds the transaction to the instance using `add_tx_to_instance`. This method updates the instance
        // to include the new transaction, ensuring it is associated with the correct instance.

        self.store.add_tx_to_instance(instance_id, tx)?;

        // Next, it marks the transaction as pending using `add_pending_instance_tx`. This method updates the storage
        // to indicate that the transaction is currently pending and needs to be processed.
        self.store.update_instance_tx_status(
            instance_id,
            &tx.compute_txid(),
            TransactionStatus::ReadyToSend,
        )?;

        info!(
            "{} Transaction ID {} for Instance ID {} move to Pending status to be send.",
            style("Orchestrator").green(),
            style(tx.compute_txid()).yellow(),
            style(instance_id).green()
        );
        Ok(())
    }

    fn add_tx_to_instance(&self, _instance_id: InstanceId, _tx: &Txid) -> Result<()> {
        // Add a non-existent transaction to an existing instance.
        // The instance should exist in the storage.
        // The transaction id should not exist in the storage.
        // Usage: This method will likely be used for the final transaction to withdraw the funds.
        Ok(())
    }

    fn add_funding_tx(&self, instance_id: InstanceId, funding_tx: &FundingTx) -> Result<()> {
        self.store.add_funding_tx(instance_id, funding_tx)
    }

    fn notify_insufficient_funds(&self, _instance_id: InstanceId) -> Result<()> {
        todo!()
    }
}

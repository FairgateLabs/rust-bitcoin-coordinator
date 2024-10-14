use crate::{
    storage::{
        BitvmxStore, CompletedApi, FundingApi, InProgressApi, InstanceApi, PendingApi, SpeedUpApi,
    },
    types::{BitvmxInstance, DeliverData, FundingTx, InstanceId, SpeedUpData, TxInstance},
};
use anyhow::{Context, Ok, Result};
use bitcoin::{Amount, Network, Transaction, TxOut, Txid};
use bitcoincore_rpc::{Auth, Client};
use bitvmx_transaction_monitor::{
    monitor::{Monitor, MonitorApi},
    types::{BlockHeight, InstanceData, TxStatus},
};
use tracing::trace;
use transaction_dispatcher::{dispatcher::TransactionDispatcher, signer::Signer};

pub struct Orchestrator {
    monitor: Box<dyn MonitorApi>,
    dispatcher: TransactionDispatcher,
    store: BitvmxStore,
    current_height: BlockHeight,
}

pub trait OrchestratorApi {
    //TODO: this should be move to another place.
    const CONFIRMATIONS_THRESHOLD: u32 = 6;
    const OPERATOR_ID: u32 = 1;

    fn new(
        node_rpc_url: &str,
        db_file_path: &str,
        checkpoint_height: Option<BlockHeight>,
        username: &str,
        password: &str,
        network: Network,
    ) -> Result<Self>
    where
        Self: Sized;

    fn monitor_new_instance(&self, instance: &BitvmxInstance) -> Result<()>;

    // Add a non-existent transaction for an existing instance.
    fn add_tx_to_instance(&self, instance_id: InstanceId, tx: &Transaction) -> Result<()>;

    // The way that the protocol ask to deliver a existing tx for a instance id.
    fn send_tx_instance(&self, instance_id: InstanceId, tx: &Transaction) -> Result<()>;

    fn is_ready(&mut self) -> Result<bool>;

    fn tick(&mut self) -> Result<()>;
}

impl Orchestrator {
    fn send_pending_txs(&mut self) -> Result<()> {
        // Get pending instance transactions to be send to the blockchain
        let pending_txs = self.store.get_pending_instance_txs()?;

        // For each pending pair
        for (instance_id, tx) in pending_txs {
            //TODO:  Send should retrieve the fee rate used to be saved.
            let _ = self
                .dispatcher
                .send(tx.clone())
                .context("Error dispatching transaction")?;

            //TODO: This should be get from the send.
            let fee_rate = Amount::default();

            // TODO: check atomics transactions. to perform add and remove.

            self.store.add_in_progress_instance_tx(
                instance_id,
                &tx.compute_txid(),
                fee_rate,
                self.current_height,
            )?;

            // Instance tx is not more pending, it belongs into progress queue
            self.store
                .remove_pending_instance_tx(instance_id, &tx.compute_txid())?;
        }

        Ok(())
    }

    fn speed_up(
        &mut self,
        instance_id: InstanceId,
        tx: &Transaction,
        funding_txid: Txid,
        funding_utxo: (u32, TxOut),
    ) -> Result<()> {
        let (speed_up_tx_id, amount) =
            self.dispatcher
                .speed_up(tx, funding_txid, funding_utxo.clone())?;

        let speed_up_tx = TxInstance {
            tx: None, // We don't have speed up transaction yet.
            tx_id: speed_up_tx_id,
            owner_operator_id: instance_id,
            deliver_data: Some(DeliverData {
                fee_rate: amount,
                block_height: self.current_height,
            }),
            speed_up_data: Some(SpeedUpData {
                is_speed_up_tx: true,
                child_tx_id: tx.compute_txid(),
                utxo_index: funding_utxo.0,
                utxo_output: funding_utxo.1,
            }),
        };

        self.store.add_instance_tx(instance_id, &speed_up_tx)?;

        self.store
            .add_speed_up_tx(instance_id, &speed_up_tx_id, speed_up_tx_id)?;

        //TODO: should we save owner true otherwise it can be confuse with txs from the protocol are not owers  ?
        self.monitor
            .save_transaction_for_tracking(instance_id, speed_up_tx_id)?;

        Ok(())
    }

    fn notify_protocol_tx_changes(
        &self,
        instance: InstanceId,
        tx: &Txid,
        tx_hex: &str,
    ) -> Result<()> {
        // Implement the notification logic here
        println!(
            "Notification sent to protocol for instance_id: {:?}  tx_id: {} tx_hex {}",
            instance, tx, tx_hex
        );
        Ok(())
    }

    fn process_instance_changes(&mut self) -> Result<()> {
        // Get any news in each instance that are being monitored.
        // TODO: Monitor need to implement reorganisations in news. at this moment Monitor
        // is not updating every instance update after a reorg.
        // Get instances news also returns the speed ups txs added for each instance.
        let news = self.monitor.get_instance_news()?;

        for (instance_id, tx_ids) in news {
            for tx_id in tx_ids {
                // Process each transaction, handling errors early to avoid nesting
                self.process_instance_tx_change(instance_id, tx_id)?;
            }
        }

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

        if tx_status.tx_was_seen {
            if tx_status.confirmations == 1 {
                //Confirmation in 1 means is it already included in the block.
                //Transaction new could be transaction speed up. We need to check if confirmation
                self.handle_confirmation_transaction(instance_id, &tx_status)?;

                return Ok(());
            }

            if tx_status.confirmations > Self::CONFIRMATIONS_THRESHOLD {
                // Transaction was mined and has sufficient confirmations for
                // move the transaction to complete.
                self.handle_complete_transaction(instance_id, &tx_status)?;
                return Ok(());
            }

            return Ok(());
        }

        //TODO: I Think this never gonna happend. Because instance txs news are just the once that have some change.
        if !tx_status.tx_was_seen {
            // Get information for the last time the transaction was sent
            let in_progress_tx = self.store.get_in_progress_txs(instance_id, &tx_id)?;

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
        tx_status: &TxStatus,
    ) -> Result<()> {
        let tx_info = self
            .store
            .get_instance_tx(instance_id, &tx_status.tx_id)?
            .ok_or(anyhow::anyhow!(
                "No transaction info found for transaction ID: {} and instance ID: {}",
                tx_status.tx_id,
                instance_id
            ))?;

        if tx_info.owner_operator_id == Self::OPERATOR_ID {
            // This transaction belongs to this instance and could be either a speed-up transaction or an instance transaction related to the protocol.

            if tx_info.speed_up_data.is_some() {
                let speed_ud_data = tx_info.speed_up_data.unwrap();
                // This indicates that this is a speed-up transaction that has been mined with 1 confirmation, which means it should be treated as the new funding transaction.
                // TODO: In the event of a reorganization, we would need to do the opposite. This involves removing the speed-up transaction and potentially replacing it with another transaction that could take its place as the last speed-up transaction or become the new last funding transaction.

                let funding_info = FundingTx {
                    tx_id: tx_info.tx_id,
                    utxo_index: speed_ud_data.utxo_index,
                    utxo_output: speed_ud_data.utxo_output,
                };

                self.store.add_funding_tx(instance_id, &funding_info)?;
            }
        }
        Ok(())
    }

    fn handle_complete_transaction(
        &mut self,
        instance_id: InstanceId,
        tx_status: &TxStatus,
    ) -> Result<()> {
        // Notify the protocol about the transaction changes, specifically for confirmed transactions.
        // This step is crucial for the protocol to be aware of the transaction's status and proceed accordingly.
        self.notify_protocol_tx_changes(
            instance_id,
            &tx_status.tx_id,
            &tx_status.tx_hex.clone().unwrap(),
        )?;

        // Move the transaction to completed given that transaction has more than the threshold confirmations
        // This step include the transaction as completed in the store, indicating that it has been successfully processed.
        self.store
            .add_completed_instance_tx(instance_id, &tx_status.tx_id)?;

        // Remove the transaction from the in-progress list to ensure it's not processed again.
        // This step is important for maintaining the integrity of the transaction processing pipeline.
        self.store
            .remove_in_progress_instance_tx(instance_id, &tx_status.tx_id)?;

        // Acknowledge the transaction news to the monitor to update its state.
        // This step ensures that the monitor is aware of the transaction's completion and can update its tracking accordingly.
        self.monitor
            .acknowledge_instance_tx_news(instance_id, &tx_status.tx_id)?;

        Ok(())
    }

    fn handle_unseen_transaction(
        &mut self,
        instance_id: InstanceId,
        tx_data: &TxInstance,
    ) -> Result<()> {
        if tx_data.speed_up_data.is_some() {
            // If it is a speed up transaction then we don't need to do anything, it is not mined.
            // Then when we check if the transaction was speed up should be decide speed up again or wait.
            return Ok(());
        }

        // We should not have speed up transactions for 2 different transaction.
        // We need to get the information about the speed ups transaction for this transactition
        let speed_up_transactions = self.store.get_speed_up_txs(instance_id, tx_data.tx_id)?;

        let old_fee_rate = if speed_up_transactions.is_empty() {
            tx_data.deliver_data.as_ref().unwrap().fee_rate
        } else {
            speed_up_transactions
                .last()
                .unwrap()
                .deliver_data
                .as_ref()
                .unwrap()
                .fee_rate
        };

        // Check if the transaction should be speed up
        let should_speed_up = self.dispatcher.should_speed_up(old_fee_rate)?;

        if !should_speed_up {
            return Ok(());
        }

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
            (funding_tx.utxo_index, funding_tx.utxo_output),
        )?;

        Ok(())
    }
}

impl OrchestratorApi for Orchestrator {
    fn new(
        node_rpc_url: &str,
        db_file_path: &str,
        checkpoint_height: Option<BlockHeight>,
        username: &str,
        password: &str,
        network: Network,
    ) -> Result<Self> {
        trace!("Creating a new instance of Orchestrator");
        let store = BitvmxStore::new_with_path(db_file_path)?;
        let monitor = Monitor::new_with_paths(node_rpc_url, db_file_path, checkpoint_height)?;
        let auth = Auth::UserPass(username.to_string(), password.to_string());
        let client = Client::new(node_rpc_url, auth)?;
        let signer = Signer::new(None);
        let dispatcher = TransactionDispatcher::new(client, signer, network);
        trace!("TransactionDispatcher instance created successfully");

        Ok(Self {
            monitor: Box::new(monitor),
            dispatcher,
            store,
            current_height: 0,
        })
    }

    fn tick(&mut self) -> Result<()> {
        //TODO Question: Should we handle the scenario where there are more than one instance per operator running?
        // This scenario raises concerns that the protocol should be aware of a transaction that belongs to it but was not sent by itself (was seen in the blockchain)

        // The monitor is considered ready when it has fully indexed the blockchain and is up to date with the latest block.
        // Note that if there is a significant gap in the indexing process, it may take multiple ticks for the monitor to become ready.
        if !self.monitor.is_ready()? {
            self.monitor
                .detect_instance_changes()
                .context("Error detecting instances")?;

            return Ok(());
        }

        //TODO QUESTION?: I think we could not recieve a tx to be send for an instance that
        //  has a pending tx be dispatch. Otherwise we could add some warning..

        // Send pending transactions that were queued.
        self.send_pending_txs()
            .context("Error sending pending transactions")?;

        let last_block_height: u32 = self.current_height;
        self.current_height = self.monitor.get_current_height();

        // If the last block height is the same as the current one, there's no need to continue.
        if last_block_height == self.current_height {
            return Ok(());
        }

        // - Transactions that are stalled and should be dispatched again. And are not news

        // Handle any updates related to instances, including new information about transactions that have not been reviewed yet.
        self.process_instance_changes()
            .context("Failed to process instance updates")?;

        Ok(())
    }

    fn monitor_new_instance(&self, instance: &BitvmxInstance) -> Result<()> {
        self.store.add_instance(instance)?;

        let instance_new = InstanceData {
            id: instance.instance_id,
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

    //TODO: Send a transaction for an instance. Protocol should
    fn send_tx_instance(&self, instance_id: InstanceId, tx: &Transaction) -> Result<()> {
        let exists = self.store.tx_exists(instance_id, &tx.compute_txid())?;

        if exists {
            self.store
                .add_pending_instance_tx(instance_id, &tx.clone())?;
        } else {
            return Err(anyhow::anyhow!(
                "Transaction with ID {} for instance {} does not exist",
                tx.compute_txid(),
                instance_id
            ));
        }

        Ok(())
    }

    fn add_tx_to_instance(&self, _instance_id: InstanceId, _tx: &Transaction) -> Result<()> {
        // Add a non-existent transaction to an existing instance.

        // The instance should exist in the storage.
        // The transaction id should not exist in the storage.

        // Usage: This method will likely be used for the final transaction to withdraw the funds.
        Ok(())
    }
}

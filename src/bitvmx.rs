use crate::{
    storage::{BitvmxStore, CompletedApi, FundingApi, InProgressApi, InstanceApi, PendingApi},
    types::{BitvmxInstance, FundingTx, InProgressTx, InstanceId},
};
use anyhow::{bail, Context, Ok, Result};
use bitcoin::{
    absolute::LockTime, transaction::Version, Amount, Network, ScriptBuf, Transaction, TxOut, Txid,
};
use bitcoincore_rpc::{Auth, Client};
use bitvmx_transaction_monitor::{
    monitor::{Monitor, MonitorApi},
    types::{BlockHeight, TxStatus},
};
use transaction_dispatcher::{dispatcher::TransactionDispatcher, signer::Signer};

pub struct BitVMXOrchestrator {
    monitor: Box<dyn MonitorApi>,
    dispatcher: TransactionDispatcher,
    store: BitvmxStore,
    current_height: BlockHeight,
}

pub trait BitVMXOrchestratorApi {
    const CONFIRMATIONS_THRESHOLD: u32 = 6;

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

    fn monitor_new_instance(&self, instance: BitvmxInstance) -> Result<()>;

    fn tick(&mut self) -> Result<()>;
}

impl BitVMXOrchestrator {
    fn send_pending_txs(&mut self) -> Result<()> {
        // Get pending instance transactions to be send to the blockchain
        let pending_txs = self.store.get_pending_list()?;

        // For each pending pair
        for (instance_id, tx) in pending_txs {
            //TODO: send should return the fee_remove_pending_instance_txrate was send the transaction.
            //Dispatch transaction.
            let _ = self.dispatcher.send(tx.clone())?;

            //TODO: This should be get from the send.
            let fee_rate = Amount::default();

            // Store instance and tx already send to the blockchain to be audited and check if was mined in the next tick.
            self.store.add_in_progress_instance_tx(
                instance_id,
                &tx,
                fee_rate,
                self.current_height,
            )?;

            // Instance tx is not more pending, it belongs into progress queue
            self.store
                .remove_pending_instance_tx(instance_id, &tx.compute_txid())?;
        }

        Ok(())
    }

    //TODO: This should be done inside the dispatcher
    fn speed_up(
        &mut self,
        tx: &Transaction,
        funding_txid: Txid,
        funding_utxo: (u32, TxOut),
    ) -> Result<(Txid, Amount, FundingTx)> {
        let (tx_id, amount) = self.dispatcher.speed_up(tx, funding_txid, funding_utxo)?;

        //Todo this is mock
        let funding_tx = Transaction {
            version: Version::TWO,     // Post BIP-68.
            lock_time: LockTime::ZERO, // Ignore the locktime.
            input: vec![],
            output: vec![],
        };

        let new_funding_tx = FundingTx {
            tx_id: funding_tx.compute_txid(),
            utxo_index: 1,
            utxo_output: TxOut {
                value: Amount::default(),
                script_pubkey: ScriptBuf::default(),
            },
        };

        Ok((tx_id, amount, new_funding_tx))
    }

    fn notify_protocol_tx_changes(&self, instance: InstanceId, tx: &Txid) -> Result<()> {
        // Implement the notification logic here
        println!(
            "Notifying protocol about changes in instance {:?}, tx: {}",
            instance, tx
        );
        Ok(())
    }

    fn resolve_in_progress_instances_txs(&mut self) -> Result<()> {
        // Get any news in each instance that are being monitored
        let news = self.monitor.get_instance_news()?;

        for (instance_id, tx_ids) in news {
            for tx_id in tx_ids {
                // Process each transaction, handling errors early to avoid nesting
                self.process_instance_tx(instance_id, tx_id)?;
            }
        }

        Ok(())
    }

    fn process_instance_tx(&mut self, instance_id: InstanceId, tx_id: Txid) -> Result<()> {
        // Get information for the last time the transaction was sent
        let in_progress_tx = match self
            .store
            .get_in_progress_instance_tx(instance_id, &tx_id)?
        {
            Some(pending_tx) => pending_tx,
            None => {
                // Notify the protocol if no pending transaction is found
                self.notify_protocol_tx_changes(instance_id, &tx_id)?;
                return Ok(());
            }
        };

        // Get the transaction status from the monitor
        let tx_status = match self.monitor.get_instance_tx_status(instance_id, tx_id)? {
            Some(tx_status) => tx_status,
            None => bail!(
                "No status for tx_id: {} , instance_id: {}",
                tx_id,
                instance_id
            ),
        };

        // Handle the transaction based on its status
        self.handle_tx_status(instance_id, &in_progress_tx, &tx_status)?;

        Ok(())
    }

    fn handle_tx_status(
        &mut self,
        instance_id: InstanceId,
        in_progress_tx: &InProgressTx,
        tx_status: &TxStatus,
    ) -> Result<()> {
        if tx_status.tx_was_seen && tx_status.confirmations > Self::CONFIRMATIONS_THRESHOLD {
            // Transaction was mined and has sufficient confirmations
            self.complete_transaction(instance_id, tx_status)?;
        } else if !tx_status.tx_was_seen {
            // Transaction was not seen, consider speeding up
            self.handle_unseen_transaction(instance_id, in_progress_tx)?;
        }

        Ok(())
    }

    fn complete_transaction(
        &mut self,
        instance_id: InstanceId,
        tx_status: &TxStatus,
    ) -> Result<()> {
        self.store
            .add_completed_instance_tx(instance_id, &tx_status.tx_id)?;
        self.store
            .remove_in_progress_instance_tx(instance_id, &tx_status.tx_id)?;
        self.monitor
            .acknowledge_instance_tx_news(instance_id, tx_status.tx_id)?;
        Ok(())
    }

    fn handle_unseen_transaction(
        &mut self,
        instance_id: InstanceId,
        in_progress_tx: &InProgressTx,
    ) -> Result<()> {
        // Check if the transaction should be sped up
        let should_speed_up = self.dispatcher.should_speed_up(in_progress_tx.fee_rate)?;

        if should_speed_up {
            let funding_tx = self.store.get_funding_tx()?;

            let funding_tx = match funding_tx {
                Some(funding_tx) => funding_tx,
                None => panic!("No funding transaction available for speed up"),
            };

            // Speed up the transaction
            let (_tx_id, fee_rate, new_funding) = self.speed_up(
                &in_progress_tx.tx,
                funding_tx.tx_id,
                (funding_tx.utxo_index, funding_tx.utxo_output),
            )?;

            // Update the store with new transaction details
            self.store.update_in_progress_instance_tx(
                instance_id,
                &in_progress_tx.tx.compute_txid(),
                fee_rate,
                self.current_height,
            )?;

            // Create FundingTx struct

            // Add the new funding transaction to the store
            self.store.add_funding_tx(&new_funding)?;
        }

        Ok(())
    }
}

impl BitVMXOrchestratorApi for BitVMXOrchestrator {
    fn new(
        node_rpc_url: &str,
        db_file_path: &str,
        checkpoint_height: Option<BlockHeight>,
        username: &str,
        password: &str,
        network: Network,
    ) -> Result<Self> {
        let store = BitvmxStore::new_with_path(db_file_path)?;
        let monitor = Monitor::new_with_paths(node_rpc_url, db_file_path, checkpoint_height)?;
        let auth = Auth::UserPass(username.to_string(), password.to_string());
        let client = Client::new(node_rpc_url, auth)?;
        let signer = Signer::new(None);
        let dispatcher = TransactionDispatcher::new(client, signer, network);

        Ok(Self {
            monitor: Box::new(monitor),
            dispatcher,
            store,
            current_height: 0,
        })
    }

    fn tick(&mut self) -> Result<()> {
        // Monitor detects new blocks and transactions in the blockchain and
        // saves any changes related to BitVMX instances.
        self.monitor
            .detect_instances()
            .context("Error detecting instances")?;

        // Send pending transactions that were queued.
        self.send_pending_txs()
            .context("Error sending pending transactions")?;

        let last_block_height: u32 = self.current_height;
        self.current_height = self.monitor.get_current_height();

        // If the last block height is the same as the current one, there's no need to continue.
        if last_block_height == self.current_height {
            return Ok(());
        }

        // Resolve in-progress transactions:
        // - Transactions that have been mined
        // - Transactions that are stalled and should be dispatched again
        self.resolve_in_progress_instances_txs()
            .context("Error resolving in-progress transactions")?;

        Ok(())
    }

    fn monitor_new_instance(&self, instance: BitvmxInstance) -> Result<()> {
        self.store.add_instance(&instance)?;
        Ok(())
    }
}

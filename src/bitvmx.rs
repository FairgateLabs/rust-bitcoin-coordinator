use crate::{
    storage::{BitvmxApi, BitvmxStore},
    types::InstanceId,
};
use anyhow::{bail, Context, Result};
use bitcoin::{
    absolute::LockTime, transaction::Version, Amount, Network, Transaction, TxOut, Txid,
};
use bitcoincore_rpc::{Auth, Client};
use bitvmx_transaction_monitor::{
    monitor::{Monitor, MonitorApi},
    types::{BitvmxInstance, BlockHeight},
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

    fn push_bitvmx_instance(&self, instances: Vec<BitvmxInstance>);

    fn tick(&mut self) -> Result<()>;
}

impl BitVMXOrchestrator {
    fn send_pending_instance_txs(&mut self) -> Result<()> {
        let pending_txs = self.store.get_pending_instance_txs()?;

        for (_, tx) in pending_txs {
            //TODO: send should return the fee_rate was send the transaction.
            let _ = self.dispatcher.send(tx.clone())?;

            //TODO: I think we should add the instance id of the transaction that is in progress.
            self.store
                .add_in_progress_tx(&tx, Amount::default(), self.current_height)?;
        }

        Ok(())
    }

    //TODO: This should be done inside the dispatcher
    fn speed_up(
        &mut self,
        tx: &Transaction,
        funding_txid: Txid,
        funding_utxo: (u32, TxOut),
    ) -> Result<(Txid, Amount, Transaction)> {
        let (tx_id, amount) = self.dispatcher.speed_up(tx, funding_txid, funding_utxo)?;

        let funding_tx = Transaction {
            version: Version::TWO,     // Post BIP-68.
            lock_time: LockTime::ZERO, // Ignore the locktime.
            input: vec![],
            output: vec![],
        };

        Ok((tx_id, amount, funding_tx))
    }

    fn notify_protocol_tx_changes(&self, instance: InstanceId, tx: &Txid) -> Result<()> {
        // Implement the notification logic here
        println!(
            "Notifying protocol about changes in instance {:?}, tx: {}",
            instance, tx
        );
        Ok(())
    }

    fn resolve_in_progress_instance_txs(&mut self) -> Result<()> {
        let tx_news_in_instances = self.monitor.get_instance_news()?;

        for (instance_id, tx_ids) in tx_news_in_instances {
            for tx_id in tx_ids {
                let pending_tx = self.store.get_in_progress_tx(&tx_id)?;

                match pending_tx {
                    Some(pending_tx) => {
                        let tx_status = self.monitor.get_instance_tx_status(instance_id, tx_id)?;

                        match tx_status {
                            Some(tx_status) => {
                                if tx_status.tx_was_seen
                                    && tx_status.confirmations > Self::CONFIRMATIONS_THRESHOLD
                                {
                                    //Transaction was mined and there are sufficients confirmations to say is completed.
                                    self.store.add_completed_instance_tx(instance_id, &tx_id);
                                    self.store
                                        .remove_in_progress_instance_tx(instance_id, &tx_id)?;
                                    self.monitor.acknowledge_instance_tx_news(
                                        instance_id,
                                        tx_status.tx_id,
                                    )?;

                                    continue;
                                }

                                if tx_status.tx_was_seen {
                                    let should_speed_up =
                                        self.dispatcher.should_speed_up(pending_tx.fee_rate)?;

                                    if should_speed_up {
                                        let funding_tx = self.store.get_funding_tx()?;

                                        match funding_tx {
                                            Some(funding_tx) => {
                                                let (_tx_id, fee_rate, new_funding) = self
                                                    .speed_up(
                                                        &pending_tx.tx,
                                                        funding_tx.tx_id,
                                                        (
                                                            funding_tx.utxo_index,
                                                            funding_tx.utxo_output,
                                                        ),
                                                    )?;

                                                self.store
                                                    .mark_funding_tx_as_used(&funding_tx.tx_id);
                                                self.store.update_in_progress_tx(
                                                    &pending_tx.tx.compute_txid(),
                                                    fee_rate,
                                                    self.current_height,
                                                );

                                                //TODO: here we have a problem, what happend with this new speed up transactions if prev tx is not mined.
                                                self.store.add_funding_tx(&new_funding);
                                            }
                                            None => {
                                                //TODO: Should not reach here.
                                                // What should we do ?
                                                panic!(
                                                    "No funding transaction available for speed up"
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            None => {
                                // Monitor shoud not return None for a transaction that is in stalled_txs
                                // We should never get here
                                bail!(
                                    "There is no status for a tx_id: {} , instance_id: {}",
                                    tx_id,
                                    instance_id
                                );
                            }
                        }
                    }
                    None => {
                        //TODO:
                        // There is some news.
                        // Protocol should decide what to do with this instance tx news

                        // Notify the protocol about the changes in the transaction
                        self.notify_protocol_tx_changes(instance_id, &tx_id)?;
                    }
                }
            }
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
        // Detect new change in the blockchain about bitvmx instances and each transaction.
        self.monitor
            .detect_instances()
            .context("There was an error detecting instances")?;

        // Send new transacctions that are pending to send.
        self.send_pending_instance_txs()
            .context("There was an error sending pending txs")?;

        let last_block_height: u32 = self.current_height;
        self.current_height = self.monitor.get_current_height();

        // if last block_height is the same that the current one. It does not make sense to continue.
        if last_block_height == self.current_height {
            return Ok(());
        }

        // Resolve transaction that are in progress, transactions are not finished yet
        // Transaction that are already mined or are stalled and should be dispatch again for some reason.
        self.resolve_in_progress_instance_txs()
            .context("There was an error resolving progress txs")?;

        Ok(())
    }

    fn push_bitvmx_instance(&self, instances: Vec<BitvmxInstance>) {
        self.store.add_instances(&instances);
    }
}

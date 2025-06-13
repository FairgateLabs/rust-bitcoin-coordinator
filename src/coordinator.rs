use crate::{
    constants::{CPFP_TRANSACTION_CONTEXT, MAX_MONITORING_CONFIRMATIONS, MAX_TX_WEIGHT},
    errors::BitcoinCoordinatorError,
    speedup::SpeedupStore,
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::{
        AckNews, CoordinatedSpeedUpTransaction, CoordinatedTransaction, CoordinatorNews, News,
        SpeedupState, TransactionState,
    },
};
use bitcoin::{Address, CompressedPublicKey, Network, Transaction, Txid};
use bitvmx_bitcoin_rpc::{bitcoin_client::BitcoinClient, rpc_config::RpcConfig};
use bitvmx_bitcoin_rpc::{bitcoin_client::BitcoinClientApi, types::BlockHeight};
use bitvmx_transaction_monitor::{
    errors::MonitorError,
    monitor::{Monitor, MonitorApi},
    types::{AckMonitorNews, MonitorNews, MonitorType, TransactionStatus, TypesToMonitor},
};
use console::style;
use key_manager::key_manager::KeyManager;
use protocol_builder::{builder::ProtocolBuilder, types::Utxo};
use std::rc::Rc;
use storage_backend::storage::Storage;
use tracing::{error, info, warn};

pub struct BitcoinCoordinator {
    monitor: MonitorType,
    key_manager: Rc<KeyManager>,
    store: BitcoinCoordinatorStore,
    client: BitcoinClient,
    network: Network,
}

pub trait BitcoinCoordinatorApi {
    /// Checks if the coordinator is ready to process transactions
    /// Returns true if the coordinator is ready, false otherwise
    fn is_ready(&self) -> Result<bool, BitcoinCoordinatorError>;

    /// Processes pending transactions and updates their status
    /// This method should be called periodically to keep the coordinator state up-to-date
    fn tick(&self) -> Result<(), BitcoinCoordinatorError>;

    /// Registers a type of data to be monitored by the coordinator
    /// The data will be tracked for confirmations and status changes, and updates will be reported through the news.
    ///
    /// # Arguments
    /// * `data` - The data to monitors
    fn monitor(&self, data: TypesToMonitor) -> Result<(), BitcoinCoordinatorError>;

    /// Dispatches a transaction to the Bitcoin network
    ///
    /// # Arguments
    /// * `tx` - The Bitcoin transaction to dispatch
    /// * `speedup` - Speed up information for the transaction (None means it should not be speed up)
    /// * `context` - Additional context information for the transaction to be returned in news
    /// * `block_height` - Block height to dispatch the transaction (None means now)
    fn dispatch(
        &self,
        tx: Transaction,
        speedup: Option<Utxo>,
        context: String,
        block_height: Option<BlockHeight>,
    ) -> Result<(), BitcoinCoordinatorError>;

    /// Cancels the monitor and the dispatch of a type of data
    /// This method removes the monitor and the dispatch from the coordinator's store.
    /// Which means that the data will no longer be monitored.
    ///
    /// # Arguments
    /// * `data` - The data to cancel
    fn cancel(&self, data: TypesToMonitor) -> Result<(), BitcoinCoordinatorError>;

    /// Registers funding information for potential transaction speed-ups
    /// This allows the coordinator to create child pays for parents transactions when needed
    ///
    /// # Arguments
    /// * `utxo` - Utxo to use for speed-ups
    fn add_funding(&self, utxo: Utxo) -> Result<(), BitcoinCoordinatorError>;

    fn get_transaction(&self, txid: Txid) -> Result<TransactionStatus, BitcoinCoordinatorError>;

    /// Retrieves news about monitored transactions
    /// Returns information about transaction confirmations.
    fn get_news(&self) -> Result<News, BitcoinCoordinatorError>;

    /// Acknowledges that news has been processed
    /// This prevents the same news from being returned in subsequent calls to get_news()
    ///
    /// # Arguments
    /// * `news` - The news items to acknowledge
    fn ack_news(&self, news: AckNews) -> Result<(), BitcoinCoordinatorError>;
}

impl BitcoinCoordinator {
    pub fn new_with_paths(
        rpc_config: &RpcConfig,
        storage: Rc<Storage>,
        key_manager: Rc<KeyManager>,
        checkpoint: Option<BlockHeight>,
        confirmation_threshold: u32,
    ) -> Result<Self, BitcoinCoordinatorError> {
        let monitor = Monitor::new_with_paths(
            rpc_config,
            storage.clone(),
            checkpoint,
            confirmation_threshold,
        )?;

        let store = BitcoinCoordinatorStore::new(storage)?;
        let bitcoin_client = BitcoinClient::new_from_config(rpc_config)?;
        let network = rpc_config.network;
        let coordinator =
            BitcoinCoordinator::new(monitor, store, key_manager, bitcoin_client, network);

        Ok(coordinator)
    }

    pub fn new(
        monitor: MonitorType,
        store: BitcoinCoordinatorStore,
        key_manager: Rc<KeyManager>,
        client: BitcoinClient,
        network: Network,
    ) -> Self {
        Self {
            monitor,
            store,
            key_manager,
            client,
            network,
        }
    }

    fn process_pending_txs_to_dispatch(&self) -> Result<(), BitcoinCoordinatorError> {
        // Get pending transactions to be send to the blockchain
        let pending_txs = self.store.get_txs_to_dispatch()?;

        if pending_txs.is_empty() {
            return Ok(());
        }

        let txs_to_dispatch: Vec<CoordinatedTransaction> = pending_txs
            .iter()
            .filter(|tx| self.should_dispatch_tx(tx).unwrap_or(false))
            .cloned()
            .collect();

        let (txs_to_dispatch_with_speedup, txs_to_dispatch_without_speedup): (Vec<_>, Vec<_>) =
            txs_to_dispatch
                .into_iter()
                .partition(|tx| self.should_need_speedup(tx));

        info!(
            "{} Number of transactions to dispatch without speedup {}",
            style("Coordinator").green(),
            style(txs_to_dispatch_without_speedup.len()).yellow()
        );

        info!(
            "{} Number of transactions to dispatch with speedup {}",
            style("Coordinator").green(),
            style(txs_to_dispatch_with_speedup.len()).yellow()
        );

        self.dispatch_txs(txs_to_dispatch_without_speedup)?;

        // Check if we can send transactions or we stop the process until CPFP transactions start to be confirmed.
        if self.store.can_speedup()? {
            // TODO: Transaction that don't need speedup should be dispatched
            self.speedup_and_dispatch_in_batch(txs_to_dispatch_with_speedup)?;
        } else {
            info!("{} Can not speedup", style("Coordinator").green());
        }

        Ok(())
    }

    fn speedup_and_dispatch_in_batch(
        &self,
        txs: Vec<CoordinatedTransaction>,
    ) -> Result<(), BitcoinCoordinatorError> {
        // Attempt to dispatch as many transactions as possible in a single CPFP (Child Pays For Parent) transaction,
        // while ensuring the resulting transaction does not exceed Bitcoin's standardness limits.
        // Maximum transaction size: 400,000 weight units.
        // Exceeding these limits will result in the transaction being considered non-standard and rejected by most mempools.
        // If the set of transactions exceeds these limits, they must be split into multiple CPFP transactions.

        // TODO: This should be change for adding the child pays for the parents tx in order to be send to the network.

        let txs_in_batch_by_size: Vec<Vec<CoordinatedTransaction>> =
            self.batch_txs_by_weight_limit(txs)?;

        for txs_batch in txs_in_batch_by_size {
            // For each batch, attempt to broadcast all transactions individually. After determining which transactions were successfully sent,
            // construct and broadcast a single CPFP (Child Pays For Parent) transaction to pay for the entire batch.
            let txs_sent: Vec<CoordinatedTransaction> = self.dispatch_txs(txs_batch)?;

            let bump_percent = self.get_bump_strategy(1.0)?;
            // TODO: For now we assume this will always work.
            self.create_and_send_cpfp_tx(txs_sent, bump_percent)?;
        }

        Ok(())
    }

    fn dispatch_txs(
        &self,
        txs: Vec<CoordinatedTransaction>,
    ) -> Result<Vec<CoordinatedTransaction>, BitcoinCoordinatorError> {
        let mut txs_sent = Vec::new();

        for tx in txs {
            info!(
                "{} Send Transaction({})",
                style("Coordinator").green(),
                style(tx.tx_id).yellow(),
            );

            let dispatch_result = self.client.send_transaction(&tx.tx);

            match dispatch_result {
                Ok(_) => {
                    let deliver_block_height = self.monitor.get_monitor_height()?;

                    self.store
                        .update_tx_to_dispatched(tx.tx_id, deliver_block_height)?;

                    txs_sent.push(tx);
                }
                Err(e) => {
                    error!(
                        "{} Error Sending Transaction({})",
                        style("Coordinator").green(),
                        style(tx.tx_id).blue()
                    );

                    let news = CoordinatorNews::DispatchTransactionError(
                        tx.tx_id,
                        tx.context.clone(),
                        e.to_string(),
                    );

                    self.store.add_news(news)?;

                    self.store
                        .update_tx_state(tx.tx_id, TransactionState::Failed)?;
                }
            }
        }

        Ok(txs_sent)
    }

    fn batch_txs_by_weight_limit(
        &self,
        txs: Vec<CoordinatedTransaction>,
    ) -> Result<Vec<Vec<CoordinatedTransaction>>, BitcoinCoordinatorError> {
        // Define the maximum total weight allowed per batch of transactions.

        let mut batches = Vec::new();
        let mut current_batch = Vec::new();
        let mut current_weight = 0;

        for tx_data in txs {
            let weight = tx_data.tx.weight().to_wu();

            if weight > MAX_TX_WEIGHT {
                return Err(BitcoinCoordinatorError::TransactionTooHeavy(
                    tx_data.tx_id.to_string(),
                    weight,
                    MAX_TX_WEIGHT,
                ));
            }

            if current_weight + weight > MAX_TX_WEIGHT {
                batches.push(current_batch);
                current_batch = Vec::new();
                current_weight = 0;
            }
            current_batch.push(tx_data);
            current_weight += weight;
        }

        if !current_batch.is_empty() {
            batches.push(current_batch);
        }

        Ok(batches)
    }

    fn process_in_progress_speedup_txs(&self) -> Result<(), BitcoinCoordinatorError> {
        let txs = self.store.get_pending_speedups()?;

        for tx in txs {
            info!(
                "{} Processing Speedup Transaction({})",
                style("Coordinator").green(),
                style(tx.tx_id).blue(),
            );

            // Get updated transaction status from monitor
            let tx_status = self.monitor.get_tx_status(&tx.tx_id);

            match tx_status {
                Ok(tx_status) => {
                    // Handle the case where the transaction is a CPFP (Child Pays For Parent) transaction.

                    // First we acknowledge the transaction to clear any related news.
                    let ack = AckMonitorNews::Transaction(tx_status.tx_id);
                    self.monitor.ack_news(ack)?;

                    if tx_status.confirmations >= MAX_MONITORING_CONFIRMATIONS {
                        // Once the transaction is finalized, we are not monitoring it anymore.
                        self.store
                            .update_tx_state(tx_status.tx_id, TransactionState::Finalized)?;

                        continue;
                    }

                    if tx_status.is_confirmed() {
                        // We want to keep the the confirmation on the storage to  calculate the maximum speedups
                        self.store
                            .update_tx_state(tx_status.tx_id, TransactionState::Confirmed)?;

                        continue;
                    }

                    if tx_status.is_orphan() {
                        // Move the
                        self.store
                            .update_tx_state(tx_status.tx_id, TransactionState::Dispatched)?;
                    }
                }
                Err(MonitorError::TransactionNotFound(_)) => {}
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }

    fn process_in_progress_txs(&self) -> Result<(), BitcoinCoordinatorError> {
        let txs = self.store.get_txs_in_progress()?;

        for tx in txs {
            info!(
                "{} Processing Transaction({})",
                style("Coordinator").green(),
                style(tx.tx_id).blue(),
            );

            // Get updated transaction status from monitor
            let tx_status = self.monitor.get_tx_status(&tx.tx_id);

            match tx_status {
                Ok(tx_status) => {
                    if tx_status.confirmations >= MAX_MONITORING_CONFIRMATIONS {
                        // Once the transaction is finalized, we are not monitoring it anymore.
                        self.store
                            .update_tx_state(tx_status.tx_id, TransactionState::Finalized)?;

                        continue;
                    }

                    if tx_status.is_confirmed() {
                        self.store
                            .update_tx_state(tx_status.tx_id, TransactionState::Confirmed)?;
                    }
                }
                Err(MonitorError::TransactionNotFound(_)) => {
                    // In case a transaction is not found, we just wait.
                    // We are going to speed up the CPFP.
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }

    fn should_need_speedup(&self, tx: &CoordinatedTransaction) -> bool {
        tx.cpfp_utxo.is_some()
    }

    fn should_dispatch_tx(
        &self,
        pending_tx: &CoordinatedTransaction,
    ) -> Result<bool, BitcoinCoordinatorError> {
        let should_be_dispatched_now = pending_tx.target_block_height.is_none();

        if should_be_dispatched_now {
            return Ok(true);
        }

        let was_already_broadcasted = pending_tx.broadcast_block_height.is_some();

        if was_already_broadcasted {
            warn!(
                "Transaction({}) already broadcasted. This could be a border case or a bug.",
                pending_tx.tx_id
            );

            // THIS COULD BE A BORDER CASE OR A BUG.
            // This code path should not be reached because once a transaction is broadcast,
            // it should be marked as BroadcastPendingConfirmation.
            return Ok(false);
        }

        let current_block_height = self.monitor.get_monitor_height()?;

        Ok(current_block_height >= pending_tx.target_block_height.unwrap())
    }

    fn create_and_send_cpfp_tx(
        &self,
        txs_data: Vec<CoordinatedTransaction>,
        bump_percent: f64,
    ) -> Result<(), BitcoinCoordinatorError> {
        // This function creates a CPFP (Child Pays For Parent) to fund transactions and sends it to the network.

        let funding = self.store.get_funding()?;

        if funding.is_none() {
            let news = CoordinatorNews::FundingNotFound();
            self.store.add_news(news)?;
            return Ok(());
        }

        let funding = funding.unwrap();
        let txs_to_speedup: Vec<Transaction> =
            txs_data.iter().map(|tx| tx.tx.clone()).rev().collect();
        // TODO: This logic may need to be updated to use OutputType from the protocol builder for greater flexibility.
        // Currently, we derive the change address as a P2PKH address from the funding UTXO's public key.
        let compressed = CompressedPublicKey::try_from(funding.pub_key).unwrap();
        let change_address = Address::p2wpkh(&compressed, self.network);
        let target_feerate_sat_vb = self.client.estimate_smart_fee()?;

        let utxos: Vec<Utxo> = txs_data
            .iter()
            .filter_map(|tx_data| tx_data.cpfp_utxo.clone())
            .collect();

        // SMALL TICK:
        // - Create the child tx with an empty fee to get the vsize of the tx.
        // - Then we use child_vbytes to calculate the total fee.
        // - Now we have the total fee, we can create the speedup tx.
        let child_vbytes = (ProtocolBuilder {})
            .speedup_transactions(
                &utxos,
                funding.clone(),
                change_address.clone(),
                0, // Dummy fee
                &self.key_manager,
            )?
            .vsize();

        let speedup_fee = self.calculate_speedup_fee(
            &txs_to_speedup,
            child_vbytes,
            target_feerate_sat_vb.to_sat(),
            bump_percent,
        )?;

        let speedup_tx = (ProtocolBuilder {}).speedup_transactions(
            &utxos,
            funding.clone(),
            change_address,
            speedup_fee as u64,
            &self.key_manager,
        )?;

        let change_output = speedup_tx.output.last().unwrap();
        let speedup_tx_id = speedup_tx.compute_txid();
        let txids: Vec<Txid> = txs_to_speedup.iter().map(|tx| tx.compute_txid()).collect();

        info!(
            "{} New Speedup({}) | Fee({}) | Transactions#({}) | FundingTx({})",
            style("Coordinator").green(),
            style(speedup_tx_id).blue(),
            style(speedup_fee).blue(),
            style(txids.len()).blue(),
            style(funding.txid).blue()
        );

        let new_funding_utxo = Utxo::new(
            speedup_tx_id,
            0, // After creating the speedup tx we know that the vout is 0.
            change_output.value.to_sat(),
            &funding.pub_key,
        );

        let cpfp = CoordinatedSpeedUpTransaction::new(
            speedup_tx_id,
            txids,
            speedup_fee,
            new_funding_utxo,
            false,
            0,
            SpeedupState::Dispatched,
            CPFP_TRANSACTION_CONTEXT.to_string(),
        );

        let cpfp_txid = cpfp.tx_id;

        self.store.save_speedup(cpfp)?;

        self.monitor.monitor(TypesToMonitor::Transactions(
            vec![cpfp_txid],
            CPFP_TRANSACTION_CONTEXT.to_string(),
        ))?;

        self.dispatch_txs(txs_data)?;

        info!(
            "{} Dispatch Speedup Transaction({})",
            style("Coordinator").green(),
            style(speedup_tx_id).yellow()
        );

        Ok(())
    }

    fn rbf_last_speedup(&self) -> Result<(), BitcoinCoordinatorError> {
        // We replace the last speedup transaction
        // TODO: Implement this function.
        // Esta funcion reconstruye la transaction cpfp y la envia a la red nuevmente con un fee mas alto.
        // Para reconstruir la transaction cpfp, se obtiene la ultima transaction speedup y se obtienen las transacciones hijas.

        // let child_tx_ids = txs.iter().map(|tx| tx.tx_id).collect();

        // let mut txs_to_speedup: Vec<CoordinatedTransaction> = Vec::new();

        // for tx_id in child_tx_ids {
        //     let tx = self.store.get_tx(&tx_id)?;
        //     txs_to_speedup.push(tx);
        // }

        // let last_bump_fee = self.get_bump_strategy(last_speedup_tx.fee)?;
        // self.create_and_send_cpfp_tx(txs_to_speedup, last_bump_fee)?;

        Ok(())
    }

    fn calculate_speedup_fee(
        &self,
        parents: &[Transaction],
        child_vbytes: usize,
        target_feerate_sat_vb: u64,
        bump_percent: f64,
    ) -> Result<f64, BitcoinCoordinatorError> {
        if target_feerate_sat_vb == 0 {
            return Err(BitcoinCoordinatorError::BitcoinCoordinatorError(
                "Target feerate must be greater than 0.".to_string(),
            ));
        }

        let parent_vbytes: usize = parents.iter().map(|tx| tx.vsize()).sum();

        let total_vbytes = parent_vbytes + child_vbytes;

        let bumped_feerate = target_feerate_sat_vb as f64 * bump_percent;

        let required_total_fee = (total_vbytes as f64 * bumped_feerate).ceil();

        Ok(required_total_fee)
    }

    fn get_bump_strategy(&self, last_fee_rate: f64) -> Result<f64, BitcoinCoordinatorError> {
        // TODO: Improve fee bumping strategy.
        // Currently, we simply increase the fee rate by 10%.
        // In the future, this should consider current mempool conditions, network fee estimates,
        // and urgency (e.g., how many blocks the transaction has been unconfirmed).
        let bumped_feerate = last_fee_rate * 1.1;
        Ok(bumped_feerate)
    }
}

impl BitcoinCoordinatorApi for BitcoinCoordinator {
    fn tick(&self) -> Result<(), BitcoinCoordinatorError> {
        self.monitor.tick()?;
        // The monitor is considered ready when it has fully indexed the blockchain and is up to date with the latest block.
        // Note that if there is a significant gap in the indexing process, it may take multiple ticks for the monitor to become ready.
        if !(self.monitor.is_ready()?) {
            return Ok(());
        }

        self.process_pending_txs_to_dispatch()?;
        self.process_in_progress_txs()?;
        self.process_in_progress_speedup_txs()?;

        let should_bump_last_speedup = self.store.has_reached_max_unconfirmed_speedups()?;

        if should_bump_last_speedup {
            self.rbf_last_speedup()?;
        }

        Ok(())
    }

    fn monitor(&self, data: TypesToMonitor) -> Result<(), BitcoinCoordinatorError> {
        if let TypesToMonitor::Transactions(txs, _) = data.clone() {
            if txs.is_empty() {
                return Err(BitcoinCoordinatorError::BitcoinCoordinatorError(
                    "transactions array is empty".to_string(),
                ));
            }
        }

        self.monitor.monitor(data)?;

        Ok(())
    }

    fn is_ready(&self) -> Result<bool, BitcoinCoordinatorError> {
        //TODO: The coordinator is currently considered ready when the monitor is ready.
        // However, we may decide to take into consideration pending and in progress transactions in the future.
        Ok(self.monitor.is_ready()?)
    }

    fn dispatch(
        &self,
        tx: Transaction,
        cpfp: Option<Utxo>,
        context: String,
        target_block_height: Option<BlockHeight>,
    ) -> Result<(), BitcoinCoordinatorError> {
        let to_monitor = TypesToMonitor::Transactions(vec![tx.compute_txid()], context.clone());
        self.monitor.monitor(to_monitor)?;

        // Save the transaction to be dispatched.
        self.store
            .save_tx(tx.clone(), cpfp, target_block_height, context)?;

        info!(
            "{} Dispatch Transaction({})",
            style("Coordinator").green(),
            style(tx.compute_txid()).yellow()
        );

        Ok(())
    }

    fn cancel(&self, data: TypesToMonitor) -> Result<(), BitcoinCoordinatorError> {
        self.monitor.cancel(data.clone())?;

        if let TypesToMonitor::Transactions(txs, _) = data {
            for tx in txs {
                self.store.remove_tx(tx)?;
            }
        }

        Ok(())
    }

    fn get_transaction(&self, txid: Txid) -> Result<TransactionStatus, BitcoinCoordinatorError> {
        let tx_status = self.monitor.get_tx_status(&txid)?;
        Ok(tx_status)
    }

    fn add_funding(&self, utxo: Utxo) -> Result<(), BitcoinCoordinatorError> {
        // Each time a speedup transaction is generated, it consumes the previous funding UTXO and leaves any change as the new funding for subsequent speedups.
        // Therefore, every new funding UTXO should be recorded in the same format as a speedup transaction, ensuring the coordinator always tracks the latest available funding.

        self.store.add_funding(utxo)?;

        Ok(())
    }

    fn get_news(&self) -> Result<News, BitcoinCoordinatorError> {
        let list_monitor_news = self.monitor.get_news()?;

        //TODO: Remove transactions new that are speed up transactions.
        let monitor_news = list_monitor_news
            .into_iter()
            .filter(|tx| {
                if let MonitorNews::Transaction(_, _, context_data) = tx {
                    !context_data.contains(CPFP_TRANSACTION_CONTEXT)
                } else {
                    true
                }
            })
            .collect();

        let coordinator_news = self.store.get_news()?;

        Ok(News::new(monitor_news, coordinator_news))
    }

    fn ack_news(&self, news: AckNews) -> Result<(), BitcoinCoordinatorError> {
        match news {
            AckNews::Monitor(news) => self.monitor.ack_news(news)?,
            AckNews::Coordinator(news) => self.store.ack_news(news)?,
        }
        Ok(())
    }
}

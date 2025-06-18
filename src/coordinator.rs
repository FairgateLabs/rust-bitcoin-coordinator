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
                .partition(|tx| self.should_speedup(tx));

        if !txs_to_dispatch_without_speedup.is_empty() {
            info!(
                "{} Number of transactions to dispatch without speedup {}",
                style("Coordinator").green(),
                style(txs_to_dispatch_without_speedup.len()).yellow()
            );

            self.dispatch_txs(txs_to_dispatch_without_speedup)?;
        }

        if !txs_to_dispatch_with_speedup.is_empty() {
            info!(
                "{} Number of transactions to dispatch with speedup {}",
                style("Coordinator").green(),
                style(txs_to_dispatch_with_speedup.len()).yellow()
            );
            // Check if we can send transactions or we stop the process until CPFP transactions start to be confirmed.
            if self.store.can_speedup()? {
                // TODO: Transaction that don't need speedup should be dispatched
                self.speedup_and_dispatch_in_batch(txs_to_dispatch_with_speedup)?;
            } else {
                info!("{} Can not speedup", style("Coordinator").green());
            }
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

            // We need to pay for transactions that were sent. If there is no transactions sent, we don't need to create a CPFP.
            if !txs_sent.is_empty() {
                let bump_fee_porcentage = self.get_bump_fee_porcentage_strategy(0)?;

                let funding = self.store.get_funding()?;

                if funding.is_none() {
                    let news = CoordinatorNews::FundingNotFound();
                    self.store.add_news(news)?;
                    return Ok(());
                }

                let funding = funding.unwrap();

                self.create_and_send_cpfp_tx(txs_sent, funding, bump_fee_porcentage, false)?;
            }
        }

        Ok(())
    }

    fn dispatch_speedup(
        &self,
        tx: Transaction,
        speedup_data: CoordinatedSpeedUpTransaction,
    ) -> Result<(), BitcoinCoordinatorError> {
        let speedup_type = speedup_data.get_tx_name();

        info!(
            "{} Send {} Transaction({})",
            style("Coordinator").green(),
            speedup_type,
            style(speedup_data.tx_id).yellow(),
        );

        let dispatch_result = self.client.send_transaction(&tx);

        match dispatch_result {
            Ok(_) => {
                self.monitor.monitor(TypesToMonitor::Transactions(
                    vec![speedup_data.tx_id],
                    CPFP_TRANSACTION_CONTEXT.to_string(),
                ))?;

                self.store.save_speedup(speedup_data)?;
            }
            Err(e) => {
                error!(
                    "{} Error Sending {} Transaction({})",
                    style("Coordinator").green(),
                    speedup_type,
                    style(speedup_data.tx_id).blue()
                );

                let news = CoordinatorNews::DispatchTransactionError(
                    speedup_data.tx_id,
                    CPFP_TRANSACTION_CONTEXT.to_string(),
                    e.to_string(),
                );

                self.store.add_news(news)?;
            }
        }

        // TODO: Implement this function.
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
            // Get updated transaction status from monitor
            let tx_status = self.monitor.get_tx_status(&tx.tx_id);

            match tx_status {
                Ok(tx_status) => {
                    info!(
                        "{} {} Transaction({}) | Confirmations({})",
                        style("Coordinator").green(),
                        tx.get_tx_name(),
                        style(tx.tx_id).blue(),
                        style(tx_status.confirmations).blue(),
                    );
                    // Handle the case where the transaction is a CPFP (Child Pays For Parent) transaction.

                    // First we acknowledge the transaction to clear any related news.
                    let ack = AckMonitorNews::Transaction(tx_status.tx_id);
                    self.monitor.ack_news(ack)?;

                    if tx_status.confirmations >= MAX_MONITORING_CONFIRMATIONS {
                        // Once the transaction is finalized, we are not monitoring it anymore.
                        self.store
                            .update_speedup_state(tx_status.tx_id, SpeedupState::Finalized)?;
                        continue;
                    }

                    if tx_status.is_confirmed() {
                        // We want to keep the the confirmation on the storage to  calculate the maximum speedups
                        self.store
                            .update_speedup_state(tx_status.tx_id, SpeedupState::Confirmed)?;
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
            // Get updated transaction status from monitor
            let tx_status = self.monitor.get_tx_status(&tx.tx_id);

            match tx_status {
                Ok(tx_status) => {
                    info!(
                        "{} Transaction({}) | Confirmations({})",
                        style("Coordinator").green(),
                        style(tx.tx_id).blue(),
                        style(tx_status.confirmations).blue(),
                    );

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

    fn should_speedup(&self, tx: &CoordinatedTransaction) -> bool {
        // If the transaction has a CPFP UTXO, we should speed up it.
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
        funding: Utxo,
        fee_porcentage: f64,
        is_rbf: bool,
    ) -> Result<(), BitcoinCoordinatorError> {
        // TODO: This logic may need to be updated to use OutputType from the protocol builder for greater flexibility.
        // Currently, we derive the change address as a P2PKH address from the funding UTXO's public key.
        let compressed = CompressedPublicKey::try_from(funding.pub_key).unwrap();
        let change_address = Address::p2wpkh(&compressed, self.network);

        let utxos: Vec<Utxo> = txs_data
            .iter()
            .filter_map(|tx_data| tx_data.cpfp_utxo.clone())
            .collect();

        // SMALL TICK:
        // - Create the child tx with an dummy fee to get the vsize of the tx.
        // - Then we use child_vbytes to calculate the total fee.
        // - Now we have the total fee, we can create the speedup tx.
        let child_vbytes = (ProtocolBuilder {})
            .speedup_transactions(
                &utxos,
                funding.clone(),
                change_address.clone(),
                10000, // Dummy fee
                &self.key_manager,
            )?
            .vsize();

        let speedup_fee =
            self.calculate_speedup_fee(&txs_data, child_vbytes, fee_porcentage, is_rbf)?;

        let speedup_tx = (ProtocolBuilder {}).speedup_transactions(
            &utxos,
            funding.clone(),
            change_address,
            speedup_fee,
            &self.key_manager,
        )?;

        let speedup_tx_id = speedup_tx.compute_txid();
        let txids: Vec<Txid> = txs_data.iter().map(|tx| tx.tx_id).collect();

        let speedup_type = if is_rbf { "RBF" } else { "CPFP" };

        info!(
            "{} New {} Transaction({}) | Fee({}) | Transactions#({}) | FundingTx({})",
            style("Coordinator").green(),
            speedup_type,
            style(speedup_tx_id).blue(),
            style(speedup_fee).blue(),
            style(txids.len()).blue(),
            style(funding.txid).blue()
        );

        let new_funding_utxo = Utxo::new(
            speedup_tx_id,
            0, // After creating the speedup tx we know that the vout is 0.
            speedup_tx.output.last().unwrap().value.to_sat(),
            &funding.pub_key,
        );

        let monitor_height = self.monitor.get_monitor_height()?;

        let speedup_data = CoordinatedSpeedUpTransaction::new(
            speedup_tx_id,
            txids,
            funding,
            new_funding_utxo,
            is_rbf,
            monitor_height,
            SpeedupState::Dispatched,
        );

        self.dispatch_speedup(speedup_tx, speedup_data)?;

        Ok(())
    }

    fn rbf_last_speedup(&self) -> Result<(), BitcoinCoordinatorError> {
        let (speedup, replace_speedup_count) = self.store.get_speedup_to_replace()?;

        let child_tx_ids = speedup.child_tx_ids;

        let mut txs_to_speedup: Vec<CoordinatedTransaction> = Vec::new();

        for tx_id in child_tx_ids {
            let tx = self.store.get_tx(&tx_id)?;
            txs_to_speedup.push(tx);
        }

        let bump_fee_porcentage =
            self.get_bump_fee_porcentage_strategy(replace_speedup_count + 1)?;

        self.create_and_send_cpfp_tx(
            txs_to_speedup,
            speedup.prev_funding,
            bump_fee_porcentage,
            true,
        )?;

        Ok(())
    }

    fn calculate_speedup_fee(
        &self,
        parents: &[CoordinatedTransaction],
        child_vbytes: usize,
        bump_fee_percentage: f64,
        is_replacement: bool,
    ) -> Result<u64, BitcoinCoordinatorError> {
        // Assumes that each parent transaction pays 1 sat/vbyte.
        // To calculate the total fee, we need to know the vsize of the child (CPFP) + the vsize of each parent.
        // Also we have to subtract the parent's transaction vbytes and the total output amounts once.

        let target_feerate_sat_vb = self.client.estimate_smart_fee()? as usize;
        let parent_vbytes: usize = parents.iter().map(|tx_data| tx_data.tx.vsize()).sum();

        let mut parent_amount_outputs: usize = 0;

        for tx_data in parents {
            if let Some(utxo) = &tx_data.cpfp_utxo {
                let tx_vout_amount = tx_data.tx.output[utxo.vout as usize].value;
                parent_amount_outputs += tx_vout_amount.to_sat() as usize;
            }
        }

        // We substract the vbytes of the parents and the amount of outputs.
        // Because the child pays for the parents and the parents pay for the outputs
        let parent_total_sats = parent_vbytes * target_feerate_sat_vb;
        let child_total_sats = child_vbytes * target_feerate_sat_vb;
        let total_sats = parent_total_sats + child_total_sats;
        let total_fee = (total_sats as f64 * bump_fee_percentage).ceil().round() as u64;
        let total_fee = total_fee
            .saturating_sub(parent_amount_outputs as u64)
            .saturating_sub(parent_vbytes as u64);

        if is_replacement && total_fee < child_total_sats as u64 {
            // Somethimes new calculated fee for the child tx is less than the previous child tx (+-1).
            // In this case we add 10 sats to the fee to avoid underpaying.
            let fee_to_add = child_total_sats as u64 + 10;
            return Ok(fee_to_add);
        }

        Ok(total_fee)
    }

    fn get_bump_fee_porcentage_strategy(
        &self,
        previous_count_rbf: u64,
    ) -> Result<f64, BitcoinCoordinatorError> {
        if previous_count_rbf == 0 {
            return Ok(1.0);
        }

        // Strategy explanation:
        // This function determines the bumping strategy for increasing the fee rate when performing a Speedup on a transaction.
        // The input `previous_count_rbf` represents how many times the transaction has already been replaced/bumped.
        // The current approach is simple: for each previous RBF, we multiply the count by 1.5 to get the new bump factor.
        // For example, if this is the first RBF (previous_count_rbf == 1), the bump factor is 1.5.
        // If this is the second RBF (previous_count_rbf == 2), the bump factor is 3.0, and so on.
        // This means the fee rate increases linearly with the number of RBF attempts, scaled by 1.5.

        let bumped_feerate = previous_count_rbf as f64 * 1.5;
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

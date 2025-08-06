use crate::{
    config::CoordinatorSettings,
    errors::BitcoinCoordinatorError,
    settings::CPFP_TRANSACTION_CONTEXT,
    speedup::SpeedupStore,
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::{
        AckNews, CoordinatedSpeedUpTransaction, CoordinatedTransaction, CoordinatorNews, News,
        SpeedupState, TransactionState,
    },
};
use bitcoin::{Network, Transaction, Txid};
use bitvmx_bitcoin_rpc::{bitcoin_client::BitcoinClient, rpc_config::RpcConfig};
use bitvmx_bitcoin_rpc::{bitcoin_client::BitcoinClientApi, types::BlockHeight};
use bitvmx_transaction_monitor::{
    errors::MonitorError,
    monitor::{Monitor, MonitorApi},
    types::{AckMonitorNews, MonitorNews, MonitorType, TransactionStatus, TypesToMonitor},
};
use console::style;
use key_manager::key_manager::KeyManager;
use protocol_builder::{
    builder::ProtocolBuilder,
    types::{output::SpeedupData, Utxo},
};
use std::{rc::Rc, vec};
use storage_backend::storage::Storage;
use tracing::{debug, error, info, warn};

pub struct BitcoinCoordinator {
    monitor: MonitorType,
    key_manager: Rc<KeyManager>,
    store: BitcoinCoordinatorStore,
    client: BitcoinClient,
    _network: Network,
    settings: CoordinatorSettings,
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
    /// * `data` - The data to monitor
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
        speedup: Option<SpeedupData>,
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
        settings: Option<CoordinatorSettings>,
    ) -> Result<Self, BitcoinCoordinatorError> {
        let settings = settings.unwrap_or_default();

        let monitor = Monitor::new_with_paths(
            rpc_config,
            storage.clone(),
            Some(settings.monitor_settings.clone()),
        )?;

        let store = BitcoinCoordinatorStore::new(storage, settings.max_unconfirmed_speedups)?;
        let client = BitcoinClient::new_from_config(rpc_config)?;
        let network = rpc_config.network;

        Ok(Self {
            monitor,
            store,
            key_manager,
            client,
            _network: network,
            settings,
        })
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
                self.speedup_and_dispatch_in_batch(txs_to_dispatch_with_speedup)?;
            } else {
                warn!("{} Can not speedup", style("Coordinator").green());
                let is_funding_available = self.store.is_funding_available()?;

                if !is_funding_available {
                    self.notify_funding_not_found()?;
                }
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
        // We have two policies to dispatch the transactions:
        // 1. Maximum transaction size: MAX_TX_WEIGHT weight units. Exceeding these limits will result in the transaction
        // being considered non-standard and rejected by most mempools.
        // 2. Maximum number of unconfirmed transactions is 25 (MAX_LIMIT_UNCONFIRMED_PARENTS)
        // If the set of transactions exceeds these limits, will fail the dispatch.

        let txs_in_batch_by_policies: Vec<Vec<CoordinatedTransaction>> =
            self.batch_txs_by_weight_limit(txs)?;

        for txs_batch in txs_in_batch_by_policies {
            // For each batch, attempt to broadcast all transactions individually. After determining which transactions were successfully sent,
            // construct and broadcast a single CPFP transaction to pay for the entire batch.
            let txs_sent: Vec<CoordinatedTransaction> = self.dispatch_txs(txs_batch)?;

            info!(
                "{} Sending batch of {} transactions",
                style("Coordinator").green(),
                txs_sent.len()
            );
            // Only create a CPFP (Child Pays For Parent) transaction if there are transactions that were successfully sent in this batch.
            // If no transactions were sent, skip CPFP creation for this batch.
            if !txs_sent.is_empty() {
                let txs_data = txs_sent
                    .iter()
                    .map(|coordinated_tx| {
                        (
                            coordinated_tx.speedup_data.clone().unwrap(),
                            coordinated_tx.tx.clone(),
                        )
                    })
                    .collect();
                // Up to here we have funding and we are sure we have funding.
                let funding = self.store.get_funding()?.unwrap();
                self.create_and_send_cpfp_tx(txs_data, funding, 1.0, None)?;
            }
        }

        Ok(())
    }

    fn notify_funding_not_found(&self) -> Result<(), BitcoinCoordinatorError> {
        let news = CoordinatorNews::FundingNotFound;
        self.store.add_news(news)?;
        Ok(())
    }

    // This function is designed to expedite a CPFP (Child Pays For Parent) transaction.
    // It achieves this by creating an additional CPFP transaction to provide further funding to the previous one.
    // It is ensured that funding is available before invoking this function.
    fn speedup_cpfp_tx(&self) -> Result<(), BitcoinCoordinatorError> {
        let funding = self.store.get_funding()?.unwrap();

        let last_speedup = self.store.get_last_speedup()?;

        if let Some((speedup, _)) = last_speedup {
            let bump_fee_percentage =
                self.get_bump_fee_percentage_strategy(speedup.bump_fee_percentage_used)?;

            info!(
                "{} Boosting CPFP Transaction({})",
                style("Coordinator").green(),
                style(speedup.tx_id).yellow()
            );
            self.create_and_send_cpfp_tx(vec![], funding, bump_fee_percentage, None)?;
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
                    style(speedup_data.tx_id).yellow()
                );

                let news = CoordinatorNews::DispatchTransactionError(
                    speedup_data.tx_id,
                    CPFP_TRANSACTION_CONTEXT.to_string(),
                    e.to_string(),
                );

                self.store.add_news(news)?;
            }
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

                    // TODO: Handle specific errors when we send a tx and decide what to do.
                    let error_msg = e.to_string();

                    // let coordinator_error = if error_msg.contains("already in mempool") {
                    //     BitcoinCoordinatorError::TransactionAlreadyInMempool(tx.tx_id.to_string())
                    // } else if error_msg.contains("mempool full")
                    //     || error_msg.contains("insufficient priority")
                    // {
                    //     BitcoinCoordinatorError::MempoolFull(error_msg.clone())
                    // } else if error_msg.contains("network") || error_msg.contains("connection") {
                    //     BitcoinCoordinatorError::NetworkError(error_msg.clone())
                    // } else {
                    //     BitcoinCoordinatorError::BitcoinClientError(e)
                    // };

                    let news = CoordinatorNews::DispatchTransactionError(
                        tx.tx_id,
                        tx.context.clone(),
                        error_msg,
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
        let mut allow_unconfirmed_txs = self.store.get_available_unconfirmed_txs()?;

        for tx_data in txs {
            let weight = tx_data.tx.weight().to_wu();

            if weight > self.settings.max_tx_weight {
                return Err(BitcoinCoordinatorError::TransactionTooHeavy(
                    tx_data.tx_id.to_string(),
                    weight,
                    self.settings.max_tx_weight,
                ));
            }

            // When adding this transaction, we're extending the mempool ancestry chain: the new CPFP (Child Pays For Parent) transaction becomes,
            // for example, the 26th ancestor in the mempool's view.
            // Therefore, we must decrement the available unconfirmed CPFP slots,
            // since each batch will require a new CPFP transaction and further extend the ancestry chain.
            if allow_unconfirmed_txs - 1 > 0 {
                allow_unconfirmed_txs -= 1;
            } else {
                batches.push(current_batch);
                // Up to here we have reached the limit of unconfirmed txs. We can't dispatch more txs.
                return Ok(batches);
            }

            if current_weight + weight > self.settings.max_tx_weight {
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

                    if tx_status
                        .is_finalized(self.settings.monitor_settings.max_monitoring_confirmations)
                    {
                        // Once the transaction is finalized, we are not monitoring it anymore.
                        self.store
                            .update_speedup_state(tx_status.tx_id, SpeedupState::Finalized)?;
                        continue;
                    }

                    if tx_status.is_confirmed() {
                        // We want to keep the confirmation on the storage to calculate the maximum speedups
                        self.store
                            .update_speedup_state(tx_status.tx_id, SpeedupState::Confirmed)?;
                        continue;
                    }

                    if tx_status.is_orphan() {
                        self.store
                            .update_speedup_state(tx_status.tx_id, SpeedupState::Dispatched)?;
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
                        style(tx.tx_id).yellow(),
                        style(tx_status.confirmations).blue(),
                    );

                    if tx_status
                        .is_finalized(self.settings.monitor_settings.max_monitoring_confirmations)
                    {
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
        // If the transaction has a CPFP UTXO, we have to speed it up.
        tx.speedup_data.is_some()
    }

    fn should_dispatch_tx(
        &self,
        pending_tx: &CoordinatedTransaction,
    ) -> Result<bool, BitcoinCoordinatorError> {
        if pending_tx.target_block_height.is_none() {
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
        txs_data: Vec<(SpeedupData, Transaction)>,
        funding: Utxo,
        bump_fee: f64,
        cpfp_id_to_replace: Option<Txid>,
    ) -> Result<(), BitcoinCoordinatorError> {
        // Check if the funding amount is below the minimum required for a speedup.
        // If so, notify via CoordinatorNews and exit early.
        if funding.amount < self.settings.min_funding_amount_sats {
            let news = CoordinatorNews::InsufficientFunds(
                funding.txid,
                funding.amount,
                self.settings.min_funding_amount_sats,
            );
            self.store.add_news(news)?;
            return Ok(());
        }

        let is_rbf = cpfp_id_to_replace.is_some();

        let txs_speedup_data = txs_data
            .iter()
            .map(|(speedup_data, tx)| (speedup_data.clone(), tx.vsize()))
            .collect();

        let new_network_fee_rate = self.get_network_fee_rate()?;

        let diff_fee_for_unconfirmed_chain =
            self.get_diff_fee_for_unconfirmed_chain(new_network_fee_rate)?;

        let (speedup_tx, speedup_fee) = self.get_speedup_tx(
            &txs_speedup_data,
            &funding,
            bump_fee,
            is_rbf,
            new_network_fee_rate,
            diff_fee_for_unconfirmed_chain,
        )?;
        // Validate that funding can cover the fee
        if speedup_fee > funding.amount {
            let news =
                CoordinatorNews::InsufficientFunds(funding.txid, funding.amount, speedup_fee);
            self.store.add_news(news)?;
            return Ok(());
        }

        let speedup_tx_id = speedup_tx.compute_txid();
        let txids: Vec<Txid> = txs_data.iter().map(|(_, tx)| tx.compute_txid()).collect();

        let speedup_type = if is_rbf { "RBF" } else { "CPFP" };
        let mut cpfp_to_replace = String::new();

        if is_rbf {
            cpfp_to_replace = format!("| CPFP_TO_REPLACE({})", cpfp_id_to_replace.unwrap());
        }

        info!(
            "{} New {} Transaction({}) | Fee({}) | Transactions#({}) | FundingTx({}) {} | BumpFee({})",
            style("Coordinator").green(),
            speedup_type,
            style(speedup_tx_id).yellow(),
            style(speedup_fee).blue(),
            style(txids.len()).blue(),
            style(funding.txid).blue(),
            style(cpfp_to_replace).blue(),
            style(bump_fee).blue(),
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
            funding,
            new_funding_utxo,
            is_rbf,
            monitor_height,
            SpeedupState::Dispatched,
            bump_fee,
            txs_data,
            new_network_fee_rate,
        );

        self.dispatch_speedup(speedup_tx, speedup_data)?;

        Ok(())
    }

    fn get_diff_fee_for_unconfirmed_chain(
        &self,
        new_network_fee_rate: u64,
    ) -> Result<u64, BitcoinCoordinatorError> {
        let speedups_unconfirmed = self.store.get_unconfirmed_speedups()?;

        if speedups_unconfirmed.is_empty() {
            return Ok(0);
        }

        let last_fee_rate_used = speedups_unconfirmed.last().unwrap().network_fee_rate_used;

        if last_fee_rate_used >= new_network_fee_rate {
            return Ok(0);
        }

        let mut fee_chain_difference = 0;

        for speedup in speedups_unconfirmed {
            let fee_rate_to_pay = new_network_fee_rate.saturating_sub(last_fee_rate_used);
            let txs_data = speedup
                .speedup_tx_data
                .iter()
                .map(|(speedup_data, tx)| (speedup_data.clone(), tx.vsize()))
                .collect();

            let (_, fee_to_pay) = self.get_speedup_tx(
                &txs_data,
                &speedup.prev_funding,
                1.0, // We should not bump this fee, we are just calculating the difference.
                speedup.is_rbf,
                fee_rate_to_pay,
                0,
            )?;

            fee_chain_difference += fee_to_pay;
        }

        Ok(fee_chain_difference)
    }

    fn get_network_fee_rate(&self) -> Result<u64, BitcoinCoordinatorError> {
        let mut network_fee_rate: u64 = self.client.estimate_smart_fee()?;

        if network_fee_rate > self.settings.max_feerate_sat_vb {
            warn!(
                "{} Estimate feerate sat/vbyte is greater than the max allowed. This could be a bug. | EstimateFeerate({}) | MaxAllowed({})",
                style("Coordinator").red(),
                style(network_fee_rate).red(),
                style(self.settings.max_feerate_sat_vb).red(),
            );

            // Inform this with news
            let news = CoordinatorNews::EstimateFeerateTooHigh(
                network_fee_rate,
                self.settings.max_feerate_sat_vb,
            );

            self.store.add_news(news)?;

            // Set the estimate feerate to the max allowed
            network_fee_rate = self.settings.max_feerate_sat_vb;
        }
        Ok(network_fee_rate)
    }

    fn get_speedup_tx(
        &self,
        txs_data: &Vec<(SpeedupData, usize)>,
        funding: &Utxo,
        bump_fee_percentage: f64,
        is_rbf: bool,
        network_fee_rate: u64,
        diff_fee_for_unconfirmed_chain: u64,
    ) -> Result<(Transaction, u64), BitcoinCoordinatorError> {
        let speedups_data: Vec<SpeedupData> =
            txs_data.iter().map(|tx_data| tx_data.0.clone()).collect();

        // TRICK:
        // - Create the child transaction with a dummy fee to determine the transaction's virtual size (vsize).
        // - Use the vsize of the child transaction to calculate the total fee required.
        // - With the total fee calculated, create the final speedup transaction.
        // - If the child vsize is greater than or equal to the final speedup vsize,
        //  means the final speedup has sufficient fee.
        // - Otherwise, we need to increase the fee, we will use the final speedup vsize as the new child vsize.

        let mut child_vsize = 0;

        loop {
            let dummy_speedup_vsize = (ProtocolBuilder {})
                .speedup_transactions(
                    speedups_data.as_slice(),
                    funding.clone(),
                    &funding.pub_key,
                    10000, // Dummy fee
                    &self.key_manager,
                )?
                .vsize();

            if child_vsize == 0 {
                child_vsize = dummy_speedup_vsize;
            }

            let speedup_fee = self.calculate_speedup_fee(
                &txs_data,
                child_vsize,
                bump_fee_percentage,
                network_fee_rate,
                is_rbf,
                diff_fee_for_unconfirmed_chain,
            )?;

            let final_speedup_tx = (ProtocolBuilder {}).speedup_transactions(
                &speedups_data,
                funding.clone(),
                &funding.pub_key,
                speedup_fee,
                &self.key_manager,
            )?;

            let final_speedup_vsize = final_speedup_tx.vsize();

            if child_vsize >= final_speedup_vsize {
                // If the child vsize is greater than or equal to the final speedup vsize,
                //  means the final speedup has sufficient fee.
                return Ok((final_speedup_tx, speedup_fee));
            } else {
                // Otherwise, we need to increase the fee, we will use the final speedup vsize as the new child vsize.
                child_vsize = final_speedup_vsize;
            }
        }
    }

    fn rbf_last_cpfp(&self) -> Result<(), BitcoinCoordinatorError> {
        // When this function is called, we know that the last speedup exists to be replaced.
        let (speedup, rbf_tx) = self.store.get_last_speedup()?.unwrap();

        let mut txs_to_speedup: Vec<CoordinatedTransaction> = Vec::new();

        for (_, tx) in speedup.speedup_tx_data.clone() {
            let tx = self.store.get_tx(&tx.compute_txid())?;
            txs_to_speedup.push(tx);
        }

        // The new_bump_fee will increase the previous bump fee from the CPFP used by adding the number of RBF operations performed + 1.
        let mut increase_last_bump_fee = speedup.bump_fee_percentage_used;

        if let Some(rbf_tx) = rbf_tx {
            increase_last_bump_fee = rbf_tx.bump_fee_percentage_used;
        }

        let new_bump_fee = self.get_bump_fee_percentage_strategy(increase_last_bump_fee)?;

        self.create_and_send_cpfp_tx(
            speedup.speedup_tx_data,
            speedup.prev_funding,
            new_bump_fee,
            Some(speedup.tx_id),
        )?;

        Ok(())
    }

    fn boost_cpfp_again(&self) -> Result<(), BitcoinCoordinatorError> {
        // Check if we can send transactions or we stop the process until CPFP transactions start to be confirmed.
        if self.store.can_speedup()? {
            self.speedup_cpfp_tx()?;
        } else {
            warn!("{} Can not speedup", style("Coordinator").green());

            let is_funding_available = self.store.is_funding_available()?;

            if !is_funding_available {
                self.notify_funding_not_found()?;
            }
        }

        Ok(())
    }

    fn calculate_speedup_fee(
        &self,
        tx_to_speedup_info: &[(SpeedupData, usize)],
        child_vbytes: usize,
        bump_fee_percentage: f64,
        network_fee_rate: u64,
        is_rbf: bool,
        fee_chain_difference: u64,
    ) -> Result<u64, BitcoinCoordinatorError> {
        // Assumes that each parent transaction pays 1 sat/vbyte.
        // To calculate the total fee, we need to know the vsize of the child (CPFP) + the vsize of each parent.
        // Also we have to subtract the parent's transaction vbytes and the total output amounts once.

        let mut parent_amount_outputs: usize = 0;
        let mut parent_vbytes: usize = 0;

        for (speedup_data, vsize) in tx_to_speedup_info {
            let amount = if let Some(utxo) = &speedup_data.utxo {
                utxo.amount as usize
            } else {
                speedup_data.partial_utxo.as_ref().unwrap().2 as usize
            };
            parent_amount_outputs += amount;
            parent_vbytes += vsize;
        }

        // We substract the vbytes of the parents and the amount of outputs.
        // Because the child pays for the parents and the parents pay for the outputs
        let parent_total_sats = parent_vbytes * network_fee_rate as usize;
        let child_total_sats = child_vbytes * network_fee_rate as usize;
        let total_sats = parent_total_sats + child_total_sats;

        let mut total_fee = total_sats
            .saturating_sub(parent_amount_outputs) // amount comming from the parents to discount
            .saturating_sub(parent_vbytes); // min relay fee of the parents to discount

        if is_rbf && total_fee < child_total_sats * 2 {
            // Bitcoin Policy (https://github.com/bitcoin/bitcoin/blob/master/doc/policy/mempool-replacements.md?plain=1#L32):
            // The additional fees (difference between absolute fee paid by the replacement transaction and the
            // sum paid by the original transactions) pays for the replacement transaction's bandwidth at or
            // above the rate set by the node's incremental relay feerate. For example, if the incremental relay
            // feerate is 1 satoshi/vB and the replacement transaction is 500 virtual bytes total, then the
            // replacement pays a fee at least 500 satoshis higher than the sum of the original transactions.

            // *Rationale*: Try to prevent DoS attacks where an attacker causes the network to repeatedly relay
            // transactions each paying a tiny additional amount in fees, e.g. just 1 satoshi.
            total_fee = child_total_sats * 2;
        }

        total_fee += fee_chain_difference as usize;

        let total_fee_bumped = (total_fee as f64 * bump_fee_percentage).ceil().round() as u64;

        // TODO IMPORTANT:
        // To accurately calculate the fee when the estimated fee changes over time, it is essential to retain the estimate_fee
        // used for each CPFP and recalculate the new value if the estimate_fee differs. Failing to do so may result in overpayment or underpayment.
        // In this scenario, we need to compute the fee difference between the parent transactions already sent in the previous CPFP chain and the new estimate_fee value.

        debug!(
            "{} EstimateNetworkFee({}) | ParentTotalSats({}) | ChildTotalSats({}) | BumpFeePercentage({}) | ParentAmountOutputs({}) | ParentVbytes({}) | TotalFee({})",
            style("Coordinator").green(),
            style(network_fee_rate).red(),
            style(parent_total_sats).red(),
            style(child_total_sats).red(),
            style(bump_fee_percentage).red(),
            style(parent_amount_outputs).red(),
            style(parent_vbytes).red(),
            style(total_fee_bumped).red(),
        );

        Ok(total_fee_bumped)
    }

    fn get_bump_fee_percentage_strategy(
        &self,
        prev_bump_fee: f64,
    ) -> Result<f64, BitcoinCoordinatorError> {
        if prev_bump_fee <= 0.0 {
            return Ok(1.0);
        }

        // This method increases the previous bump fee by 50%.
        // The `prev_bump_fee` parameter is the fee used in the last bump.
        // The new bump factor is calculated by multiplying the previous bump fee by 1.5.
        // For instance, if the previous bump fee was 1, the new bump factor becomes 1.5.
        // If the previous bump fee was 2, the new bump factor becomes 3.0.
        // This approach ensures a proportional increase in the fee rate with each bump attempt.

        info!(
            "{} Bumping fee from {} to {}",
            style("Coordinator").green(),
            style(prev_bump_fee).blue(),
            style(prev_bump_fee * 1.5).blue(),
        );
        let bumped_feerate = prev_bump_fee * 1.5;
        Ok(bumped_feerate)
    }

    fn should_rbf_last_speedup(&self) -> Result<bool, BitcoinCoordinatorError> {
        let reached_unconfirmed_speedups = self.store.has_reached_max_unconfirmed_speedups()?;

        if reached_unconfirmed_speedups {
            info!(
                "{} Reached max unconfirmed speedups.",
                style("Coordinator").green()
            );

            return Ok(true);
        }

        Ok(false)
    }

    fn should_boost_speedup_again(&self) -> Result<bool, BitcoinCoordinatorError> {
        let last_speedup = self.store.get_last_speedup()?;

        if let Some((speedup, rbf_tx)) = last_speedup {
            let current_block_height = self.monitor.get_monitor_height()?;
            // This block checks if the last speedup transaction should be replaced-by-fee.
            // It retrieves the last speedup transaction and the number of times it has already been replaced (replace_speedup_count).
            // The logic is: if the current block height is greater than the sum of the speedup's broadcast block height and the number of RBFs,
            // then enough blocks have passed without confirmation, so we should bump the fee again.
            // This helps ensure that stuck transactions are periodically rebroadcast with higher fees to improve their chances of confirmation.
            let last_broadcast_block_height = if let Some(rbf_tx) = rbf_tx {
                rbf_tx.broadcast_block_height
            } else {
                speedup.broadcast_block_height
            };

            if current_block_height.saturating_sub(last_broadcast_block_height)
                >= self.settings.min_blocks_before_resend_speedup
            {
                debug!(
                    "{} Last CPFP should be bumped | CurrentHeight({}) | BroadcastHeight({}) | MinBlocksBeforeRBF({})",
                    style("Coordinator").green(),
                    style(current_block_height).blue(),
                    style(last_broadcast_block_height).blue(),
                    style(self.settings.min_blocks_before_resend_speedup).blue(),
                );

                return Ok(true);
            }
        }

        Ok(false)
    }
}

impl BitcoinCoordinatorApi for BitcoinCoordinator {
    fn tick(&self) -> Result<(), BitcoinCoordinatorError> {
        self.monitor.tick()?;
        // The monitor is considered ready when it has fully indexed the blockchain and is up to date with the latest block.
        // Note that if there is a significant gap in the indexing process, it may take multiple ticks for the monitor to become ready.
        let is_ready = self.monitor.is_ready()?;

        let is_ready_str = if is_ready { "Ready" } else { "Not Ready" };
        info!("{} {}", style("Coordinator").green(), is_ready_str);

        if !is_ready {
            return Ok(());
        }

        self.process_pending_txs_to_dispatch()?;
        self.process_in_progress_txs()?;
        self.process_in_progress_speedup_txs()?;

        if self.should_boost_speedup_again()? {
            if self.should_rbf_last_speedup()? {
                self.rbf_last_cpfp()?;
                return Ok(());
            }

            self.boost_cpfp_again()?;
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
        // The coordinator is currently considered ready when the monitor is ready.
        Ok(self.monitor.is_ready()?)
    }

    fn dispatch(
        &self,
        tx: Transaction,
        speedup_data: Option<SpeedupData>,
        context: String,
        target_block_height: Option<BlockHeight>,
    ) -> Result<(), BitcoinCoordinatorError> {
        let to_monitor = TypesToMonitor::Transactions(vec![tx.compute_txid()], context.clone());
        self.monitor.monitor(to_monitor)?;

        // Save the transaction to be dispatched.
        self.store
            .save_tx(tx.clone(), speedup_data, target_block_height, context)?;

        info!(
            "{} Mark Transaction({}) to dispatch",
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

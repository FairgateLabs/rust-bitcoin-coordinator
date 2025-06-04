use crate::{
    errors::BitcoinCoordinatorError,
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::{
        AckNews, BitcoinCoordinatorType, CoordinatedTransaction, CoordinatorNews, News, SpeedUpTx,
        TransactionDispatchState,
    },
};
use bitcoin::{Address, CompressedPublicKey, Network, Transaction, Txid};
use bitvmx_bitcoin_rpc::{bitcoin_client::BitcoinClient, rpc_config::RpcConfig};
use bitvmx_bitcoin_rpc::{bitcoin_client::BitcoinClientApi, types::BlockHeight};
use bitvmx_transaction_monitor::{
    errors::MonitorError,
    monitor::{Monitor, MonitorApi},
    types::{
        AckMonitorNews, MonitorNews, TransactionBlockchainStatus, TransactionStatus, TypesToMonitor,
    },
};
use console::style;
use key_manager::key_manager::KeyManager;
use protocol_builder::{builder::ProtocolBuilder, types::Utxo};
use std::rc::Rc;
use storage_backend::storage::Storage;
use tracing::{error, info, warn};

pub struct BitcoinCoordinator<M, B, C>
where
    M: MonitorApi,
    B: BitcoinCoordinatorStoreApi,
    C: BitcoinClientApi,
{
    monitor: M,
    store: B,
    key_manager: Rc<KeyManager>,
    client: C,
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

impl BitcoinCoordinatorType {
    //#[warn(clippy::too_many_arguments)]
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
}

impl<M, B, C> BitcoinCoordinator<M, B, C>
where
    M: MonitorApi,
    B: BitcoinCoordinatorStoreApi,
    C: BitcoinClientApi,
{
    const SPEED_UP_CONTEXT: &str = "SPEED_UP_TRANSACTION";

    // Stop monitoring a transaction after 100 confirmations.
    // In case of a reorganization bigger than 100 blocks, we have to do a rework in the coordinator.
    const MAX_MONITORING_CONFIRMATIONS: u32 = 100;

    pub fn new(
        monitor: M,
        store: B,
        key_manager: Rc<KeyManager>,
        client: C,
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

    fn process_pending_txs(&self) -> Result<(), BitcoinCoordinatorError> {
        // Get pending transactions to be send to the blockchain
        let pending_txs = self
            .store
            .get_txs(TransactionDispatchState::PendingDispatch)?;

        if !pending_txs.is_empty() {
            info!(
                "{} Transactions to Dispatch #{}",
                style("Coordinator").green(),
                style(pending_txs.len()).yellow()
            );
        }

        for pending_tx in pending_txs {
            if !self.should_be_dispatch(&pending_tx)? {
                info!(
                    "{} Transaction {} should not be dispatched.",
                    style("Coordinator").green(),
                    style(pending_tx.tx_id).yellow()
                );
                continue;
            }

            let tx_id = pending_tx.tx.compute_txid();

            info!(
                "{} Send Transaction({})",
                style("Coordinator").green(),
                style(tx_id).yellow(),
            );

            let dispatch_result = self.client.send_transaction(&pending_tx.tx);

            if let Err(error) = dispatch_result {
                error!(
                    "{} Error Sending Transaction({})",
                    style("Coordinator").green(),
                    style(tx_id).blue()
                );
                let news = CoordinatorNews::DispatchTransactionError(
                    tx_id,
                    pending_tx.context,
                    error.to_string(),
                );
                self.store.add_news(news)?;

                self.store
                    .update_tx(tx_id, TransactionDispatchState::FailedToBroadcast)?;

                continue;
            }

            // Error here should be handled. And saved the error in news if needed.

            let deliver_block_height = self.monitor.get_monitor_height()?;

            self.store
                .update_tx_to_dispatched(tx_id, deliver_block_height)?;
        }

        Ok(())
    }

    fn should_be_dispatch(
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
                "Transaction {} has a target block height set but was already broadcasted.",
                pending_tx.tx_id
            );
            // This code path should not be reached because once a transaction is broadcast,
            // it should be marked as BroadcastPendingConfirmation.
            return Ok(false);
        }

        let current_block_height = self.monitor.get_monitor_height()?;

        Ok(current_block_height >= pending_tx.target_block_height.unwrap())
    }

    fn process_in_progress_txs(&self) -> Result<(), BitcoinCoordinatorError> {
        //TODO: THIS COULD BE IMPROVED.
        // If transaction still in sent means it should be speed up, and is not confirmed.
        // otherwise it should be moved as confirmed in the previous validations for news.
        let txs = self
            .store
            .get_txs(TransactionDispatchState::BroadcastPendingConfirmation)?;

        let mut txs_to_speedup: Vec<CoordinatedTransaction> = Vec::new();

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
                    if tx_status.confirmations >= Self::MAX_MONITORING_CONFIRMATIONS {
                        self.store
                            .update_tx(tx_status.tx_id, TransactionDispatchState::Finalized)?;

                        return Ok(());
                    }

                    if tx_status.is_orphan() {
                        // THIS IS A BORDER CASE.
                        // If the transaction is orphan, it means it has been removed from the blockchain.
                        // We should speed up the transaction.
                        txs_to_speedup.push(tx);
                    }
                }
                Err(MonitorError::TransactionNotFound(_)) => {
                    txs_to_speedup.push(tx);
                }
                Err(e) => return Err(e.into()),
            }
        }

        let txs_filtered: Vec<CoordinatedTransaction> = txs_to_speedup
            .into_iter()
            .filter(|tx| {
                if tx.context.contains(Self::SPEED_UP_CONTEXT) {
                    return false;
                }
                true
            })
            .collect();

        if !txs_filtered.is_empty() {
            self.perform_speed_up_in_batch(txs_filtered)?;
        }

        Ok(())
    }

    fn process_monitor_news(&self) -> Result<(), BitcoinCoordinatorError> {
        let list_news = self.monitor.get_news()?;

        for news in list_news {
            if let MonitorNews::Transaction(tx_id, tx_status, tx_context) = news {
                // Check if context_data contains the string "speed_up"

                if !tx_context.contains(Self::SPEED_UP_CONTEXT) {
                    // Skip processing this news as it is not a speed-up transaction
                    // TODO:
                    // Since there could be reorganizations larger than 6 blocks, we set 100 blocks,
                    // we could check if a transaction was at some point meant to be sent,
                    // and it became orphaned, in that case we should try to speed it up and move the state to BroadcastPendingConfirmation.
                    // This is because at some point we move it to finalized status, which means we stop monitoring it.
                    continue;
                }

                self.process_speedup(&tx_status)?;

                let ack = AckMonitorNews::Transaction(tx_id);
                self.monitor.ack_news(ack)?;
            }
        }

        Ok(())
    }

    fn perform_speed_up_in_batch(
        &self,
        txs: Vec<CoordinatedTransaction>,
    ) -> Result<(), BitcoinCoordinatorError> {
        let mut new_txs_to_speedup: Vec<CoordinatedTransaction> = Vec::new();

        for tx in txs.iter() {
            if self.should_speedup_tx(&tx)? {
                new_txs_to_speedup.push(tx.clone());
            }
        }

        if new_txs_to_speedup.is_empty() {
            info!(
                "{} No transactions need speedup.",
                style("Coordinator").green()
            );
            return Ok(());
        }

        // Otherwise we should speed up all the remaining transactions (including the ones that were speed up previously)

        let funding_tx_utxo = match self.store.get_funding()? {
            Some(utxo) => utxo,
            // No funding transaction found, we can not speed up transactions.
            None => return Ok(()),
        };

        let txs_to_speedup: Vec<Transaction> = txs.iter().map(|tx| tx.tx.clone()).rev().collect();
        // TODO: This logic may need to be updated to use OutputType from the protocol builder for greater flexibility.
        // Currently, we derive the change address as a P2PKH address from the funding UTXO's public key.
        let compressed = CompressedPublicKey::try_from(funding_tx_utxo.pub_key).unwrap();
        let change_address = Address::p2wpkh(&compressed, self.network);
        let target_feerate_sat_vb = self.client.estimate_smart_fee()?;
        let bump_percent = 1.1; // 10% more fee.

        let utxos: Vec<Utxo> = txs
            .iter()
            .filter_map(|tx_data| tx_data.speedup_utxo.clone())
            .collect();

        // SMALL TICK:
        // - Create the child tx with an empty fee to get the vsize of the tx.
        // - Then we use child_vbytes to calculate the total fee.
        // - Now we have the total fee, we can create the speedup tx.
        let child_vbytes = (ProtocolBuilder {})
            .speedup_transactions(
                &utxos,
                funding_tx_utxo.clone(),
                change_address.clone(),
                0, // Dummy fee
                &self.key_manager,
            )?
            .vsize();

        let speedup_fee: u64 = self.calculate_speedup_fee(
            &txs_to_speedup,
            child_vbytes,
            target_feerate_sat_vb.to_sat(),
            bump_percent,
        )?;

        let speedup_tx = (ProtocolBuilder {}).speedup_transactions(
            &utxos,
            funding_tx_utxo.clone(),
            change_address,
            speedup_fee,
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
            style(funding_tx_utxo.txid).blue()
        );

        // info!(
        //     "Speedup tx: {:#?}",
        //     speedup_tx
        //         .input
        //         .iter()
        //         .map(|input| input.previous_output.txid)
        //         .collect::<Vec<Txid>>()
        // );

        self.dispatch(
            speedup_tx.clone(),
            None,
            Self::SPEED_UP_CONTEXT.to_string(),
            None,
        )?;

        let deliver_block_height = self.monitor.get_monitor_height()?;

        let new_funding_utxo = Utxo::new(
            speedup_tx_id,
            0, // After creating the speedup tx we know that the vout is 0.
            change_output.value.to_sat(),
            &funding_tx_utxo.pub_key,
        );

        let speed_up_tx = SpeedUpTx::new(
            speedup_tx_id,
            deliver_block_height,
            txids,
            new_funding_utxo.clone(),
        );

        self.store.save_speedup_tx(&speed_up_tx)?;

        self.store.add_funding(new_funding_utxo)?;

        Ok(())
    }

    fn calculate_speedup_fee(
        &self,
        parents: &[Transaction],
        child_vbytes: usize,
        target_feerate_sat_vb: u64,
        bump_percent: f64,
    ) -> Result<u64, BitcoinCoordinatorError> {
        if target_feerate_sat_vb <= 0 {
            return Err(BitcoinCoordinatorError::BitcoinCoordinatorError(
                "Target feerate must be greater than 0.".to_string(),
            ));
        }

        let parent_vbytes: usize = parents.iter().map(|tx| tx.vsize()).sum();

        let total_vbytes = parent_vbytes + child_vbytes;

        let bumped_feerate = target_feerate_sat_vb as f64 * bump_percent;

        let required_total_fee = (total_vbytes as f64 * bumped_feerate).ceil() as u64;

        Ok(required_total_fee)
    }

    fn process_speedup(
        &self,
        tx_status: &TransactionStatus,
    ) -> Result<(), BitcoinCoordinatorError> {
        let speed_up_data = self.store.get_speedup_tx(&tx_status.tx_id)?;

        info!(
            "{} Speedup({}) for Transactions({}) Confirmation({})",
            style("Coordinator").green(),
            style(speed_up_data.tx_id).yellow(),
            style(format!("{:?}", speed_up_data.child_tx_ids)).cyan(),
            style(tx_status.confirmations).blue()
        );

        // This indicates that this is a speed-up transaction that has been mined with 1 confirmation,
        // which means it should be treated as the new funding transaction.

        if tx_status.is_confirmed() {
            if tx_status.confirmations == 1 {
                info!(
                    "{} New Funding({})",
                    style("Coordinator").green(),
                    style(tx_status.tx_id).blue()
                );

                // The transaction has received its first confirmation, indicating it is now included in a block.
                // At this point, the speed-up transaction becomes the new funding transaction for future operations.
                // self.store.add_funding(speed_up_data.utxo)?;
            }

            if tx_status.is_orphan() {
                //Speed up previouly was mined, now is orphan then, we have to remove it as a funding tx.
                self.store.remove_funding(tx_status.tx_id)?;
            }
        }

        if !tx_status.is_confirmed() {
            // If a speed-up transaction has not been seen (it has not been mined), no action is required.
            // The responsibility for creating a new speed-up transaction lies with the transaction that is delivered.
        }

        // In the event of a reorganization, we would need to do the opposite.
        // This involves removing the speed-up transaction and potentially replacing it with another transaction
        // that could take its place as the last speed-up transaction or become the new last funding transaction.

        Ok(())
    }

    fn should_speedup_tx(
        &self,
        tx_to_speedup: &CoordinatedTransaction,
    ) -> Result<bool, BitcoinCoordinatorError> {
        const SPEED_UP_THRESHOLD_BLOCKS: u32 = 0;

        if tx_to_speedup.context.contains(Self::SPEED_UP_CONTEXT) {
            // Speed up transaction should not be make to speedup.
            return Ok(false);
        }

        let current_block_height = self.monitor.get_monitor_height()?;

        if current_block_height - tx_to_speedup.broadcast_block_height.unwrap()
            < SPEED_UP_THRESHOLD_BLOCKS
        {
            return Ok(false);
        }

        // We get all the existing speed up transaction for tx_id. Then we figure out if we should speed it up again.
        let speedup_tx = self.store.get_last_speedup()?;

        // In case there are an existing speed up we have to check if a new speed up is needed.
        // Otherwise we always speed up the transaction
        if let Some(speed_up_tx) = speedup_tx {
            //Last speed up transaction should be the last created.
            let was_previously_speedup = speed_up_tx
                .child_tx_ids
                .iter()
                .any(|tx_id| tx_id == &tx_to_speedup.tx_id);

            if was_previously_speedup {
                return Ok(false);
            }

            if current_block_height - speed_up_tx.deliver_block_height < SPEED_UP_THRESHOLD_BLOCKS {
                return Ok(false);
            }
        }

        if tx_to_speedup.speedup_utxo.is_none() {
            return Ok(false);
        }

        // If we get here, we should speed up the transaction
        Ok(true)
    }
}

impl<M, B, C> BitcoinCoordinatorApi for BitcoinCoordinator<M, B, C>
where
    M: MonitorApi,
    B: BitcoinCoordinatorStoreApi,
    C: BitcoinClientApi,
{
    fn tick(&self) -> Result<(), BitcoinCoordinatorError> {
        self.monitor.tick()?;
        // The monitor is considered ready when it has fully indexed the blockchain and is up to date with the latest block.
        // Note that if there is a significant gap in the indexing process, it may take multiple ticks for the monitor to become ready.

        if !(self.monitor.is_ready()?) {
            return Ok(());
        }

        self.process_pending_txs()?;
        self.process_monitor_news()?;
        self.process_in_progress_txs()?;

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
        let result = self.monitor.is_ready()?;
        Ok(result)
    }

    fn dispatch(
        &self,
        tx: Transaction,
        speedup: Option<Utxo>,
        context: String,
        block_height: Option<BlockHeight>,
    ) -> Result<(), BitcoinCoordinatorError> {
        let to_monitor = TypesToMonitor::Transactions(vec![tx.compute_txid()], context.clone());
        self.monitor.monitor(to_monitor)?;

        // Save the transaction to be dispatched.
        self.store
            .save_tx(tx.clone(), speedup, block_height, context)?;

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
                    !context_data.contains(Self::SPEED_UP_CONTEXT)
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

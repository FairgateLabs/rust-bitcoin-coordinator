use crate::{
    errors::BitcoinCoordinatorError,
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::{
        AckNews, BitcoinCoordinatorType, CoordinatedTransaction, CoordinatorNews, News, SpeedUpTx,
        TransactionDispatchState,
    },
};
use bitcoin::{Address, Amount, Network, Transaction, Txid};
use bitvmx_bitcoin_rpc::{bitcoin_client::BitcoinClient, rpc_config::RpcConfig};
use bitvmx_bitcoin_rpc::{bitcoin_client::BitcoinClientApi, types::BlockHeight};
use bitvmx_transaction_monitor::{
    errors::MonitorError,
    monitor::{Monitor, MonitorApi},
    types::{AckMonitorNews, MonitorNews, TransactionStatus, TypesToMonitor},
};
use console::style;
use key_manager::key_manager::KeyManager;
use protocol_builder::{builder::ProtocolBuilder, types::Utxo};
use std::{rc::Rc, str::FromStr};
use storage_backend::storage::Storage;
use tracing::{info, warn};

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
    const SPEED_UP_CHILD_TXID_PREFIX: &str = "speed_up_child_txid";

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

        info!(
            "transactions pending to be dispatch #{}",
            style(pending_txs.len()).yellow()
        );

        for pending_tx in pending_txs {
            if !self.should_be_dispatched(&pending_tx)? {
                continue;
            }

            let tx_id = pending_tx.tx.compute_txid();

            info!(
                "{} Dispatching transaction ID: {}",
                style("Coordinator").green(),
                style(tx_id).blue(),
            );

            let dispatch_result = self.client.send_transaction(&pending_tx.tx);

            if let Err(error) = dispatch_result {
                let news = CoordinatorNews::DispatchTransactionError(
                    tx_id,
                    pending_tx.context,
                    error.to_string(),
                );
                self.store.add_news(news)?;
            }

            // Error here should be handled. And saved the error in news if needed.

            let deliver_block_height = self.monitor.get_monitor_height()?;

            self.store
                .update_tx_to_dispatched(tx_id, deliver_block_height)?;
        }

        Ok(())
    }

    fn should_be_dispatched(
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
                "{} Processing tx id: {}",
                style("→").cyan(),
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
                        let should_speedup = self.should_speedup_tx(&tx)?;

                        if should_speedup {
                            txs_to_speedup.push(tx);
                        }
                    }
                }
                Err(MonitorError::TransactionNotFound(_)) => {
                    let should_speedup = self.should_speedup_tx(&tx)?;

                    if should_speedup {
                        txs_to_speedup.push(tx);
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }

        if !txs_to_speedup.is_empty() {
            self.perform_speed_up_in_batch(txs_to_speedup)?;
        }

        Ok(())
    }

    fn process_monitor_news(&self) -> Result<(), BitcoinCoordinatorError> {
        let list_news = self.monitor.get_news()?;

        for news in list_news {
            if let MonitorNews::Transaction(tx_id, tx_status, context_data) = news {
                // Check if context_data contains the string "speed_up"

                if !context_data.starts_with(Self::SPEED_UP_CHILD_TXID_PREFIX) {
                    // Skip processing this news as it is not a speed-up transaction
                    // TODO:
                    // Since there could be reorganizations larger than 6 blocks, we set 100 blocks,
                    // we could check if a transaction was at some point meant to be sent,
                    // and it became orphaned, in that case we should try to speed it up and move the state to BroadcastPendingConfirmation.
                    // This is because at some point we move it to finalized status, which means we stop monitoring it.
                    continue;
                }

                let speed_up_data = self.store.get_speedup_tx(&tx_id)?;

                info!(
                    "Transaction Speed-up with ids: {} and {}",
                    style(tx_id).red(),
                    style(format!("{:?}", speed_up_data.child_tx_ids)).red()
                );

                self.process_speedup(&tx_status)?;
                let ack = AckMonitorNews::Transaction(speed_up_data.tx_id);
                self.monitor.ack_news(ack)?;
            }
        }

        Ok(())
    }

    fn perform_speed_up_in_batch(
        &self,
        txs_to_speedup: Vec<CoordinatedTransaction>,
    ) -> Result<(), BitcoinCoordinatorError> {
        let mut funding_tx_utxo = match self.store.get_funding()? {
            Some(utxo) => utxo,
            // No funding transaction found, we can not speed up transactions.
            None => return Ok(()),
        };

        let txids: Vec<Txid> = txs_to_speedup.iter().map(|tx| tx.tx_id).collect();
        let utxos: Vec<Utxo> = txs_to_speedup
            .iter()
            .filter_map(|tx_data| tx_data.speedup_utxo.clone())
            .collect();

        // TODO: This logic may need to be updated to use OutputType from the protocol builder for greater flexibility.
        // Currently, we derive the change address as a P2PKH address from the funding UTXO's public key.
        let change_address = Address::p2pkh(funding_tx_utxo.pub_key, self.network);

        let speedup_fee = self.calculate_speedup_fee(txs_to_speedup, funding_tx_utxo.clone())?;

        // We should not get any error from protocol builder.
        let speedup_tx = (ProtocolBuilder {}).speedup_transactions(
            &utxos,
            funding_tx_utxo.clone(),
            change_address,
            speedup_fee,
            &self.key_manager,
        )?;

        let speedup_tx_id = speedup_tx.compute_txid();

        self.dispatch(
            speedup_tx,
            None,
            Self::SPEED_UP_CHILD_TXID_PREFIX.to_string(),
            None,
        )?;

        let deliver_block_height = self.monitor.get_monitor_height()?;

        funding_tx_utxo.txid = speedup_tx_id;

        let speed_up_tx =
            SpeedUpTx::new(speedup_tx_id, deliver_block_height, txids, funding_tx_utxo);

        self.store.save_speedup_tx(&speed_up_tx)?;

        Ok(())
    }

    fn calculate_speedup_fee(
        &self,
        _txs: Vec<CoordinatedTransaction>,
        _funding_tx_utxo: Utxo,
    ) -> Result<u64, BitcoinCoordinatorError> {
        // let tx_out = self
        //     .client
        //     .get_tx_out(&funding_tx_utxo.txid, funding_tx_utxo.vout)?;

        // let _funding_amount = tx_out.value; // TODO: This should be the amount of the funding transaction.
        // TODO define fee bumping strategy
        // let porcentage_increase = 1.1;
        // let fee_rate = self.client.estimate_smart_fee()?;

        // let _target_feerate =
        //     Amount::from_sat((fee_rate.to_sat() as f64 * porcentage_increase) as u64);

        // TODO: This should be the size of the transaction.
        // total_size = size_parents + size_child
        // fee_child = total_size * target_feerate - fee_parents
        // change_output = funding_amount - fee_child

        //TODO: In case funds are not sufficient we should send a news.
        // if let Err(error) = speedup_tx {
        //     match error {
        //         ProtocolBuilderError::InsufficientFunds(amount, fee) => {
        //             let news = CoordinatorNews::InsufficientFunds(funding_tx_utxo.txid);

        //             self.store.add_news(news)?;

        //             return Ok(());
        //         }
        //         e => {
        //             let news = CoordinatorNews::DispatchSpeedUpError(
        //                 txs_to_speedup.iter().map(|tx| tx.tx_id).collect(),
        //                 txs_to_speedup.iter().map(|tx| tx.context).collect(),
        //                 e.to_string(),
        //             );

        //             self.store.add_news(news)?;

        //             return Ok(());
        //         }
        //     }
        // }

        //For now is hardcoded
        let change_output = 10000;
        Ok(change_output)
    }

    fn process_speedup(
        &self,
        tx_status: &TransactionStatus,
    ) -> Result<(), BitcoinCoordinatorError> {
        // This indicates that this is a speed-up transaction that has been mined with 1 confirmation,
        // which means it should be treated as the new funding transaction.
        if tx_status.is_confirmed() {
            if tx_status.confirmations == 1 {
                let mut funding_tx_utxo = self.store.get_funding()?.ok_or(
                    BitcoinCoordinatorError::BitcoinCoordinatorError(
                        "No funding transaction found".to_string(),
                    ),
                )?;

                // The transaction has received its first confirmation, indicating it is now included in a block.
                // At this point, the speed-up transaction becomes the new funding transaction for future operations.
                funding_tx_utxo.txid = tx_status.tx_id;
                self.store.add_funding(funding_tx_utxo)?;
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
        const SPEED_UP_THRESHOLD_BLOCKS: u32 = 1;

        // We do not speed up the transaction if it has not been delivered yet.
        if tx_to_speedup.broadcast_block_height.is_none() {
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
        // The monitor is considered ready when it has fully indexed the blockchain and is up to date with the latest block.
        // Note that if there is a significant gap in the indexing process, it may take multiple ticks for the monitor to become ready.
        if !(self.monitor.is_ready()?) {
            self.monitor.tick()?;
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
            "{} Transaction ID {} ready to be dispatch.",
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
                    !context_data.starts_with(Self::SPEED_UP_CHILD_TXID_PREFIX)
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

use std::{rc::Rc, str::FromStr};

use crate::{
    errors::BitcoinCoordinatorError,
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::{
        AckNews, BitcoinCoordinatorType, CoordinatedTransaction, CoordinatorNews,
        FundingTransaction, News, SpeedUpTx, TransactionDispatchState,
    },
};

use bitcoin::{Network, Transaction, Txid};
use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use bitvmx_bitcoin_rpc::types::BlockHeight;
use bitvmx_transaction_monitor::{
    errors::MonitorError,
    monitor::{Monitor, MonitorApi},
    types::{AckMonitorNews, MonitorNews, TransactionStatus, TypesToMonitor},
};
use console::style;
use key_manager::{key_manager::KeyManager, keystorage::database::DatabaseKeyStore};
use storage_backend::storage::Storage;
use tracing::{info, warn};
use transaction_dispatcher::{
    dispatcher::{TransactionDispatcher, TransactionDispatcherApi},
    errors::DispatcherError,
    signer::Account,
};

pub struct BitcoinCoordinator<M, D, B>
where
    M: MonitorApi,
    D: TransactionDispatcherApi,
    B: BitcoinCoordinatorStoreApi,
{
    monitor: M,
    dispatcher: D,
    store: B,
    account: Account,
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
    /// * `context` - Additional context information for the transaction to be returned in news
    /// * `block_height` - Block height to dispatch the transaction (None means now)
    fn dispatch(
        &self,
        tx: Transaction,
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
    /// This allows the coordinator to create RBF (Replace-By-Fee) transactions when needed
    ///
    /// # Arguments
    /// * `txs` - List of transaction IDs that may need speed-up
    /// * `funding_tx` - Funding transaction information to use for speed-ups
    /// * `context` - Additional context information to be returned in news
    fn fund_for_speedup(
        &self,
        txs: Vec<Txid>,
        funding_tx: FundingTransaction,
        context: String,
    ) -> Result<(), BitcoinCoordinatorError>;

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
        key_manager: Rc<KeyManager<DatabaseKeyStore>>,
        checkpoint: Option<BlockHeight>,
        confirmation_threshold: u32,
        network: Network,
    ) -> Result<Self, BitcoinCoordinatorError> {
        // We should pass node_rpc_url and that is all. Client should be removed.
        // The only one that connects with the blockchain is the dispatcher and the indexer.
        // So here should be initialized the BitcoinClient
        let monitor = Monitor::new_with_paths(
            rpc_config,
            storage.clone(),
            checkpoint,
            confirmation_threshold,
        )?;

        let store = BitcoinCoordinatorStore::new(storage)?;
        let account = Account::new(network);
        let dispatcher = TransactionDispatcher::new_with_path(rpc_config, key_manager)?;
        let coordinator = BitcoinCoordinator::new(monitor, store, dispatcher, account);

        Ok(coordinator)
    }
}

impl<M, D, B> BitcoinCoordinator<M, D, B>
where
    M: MonitorApi,
    D: TransactionDispatcherApi,
    B: BitcoinCoordinatorStoreApi,
{
    const SPEED_UP_CHILD_TXID_PREFIX: &str = "speed_up_child_txid:";

    // Stop monitoring a transaction after 100 confirmations.
    // In case of a reorganization bigger than 100 blocks, we have to do a rework in the coordinator.
    const MAX_MONITORING_CONFIRMATIONS: u32 = 100;

    pub fn new(monitor: M, store: B, dispatcher: D, account: Account) -> Self {
        Self {
            monitor,
            dispatcher,
            store,
            account: account.clone(),
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

            let dispatch_result = self.dispatcher.send(pending_tx.tx);

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
                        self.process_unseen_transaction(tx)?;
                        return Ok(());
                    }
                }
                Err(MonitorError::TransactionNotFound(_)) => {
                    self.process_unseen_transaction(tx)?;
                }
                Err(e) => return Err(e.into()),
            }
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

                // Remove the "speed_up_txid:" prefix from context_data
                let tx_id_child = context_data.replace(Self::SPEED_UP_CHILD_TXID_PREFIX, "");
                let tx_child_id = match Txid::from_str(&tx_id_child) {
                    Ok(txid) => txid,
                    Err(e) => {
                        return Err(BitcoinCoordinatorError::BitcoinCoordinatorError(format!(
                            "Failed to parse speed up transaction child id: {}",
                            e
                        )))
                    }
                };

                info!(
                    "Transaction Speed-up with id: {} for child {}",
                    style(tx_id).red(),
                    style(tx_child_id).red()
                );

                self.process_speed_up(&tx_status, tx_child_id)?;
                let ack = AckMonitorNews::Transaction(tx_id);
                self.monitor.ack_news(ack)?;
            }
        }

        Ok(())
    }

    fn speed_up(
        &self,
        tx_to_speedup: CoordinatedTransaction,
        funding_tx: FundingTransaction,
        funding_context: String,
    ) -> Result<(), BitcoinCoordinatorError> {
        let dispatch_result = self.dispatcher.speed_up(
            &tx_to_speedup.tx,
            self.account.pk,
            funding_tx.tx_id,
            (
                funding_tx.utxo_index,
                funding_tx.utxo_output.clone(),
                self.account.pk,
            ),
        );

        if let Err(error) = dispatch_result {
            match error {
                DispatcherError::InsufficientFunds => {
                    let news = CoordinatorNews::InsufficientFunds(
                        tx_to_speedup.tx_id,
                        tx_to_speedup.context,
                        funding_tx.tx_id,
                        funding_context,
                    );

                    self.store.add_news(news)?;

                    return Ok(());
                }
                e => {
                    let news = CoordinatorNews::DispatchSpeedUpError(
                        tx_to_speedup.tx_id,
                        tx_to_speedup.context,
                        e.to_string(),
                    );

                    self.store.add_news(news)?;

                    return Ok(());
                }
            }
        }

        if dispatch_result.is_ok() {
            let (speed_up_tx_id, deliver_fee_rate) = dispatch_result.unwrap();
            let deliver_block_height = self.monitor.get_monitor_height()?;

            let speed_up_tx = SpeedUpTx::new(
                speed_up_tx_id,
                deliver_block_height,
                deliver_fee_rate,
                tx_to_speedup.tx_id,
                funding_tx.utxo_index,
                funding_tx.utxo_output,
            );

            self.store.save_speedup_tx(&speed_up_tx)?;

            let context_data = format!(
                "{}{}",
                Self::SPEED_UP_CHILD_TXID_PREFIX,
                tx_to_speedup.tx_id
            );

            let monitor_data = TypesToMonitor::Transactions(vec![speed_up_tx_id], context_data);

            self.monitor.monitor(monitor_data)?;
        }

        Ok(())
    }

    fn process_speed_up(
        &self,
        tx_status: &TransactionStatus,
        child_txid: Txid,
    ) -> Result<(), BitcoinCoordinatorError> {
        // This indicates that this is a speed-up transaction that has been mined with 1 confirmation,
        // which means it should be treated as the new funding transaction.
        if tx_status.is_confirmed() {
            if tx_status.confirmations == 1 {
                let speed_up_tx = self.store.get_speedup_tx(&child_txid, &tx_status.tx_id)?;
                //Confirmation in 1 means the transaction is already included in the block.
                //The new transaction funding is gonna be this a speed-up transaction.
                let funding_info = FundingTransaction {
                    tx_id: speed_up_tx.tx_id,
                    utxo_index: speed_up_tx.utxo_index,
                    utxo_output: speed_up_tx.utxo_output.clone(),
                };

                self.store.update_funding(child_txid, funding_info)?;
            }

            if tx_status.is_orphan() {
                //Speed up previouly was mined, now is orphan then, we have to remove it as a funding tx.
                self.store.remove_funding(tx_status.tx_id, child_txid)?;
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

    fn process_unseen_transaction(
        &self,
        tx_data_to_speedup: CoordinatedTransaction,
    ) -> Result<(), BitcoinCoordinatorError> {
        const SPEED_UP_THRESHOLD_BLOCKS: u32 = 1;

        // We do not speed up the transaction if it has not been delivered yet.
        if tx_data_to_speedup.broadcast_block_height.is_none() {
            return Ok(());
        }

        let current_block_height = self.monitor.get_monitor_height()?;

        if current_block_height - tx_data_to_speedup.broadcast_block_height.unwrap()
            < SPEED_UP_THRESHOLD_BLOCKS
        {
            return Ok(());
        }

        // We get all the existing speed up transaction for tx_id. Then we figure out if we should speed it up again.
        let speed_up_txs = self.store.get_last_speedup_tx(&tx_data_to_speedup.tx_id)?;

        // In case there are an existing speed up we have to check if a new speed up is needed.
        // Otherwise we always speed up the transaction
        if let Some(speed_up_tx) = speed_up_txs {
            //Last speed up transaction should be the last created.
            let prev_fee_rate = speed_up_tx.deliver_fee_rate;

            // Check if the transaction should be speed up
            if !self.dispatcher.should_speed_up(prev_fee_rate)? {
                return Ok(());
            }
        };

        // Prepare to speed up the transaction using its associated funding transaction.
        // This will create a new transaction with a higher fee rate to replace the original one.
        let funding_tx = self.store.get_funding(tx_data_to_speedup.tx_id)?;

        if funding_tx.is_none() {
            //In case there is no funding transaction, we can't speed up the transaction.
            return Ok(());
        }

        let funding_data = funding_tx.unwrap();

        self.speed_up(tx_data_to_speedup, funding_data.0, funding_data.1)?;

        Ok(())
    }
}

impl<M, D, B> BitcoinCoordinatorApi for BitcoinCoordinator<M, D, B>
where
    M: MonitorApi,
    D: TransactionDispatcherApi,
    B: BitcoinCoordinatorStoreApi,
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
        context: String,
        block_height: Option<BlockHeight>,
    ) -> Result<(), BitcoinCoordinatorError> {
        let to_monitor = TypesToMonitor::Transactions(vec![tx.compute_txid()], context.clone());
        self.monitor.monitor(to_monitor)?;

        // Save the transaction to be dispatched.
        self.store.save_tx(tx.clone(), block_height, context)?;

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

    fn fund_for_speedup(
        &self,
        tx_ids: Vec<Txid>,
        funding_tx: FundingTransaction,
        context: String,
    ) -> Result<(), BitcoinCoordinatorError> {
        // Check if there are any transactions to process
        if tx_ids.is_empty() {
            return Err(BitcoinCoordinatorError::BitcoinCoordinatorError(
                "No transactions provided for funding".to_string(),
            ));
        }

        self.store.add_funding(tx_ids, funding_tx, context)?;

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

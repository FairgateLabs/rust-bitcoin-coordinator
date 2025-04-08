use std::rc::Rc;

use crate::{
    errors::BitcoinCoordinatorError,
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::{
        AcknowledgeNews, BitcoinCoordinatorType, CoordinatedTransaction, FundingTransaction, News,
        SpeedUpTx, TransactionDispatch, TransactionFund, TransactionState,
    },
};

use bitcoin::{Network, PublicKey, Transaction, TxOut, Txid};
use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use bitvmx_bitcoin_rpc::types::BlockHeight;
use bitvmx_transaction_monitor::{
    errors::MonitorError,
    monitor::{Monitor, MonitorApi},
    types::{
        AcknowledgeTransactionNews, Id, TransactionMonitor, TransactionNews, TransactionStatus,
    },
};
use console::style;
use key_manager::{key_manager::KeyManager, keystorage::database::DatabaseKeyStore};
use storage_backend::storage::Storage;
use tracing::info;
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
    fn is_ready(&self) -> Result<bool, BitcoinCoordinatorError>;

    fn tick(&self) -> Result<(), BitcoinCoordinatorError>;

    fn monitor(&self, tx_data: TransactionMonitor) -> Result<(), BitcoinCoordinatorError>;

    fn dispatch(&self, tx_data: TransactionDispatch) -> Result<(), BitcoinCoordinatorError>;

    fn fund_for_speedup(&self, tx_data: TransactionFund) -> Result<(), BitcoinCoordinatorError>;

    fn get_news(&self) -> Result<News, BitcoinCoordinatorError>;

    fn acknowledge_news(&self, news: AcknowledgeNews) -> Result<(), BitcoinCoordinatorError>;
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
    pub fn new(monitor: M, store: B, dispatcher: D, account: Account) -> Self {
        Self {
            monitor,
            dispatcher,
            store,
            account: account.clone(),
        }
    }

    fn process_pending_txs(&self) -> Result<(), BitcoinCoordinatorError> {
        // Get pending instance transactions to be send to the blockchain
        let pending_txs = self.store.get_txs_by_state(TransactionState::ReadyToSend)?;

        info!(
            "transactions pending to be sent #{}",
            style(pending_txs.len()).yellow()
        );

        for pending_tx in pending_txs {
            let tx_id = pending_tx.tx.compute_txid();

            info!(
                "{} Dispatching transaction ID: {} {}",
                style("Orchastrator").green(),
                style(tx_id).blue(),
                match pending_tx.group_id {
                    Some(id) => format!("for Group ID: {}", style(id).green()),
                    None => String::new(),
                }
            );

            self.dispatcher.send(pending_tx.tx)?;

            self.store.update_tx_state(tx_id, TransactionState::Sent)?;
        }

        Ok(())
    }

    fn speed_up(
        &self,
        group_id: Id,
        tx: &Transaction,
        funding_txid: Txid,
        tx_public_key: PublicKey,
        funding_utxo: (u32, TxOut, PublicKey),
    ) -> Result<(), BitcoinCoordinatorError> {
        let dispatch_result =
            self.dispatcher
                .speed_up(tx, tx_public_key, funding_txid, funding_utxo.clone());

        if let Err(error) = dispatch_result {
            match error {
                DispatcherError::InsufficientFunds => {
                    self.store.add_funding_request(group_id)?;
                    return Ok(());
                }
                e => return Err(e.into()),
            }
        }

        if dispatch_result.is_ok() {
            let (speed_up_tx_id, deliver_fee_rate) = dispatch_result.unwrap();
            let deliver_block_height = self.monitor.get_monitor_height()?;

            let speed_up_tx = SpeedUpTx::new(
                speed_up_tx_id,
                deliver_block_height,
                deliver_fee_rate,
                tx.compute_txid(),
                funding_utxo.0,
                funding_utxo.1,
            );

            self.store
                .add_speed_up_tx(group_id.to_string(), &speed_up_tx)?;

            let monitor_data = TransactionMonitor::GroupTransaction(group_id, vec![speed_up_tx_id]);

            self.monitor.monitor(monitor_data)?;
        }

        Ok(())
    }

    fn process_in_progress_txs(&self) -> Result<(), BitcoinCoordinatorError> {
        //TODO: THIS COULD BE IMPROVED.
        // If transaction still in sent means it should be speed up, and is not confirmed.
        // otherwise it should be moved as confirmed in the previous validations for news.
        let txs = self.store.get_txs_by_state(TransactionState::Sent)?;

        for tx in txs {
            info!(
                "{} Processing tx id: {} {}",
                style("→").cyan(),
                style(tx.tx_id).blue(),
                match tx.group_id {
                    Some(id) => format!("for Group ID: {}", style(id).green()),
                    None => String::new(),
                }
            );

            // Get updated transaction status from monitor
            let tx_status = self.monitor.get_tx_status(&tx.tx_id);

            match tx_status {
                Ok(tx_status) => self.process_seen_transaction(tx.group_id, &tx_status)?,
                Err(MonitorError::TransactionNotFound(_)) => {
                    // TODO: Check if we pass a enum instead of to params
                    self.process_unseen_transaction(tx.group_id, &tx)?
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }

    fn process_speedup_news(&self) -> Result<(), BitcoinCoordinatorError> {
        let list_news = self.monitor.get_news()?;

        for news in list_news {
            match news {
                TransactionNews::GroupTransaction(group_id, txs_status) => {
                    // Only the group transaction should be speed up.
                    let is_speed_up = self
                        .store
                        .is_speed_up_tx(group_id.to_string(), &txs_status.tx_id)?;

                    if is_speed_up {
                        info!(
                            "{} Speed-up transaction for Group ID: {} has news in tx: {}",
                            style("News").green(),
                            style(group_id).green(),
                            style(txs_status.tx_id).red()
                        );

                        self.process_speed_up_change(group_id, &txs_status)?;

                        // Acknowledge the transaction news for a speed up transaction.
                        let ack = AcknowledgeTransactionNews::GroupTransaction(
                            group_id,
                            txs_status.tx_id,
                        );
                        self.monitor.acknowledge_news(ack)?;
                    }
                }
                TransactionNews::SingleTransaction(tx_status) => {
                    let is_speed_up = self
                        .store
                        .is_speed_up_tx(tx_status.tx_id.to_string(), &tx_status.tx_id)?;

                    if is_speed_up {
                        info!(
                            "{} Single Transaction ID: {}",
                            style("News").green(),
                            style(tx_status.tx_id).red()
                        );

                        self.process_speed_up_change(tx_status.tx_id.to_string(), tx_status)?;

                        // Acknowledge the transaction news for a speed up transaction.
                        let ack = AcknowledgeTransactionNews::GroupTransaction(
                            group_id,
                            txs_status.tx_id,
                        );
                        self.monitor.acknowledge_news(ack)?;
                    }
                    // TODO: In the future, we should implement the ability to speed up single transactions
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn process_speed_up_change(
        &self,
        id: String,
        tx_status: &TransactionStatus,
    ) -> Result<(), BitcoinCoordinatorError> {
        // This indicates that this is a speed-up transaction that has been mined with 1 confirmation,
        // which means it should be treated as the new funding transaction.
        if tx_status.is_confirmed() {
            if tx_status.confirmations == 1 {
                self.handle_confirmation_speed_up_transaction(id, &tx_status.tx_id)?;
            }

            if tx_status.is_orphan() {
                self.handle_orphan_speed_up_transaction(id, &tx_status.tx_id)?;
            }
        }

        if !tx_status.is_confirmed() {
            // If a speed-up transaction has not been seen (it has not been mined), no action is required.
            // The responsibility for creating a new speed-up transaction lies with the instance transaction that is delivered.
        }

        // In the event of a reorganization, we would need to do the opposite.
        // This involves removing the speed-up transaction and potentially replacing it with another transaction
        // that could take its place as the last speed-up transaction or become the new last funding transaction.

        Ok(())
    }

    fn process_seen_transaction(
        &self,
        group_id: Option<Id>,
        tx_info: &TransactionStatus,
    ) -> Result<(), BitcoinCoordinatorError> {
        if tx_info.confirmations == 1 {
            // If the transaction has only one confirmation
            // This means it has been included in a block but not yet deeply confirmed.
            self.store
                .update_tx_state(group_id, tx_info.tx_id, TransactionState::Confirmed)?;

            return Ok(());
        }

        let confirmation_threshold = self.monitor.get_confirmation_threshold();

        if tx_info.is_finalized(confirmation_threshold) {
            // Transaction was mined and has sufficient confirmations to mark it as finalized.
            // Update the transaction to completed given that transaction has more than the threshold confirmations
            self.store
                .update_tx_state(group_id, tx_info.tx_id, TransactionState::Finalized)?;

            return Ok(());
        }

        if tx_info.is_orphan() {
            self.process_unseen_transaction(group_id, tx_info)?;
            return Ok(());
        }

        Ok(())
    }

    fn handle_confirmation_speed_up_transaction(
        &self,
        id: String,
        speed_up_tx_id: &Txid,
    ) -> Result<(), BitcoinCoordinatorError> {
        let speed_up_tx = self.store.get_speed_up_tx(id, speed_up_tx_id)?.unwrap();

        //Confirmation in 1 means the transaction is already included in the block.
        //The new transaction funding is gonna be this a speed-up transaction.
        let funding_info = FundingTransaction {
            tx_id: speed_up_tx.tx_id,
            utxo_index: speed_up_tx.utxo_index,
            utxo_output: speed_up_tx.utxo_output.clone(),
        };

        //TODO: There is something missing here. We are moving a speed-up transaction to a funding transaction.
        // The inverse should also be supported.
        self.store.fund_for_speedup(id, &funding_info)?;

        Ok(())
    }

    fn handle_orphan_speed_up_transaction(
        &self,
        id: Id,
        speed_up_tx_id: &Txid,
    ) -> Result<(), BitcoinCoordinatorError> {
        //Speed up previouly was mined, now is orphan then, we have to remove it as a funding tx.
        self.store.remove_funding_tx(id, speed_up_tx_id)?;

        Ok(())
    }

    fn process_unseen_transaction(
        &self,
        group_id: Option<Id>,
        tx_data: &CoordinatedTransaction,
    ) -> Result<(), BitcoinCoordinatorError> {
        // We get all the existing speed up transaction for tx_id. Then we figure out if we should speed it up again.
        let speed_up_txs = self
            .store
            .get_speed_up_txs_for_child(group_id, &tx_data.tx_id)?;

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

        //We are gonna have a funding transaction for each group transaction.
        let funding_tx = self.store.get_funding_tx(group_id)?;

        if funding_tx.is_none() {
            //In case there is no funding transaction, we can't speed up the transaction.
            return Ok(());
        }

        let funding_tx = funding_tx.unwrap();

        self.speed_up(
            group_id,
            &tx_data.tx,
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
        self.process_speedup_news()?;
        self.process_in_progress_txs()?;

        Ok(())
    }

    fn monitor(&self, data: TransactionMonitor) -> Result<(), BitcoinCoordinatorError> {
        match data.clone() {
            TransactionMonitor::GroupTransaction(group_id, txs) => {
                if txs.is_empty() {
                    return Err(BitcoinCoordinatorError::BitcoinCoordinatorError(format!(
                        "Group transactions array is empty for group_id: {}",
                        group_id
                    )));
                }
            }
            _ => {}
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

    fn dispatch(&self, tx_data: TransactionDispatch) -> Result<(), BitcoinCoordinatorError> {
        match tx_data.clone() {
            TransactionDispatch::GroupTransaction(group_id, tx) => {
                let to_monitor =
                    TransactionMonitor::GroupTransaction(group_id, vec![tx.compute_txid()]);

                self.monitor.monitor(to_monitor)?;
                self.store.save_tx(Some(group_id), tx.clone())?;

                info!(
                    "{} Transaction ID {} for Group ID {} moved to Pending status to be sent.",
                    style("Coordinator").green(),
                    style(tx.compute_txid()).yellow(),
                    style(group_id).green()
                );
            }
            TransactionDispatch::SingleTransaction(tx) => {
                let tx_id = tx.compute_txid();
                let to_monitor = TransactionMonitor::SingleTransaction(tx_id);

                self.monitor.monitor(to_monitor)?;
                self.store.save_tx(None, tx)?; // TODO: Check if we pass a enum instead of to params

                info!(
                    "{} Transaction ID {} moved to Pending status to be sent.",
                    style("Coordinator").green(),
                    style(tx_id).yellow()
                );
            }
        }

        Ok(())
    }

    fn fund_for_speedup(&self, data: TransactionFund) -> Result<(), BitcoinCoordinatorError> {
        self.store.fund_for_speedup(&data)?;

        Ok(())
    }

    fn get_news(&self) -> Result<News, BitcoinCoordinatorError> {
        let txs = self.monitor.get_news()?;

        let funds_requests = self.store.get_funding_requests()?;

        Ok(News {
            txs,
            funds_requests,
        })
    }

    fn acknowledge_news(&self, news: AcknowledgeNews) -> Result<(), BitcoinCoordinatorError> {
        match news {
            AcknowledgeNews::Transaction(news) => self.monitor.acknowledge_news(news)?,
            AcknowledgeNews::FundingRequest(id) => self.store.acknowledge_funding_request(id)?,
        }
        Ok(())
    }
}

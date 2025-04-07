use std::rc::Rc;

use crate::{
    errors::BitcoinCoordinatorError,
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::{
        AcknowledgeNews, BitcoinCoordinatorType, FundingTransaction, News, SpeedUpTx,
        TransactionDispatch, TransactionFund, TransactionInfo, TransactionNew, TransactionState,
    },
};

use bitcoin::{Network, PublicKey, Transaction, TxOut, Txid};
use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use bitvmx_bitcoin_rpc::types::BlockHeight;
use bitvmx_transaction_monitor::{
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

    fn monitor(&self, data: TransactionMonitor) -> Result<(), BitcoinCoordinatorError>;

    fn dispatch(&self, data: TransactionDispatch) -> Result<(), BitcoinCoordinatorError>;

    fn fund_for_speedup(&self, data: TransactionFund) -> Result<(), BitcoinCoordinatorError>;

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
        let pending_txs = self.store.get_txs_info(TransactionState::ReadyToSend)?;

        info!(
            "transactions pending to be sent #{}",
            style(pending_txs.len()).yellow()
        );

        // For each pending pair
        for (instance_id, txs) in pending_txs {
            for tx_info in txs {
                info!(
                    "{} Dispatching transaction ID: {}",
                    style("Orchastrator").green(),
                    style(tx_info.tx_id).blue()
                );

                self.dispatcher.send(tx_info.tx.unwrap())?;

                self.store.update_tx_status(
                    TransactionDispatch::SingleTransaction(tx_info.tx_id),
                    TransactionState::Sent,
                )?;
            }
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

            let speed_up_tx = SpeedUpTx {
                tx_id: speed_up_tx_id,
                deliver_fee_rate,
                deliver_block_height,
                child_tx_id: tx.compute_txid(),
                utxo_index: funding_utxo.0,
                utxo_output: funding_utxo.1,
            };

            self.store.add_speed_up_tx(group_id, &speed_up_tx)?;

            let monitor_data = TransactionMonitor::GroupTransaction(group_id, vec![speed_up_tx_id]);

            self.monitor.monitor(monitor_data)?;
        }

        Ok(())
    }

    fn process_in_progress_txs(&self) -> Result<(), BitcoinCoordinatorError> {
        //TODO: THIS COULD BE IMPROVED.
        // If transaction still in sent means it should be speed up, and is not confirmed.
        // otherwise it should be moved as confirmed in the previous validations for news.
        let instance_txs = self.store.get_txs_info(TransactionState::Sent)?;

        for (instance_id, txs) in instance_txs {
            for tx_info in txs {
                info!(
                    "{} Processing transaction: {} for instance: {}",
                    style("→").cyan(),
                    style(tx_info.tx_id).blue(),
                    style(instance_id).red()
                );

                // Get the latest transaction status from monitor for this transaction
                let tx_status = self.monitor.get_tx_status(&tx_info.tx_id);

                match tx_status {
                    Ok(tx_status) => self.process_tx_change(instance_id, &tx_status)?,
                    Err(e) => return Err(e.into()),
                }
            }
        }

        Ok(())
    }

    fn process_news(&self) -> Result<(), BitcoinCoordinatorError> {
        // Get any news in each instance that are being monitored.
        // Get instances news also returns the speed ups txs added for each instance.
        let list_news = self.monitor.get_news()?;

        for news in list_news {
            match news {
                TransactionNews::GroupTransaction(group_id, txs_status) => {
                    info!(
                        "{} Group Transaction ID: {} has news in tx: {}",
                        style("News").green(),
                        style(group_id).green(),
                        style(txs_status.tx_id).red()
                    );

                    // Only the group transaction should be speed up.
                    let is_speed_up = self.store.is_speed_up_tx(group_id, &txs_status.tx_id)?;

                    if is_speed_up {
                        self.process_speed_up_change(group_id, &txs_status)?;
                    } else {
                        self.process_tx_change(group_id, &txs_status)?;
                    }

                    // Acknowledge the transaction news to the monitor to update its state.
                    // This step ensures that the monitor is aware of the transaction's completion and can update its tracking accordingly.
                    let monitor_data =
                        AcknowledgeTransactionNews::GroupTransaction(group_id, txs_status.tx_id);
                    self.monitor.acknowledge_news(monitor_data)?;
                }
                TransactionNews::SingleTransaction(tx_status) => {
                    info!(
                        "{} Single Transaction ID: {}",
                        style("News").green(),
                        style(tx_status.tx_id).red()
                    );
                }
                TransactionNews::RskPeginTransaction(tx_status) => {
                    info!(
                        "{} RSK PegIn Transaction ID: {}",
                        style("News").green(),
                        style(tx_status.tx_id).red()
                    );
                }
                TransactionNews::SpendingUTXOTransaction(utxo_index, tx_status) => {
                    info!(
                        "{} Spending UTXO Transaction ID: {} with utxo index: {}",
                        style("News").green(),
                        style(tx_status.tx_id).red(),
                        style(utxo_index).green()
                    );
                }
            }
        }

        Ok(())
    }

    fn process_speed_up_change(
        &self,
        instance_id: Id,
        tx_status: &TransactionStatus,
    ) -> Result<(), BitcoinCoordinatorError> {
        // This indicates that this is a speed-up transaction that has been mined with 1 confirmation,
        // which means it should be treated as the new funding transaction.
        if tx_status.is_confirmed() {
            if tx_status.confirmations == 1 {
                self.handle_confirmation_speed_up_transaction(instance_id, &tx_status.tx_id)?;
            }

            if tx_status.is_orphan() {
                self.handle_orphan_speed_up_transaction(instance_id, &tx_status.tx_id)?;
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

    fn process_tx_change(
        &self,
        instance_id: Id,
        tx_status: &TransactionStatus,
    ) -> Result<(), BitcoinCoordinatorError> {
        let is_confirmed = tx_status.is_confirmed();

        if is_confirmed {
            if tx_status.confirmations == 1 {
                // If the transaction has only one confirmation:
                // This means it has been included in a block but not yet deeply confirmed.
                self.handle_confirmation_transaction(instance_id, tx_status)?;
                return Ok(());
            }

            if tx_status.confirmations >= self.monitor.get_confirmation_threshold() {
                // If the transaction has sufficient confirmations, it is considered fully complete.
                // Mark the transaction as completed
                self.handle_complete_transaction(instance_id, tx_status)?;
                return Ok(());
            }
        }

        if !is_confirmed {
            // Retrieve information about the last known state of this transaction
            let in_progress_tx = self
                .store
                .get_instance_tx(instance_id, &tx_status.tx_id)?
                .unwrap();

            if in_progress_tx.is_transaction_owned() {
                if tx_status.is_orphan() {
                    // Transaction is considered "orphaned" (removed from its block due to a reorg).
                    // Update its status to indicate it is back in the mempool.
                    self.store.update_tx_status(
                        instance_id,
                        &tx_status.tx_id,
                        TransactionState::Sent,
                    )?;
                }

                // The transaction is currently considered in-progress but has not been observed on-chain.
                // Resend or accelerate the transaction to ensure it propagates properly.
                self.handle_unseen_transaction(instance_id, &in_progress_tx)?;
            } else if tx_status.is_orphan() {
                // Update the local storage to mark the transaction as orphaned.
                self.store.update_tx_status(
                    instance_id,
                    &tx_status.tx_id,
                    TransactionState::Orphan,
                )?;

                self.store.add_tx_news(instance_id, tx_status.tx_id)?;
            }
        }

        Ok(())
    }

    fn handle_confirmation_transaction(
        &self,
        instance_id: Id,
        tx_status: &TransactionStatus,
    ) -> Result<(), BitcoinCoordinatorError> {
        // Update the transaction to completed given that transaction has more than the threshold confirmations
        self.store
            .update_tx_status(instance_id, &tx_status.tx_id, TransactionState::Confirmed)?;

        self.store.add_tx_news(instance_id, tx_status.tx_id)?;

        Ok(())
    }

    fn handle_confirmation_speed_up_transaction(
        &self,
        id: Id,
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

    fn handle_complete_transaction(
        &self,
        id: Id,
        tx_status: &TransactionStatus,
    ) -> Result<(), BitcoinCoordinatorError> {
        // Transaction was mined and has sufficient confirmations to mark it as complete.

        // Update the transaction to completed given that transaction has more than the threshold confirmations
        self.store
            .update_tx_status(id, &tx_status.tx_id, TransactionState::Finalized)?;

        self.store.add_tx_news(id, tx_status.tx_id)?;

        Ok(())
    }

    fn handle_unseen_transaction(
        &self,
        id: Id,
        tx_data: &TransactionInfo,
    ) -> Result<(), BitcoinCoordinatorError> {
        // We get all the existing speed up transaction for tx_id. Then we figure out if we should speed it up again.
        let speed_up_txs = self.store.get_speed_up_txs_for_child(id, &tx_data.tx_id)?;

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
        let funding_tx = self.store.get_funding_tx(id)?;

        if funding_tx.is_none() {
            //In case there is no funding transaction, we can't speed up the transaction.
            return Ok(());
        }

        let funding_tx = funding_tx.unwrap();

        self.speed_up(
            id,
            tx_data.tx.as_ref().unwrap(),
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
        //TODO Question: Should we handle the scenario where there are more than one instance per operator running?
        // This scenario raises concerns that the protocol should be aware of a transaction that belongs to it but was not sent by itself (was seen in the blockchain)

        // The monitor is considered ready when it has fully indexed the blockchain and is up to date with the latest block.
        // Note that if there is a significant gap in the indexing process, it may take multiple ticks for the monitor to become ready.

        if !(self.monitor.is_ready()?) {
            self.monitor.tick()?;
            return Ok(());
        }

        //TODO QUESTION?: I think we could not receive a tx to be send for an instance that
        //  has a pending tx be dispatch. Otherwise we could add some warning..

        // Send pending transactions that were queued.
        self.process_pending_txs()?;

        // Handle any updates related to transactions, including new information about transactions that have not been reviewed yet.
        self.process_news()?;

        // Handle any updates related to instances, including new information about transactions that have not been reviewed yet.
        self.process_in_progress_txs()?;

        Ok(())
    }

    fn monitor(&self, data: TransactionMonitor) -> Result<(), BitcoinCoordinatorError> {
        match data.clone() {
            TransactionMonitor::GroupTransaction(id, txs) => {
                if txs.is_empty() {
                    return Err(BitcoinCoordinatorError::BitcoinCoordinatorError(
                        "Group transactions array is empty".to_string(),
                    ));
                }

                self.store.coordinate(&data)?;
                self.monitor.monitor(data)?;
            }
            TransactionMonitor::SingleTransaction(_) => {
                self.store.coordinate(&data)?;
                self.monitor.monitor(data)?;
            }
            TransactionMonitor::RskPeginTransaction => {
                self.monitor.monitor(data)?;
            }
            TransactionMonitor::SpendingUTXOTransaction(id, utxo_index) => {
                self.monitor
                    .monitor(TransactionMonitor::SpendingUTXOTransaction(id, utxo_index))?;
            }
        }

        Ok(())
    }

    fn is_ready(&self) -> Result<bool, BitcoinCoordinatorError> {
        //TODO: The coordinator is currently considered ready when the monitor is ready.
        // However, we may decide to take into consideration pending and in progress transactions in the future.
        let result = self.monitor.is_ready()?;
        Ok(result)
    }

    fn dispatch(&self, data: TransactionDispatch) -> Result<(), BitcoinCoordinatorError> {
        self.store
            .update_tx_status(data.clone(), TransactionState::ReadyToSend)?;

        match &data {
            TransactionDispatch::GroupTransaction(group_id, tx_id) => {
                info!(
                    "{} Transaction ID {} for Group ID {} moved to Pending status to be sent.",
                    style("Coordinator").green(),
                    style(tx_id).yellow(),
                    style(group_id).green()
                );
            }
            TransactionDispatch::SingleTransaction(tx_id) => {
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
        let news = self.store.get_news()?;
        let mut txs_by_id: Vec<(Id, Vec<TransactionNew>)> = Vec::new();

        for (instance_id, tx_ids) in news {
            let mut txs = Vec::new();

            for tx_id in tx_ids {
                // Transaction information is stored in both the monitor and orchastrator storage.
                // News are generated when a transaction changes state to either:
                // - Orphaned (removed from chain due to reorg)
                // - Confirmed (included in a block)
                // - Finalized (reached required confirmation threshold)
                let tx_info = self
                    .store
                    .get_tx(instance_id, &tx_id)?
                    .expect("Transaction not found in instance");

                let tx_status = self.monitor.get_tx_status(&tx_id);

                let tx_status = match tx_status {
                    Ok(tx_status) => tx_status,
                    Err(e) => return Err(e.into()),
                };

                txs.push(TransactionNew {
                    tx_id,
                    tx: tx_status.tx.unwrap(),
                    block_info: tx_status.block_info.unwrap(),
                    confirmations: tx_status.confirmations,
                    status: tx_status.status,
                });
            }

            txs_by_id.push((instance_id, txs));
        }

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

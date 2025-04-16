use std::{rc::Rc, str::FromStr};

use crate::{
    errors::BitcoinCoordinatorError,
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::{
        AckNews, BitcoinCoordinatorType, CoordinatedTransaction, FundingTransaction, News,
        SpeedUpTx, TransactionState,
    },
};

use bitcoin::{Network, PublicKey, Transaction, TxOut, Txid};
use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use bitvmx_bitcoin_rpc::types::BlockHeight;
use bitvmx_transaction_monitor::{
    errors::MonitorError,
    monitor::{Monitor, MonitorApi},
    types::{AckTransactionNews, TransactionMonitor, TransactionNews, TransactionStatus},
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

    fn dispatch(&self, tx: Transaction, context: String) -> Result<(), BitcoinCoordinatorError>;

    fn fund_for_speedup(
        &self,
        txs: Vec<Txid>,
        funding_tx: FundingTransaction,
        context: String,
    ) -> Result<(), BitcoinCoordinatorError>;

    fn get_transaction(&self, txid: Txid) -> Result<TransactionStatus, BitcoinCoordinatorError>;

    fn get_news(&self) -> Result<News, BitcoinCoordinatorError>;

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
        let pending_txs = self.store.get_tx(TransactionState::ReadyToSend)?;

        info!(
            "transactions pending to be sent #{}",
            style(pending_txs.len()).yellow()
        );

        for pending_tx in pending_txs {
            let tx_id = pending_tx.tx.compute_txid();

            info!(
                "{} Dispatching transaction ID: {}",
                style("Coordinator").green(),
                style(tx_id).blue(),
            );

            self.dispatcher.send(pending_tx.tx)?;

            self.store.update_tx(tx_id, TransactionState::Sent)?;
        }

        Ok(())
    }

    fn process_in_progress_txs(&self) -> Result<(), BitcoinCoordinatorError> {
        //TODO: THIS COULD BE IMPROVED.
        // If transaction still in sent means it should be speed up, and is not confirmed.
        // otherwise it should be moved as confirmed in the previous validations for news.
        let txs = self.store.get_tx(TransactionState::Sent)?;

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
                    if tx_status.confirmations == 1 {
                        // If the transaction has only one confirmation
                        // This means it has been included in a block but not yet deeply confirmed.
                        info!(
                            "{} Transaction {} confirmed with 1 confirmation", 
                            style("Coordinator").green(),
                            style(tx_status.tx_id).blue()
                        );
                        
                        self.store
                            .update_tx(tx_status.tx_id, TransactionState::Confirmed)?;

                        return Ok(());
                    }

                    let confirmation_threshold = self.monitor.get_confirmation_threshold();

                    if tx_status.is_finalized(confirmation_threshold) {
                        // Transaction was mined and has sufficient confirmations to mark it as finalized.
                        // Update the transaction to completed given that transaction has more than the threshold confirmations
                        info!(
                            "{} Transaction {} finalized with {} confirmations",
                            style("Coordinator").green(),
                            style(tx_status.tx_id).blue(),
                            style(tx_status.confirmations).yellow()
                        );

                        self.store
                            .update_tx(tx_status.tx_id, TransactionState::Finalized)?;

                        return Ok(());
                    }

                    if tx_status.is_orphan() {
                        info!(
                            "{} Transaction {} is orphaned, reprocessing",
                            style("Coordinator").green(),
                            style(tx_status.tx_id).red()
                        );
                        self.process_unseen_transaction(&tx)?;
                        return Ok(());
                    }
                }
                Err(MonitorError::TransactionNotFound(_)) => {
                    info!(
                        "{} Transaction {} not found, reprocessing",
                        style("Coordinator").green(),
                        style(tx.tx_id).red()
                    );
                    self.process_unseen_transaction(&tx)?
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }

    fn process_speedup_news(&self) -> Result<(), BitcoinCoordinatorError> {
        let list_news = self.monitor.get_news()?;

        for news in list_news {
            if let TransactionNews::Transaction(tx_id, tx_status, tx_id_data) = news {
                info!(
                    "Transaction Speed-up with id: {} for child {}",
                    style(tx_id).red(),
                    style(tx_id_data.clone()).red()
                );

                let tx_child_id = match Txid::from_str(&tx_id_data) {
                    Ok(txid) => txid,
                    Err(e) => {
                        return Err(BitcoinCoordinatorError::BitcoinCoordinatorError(format!(
                            "Failed to parse transaction ID: {}",
                            e
                        )))
                    }
                };

                self.process_speed_up(&tx_status, tx_child_id)?;
                let ack = AckTransactionNews::Transaction(tx_id);
                self.monitor.ack_news(ack)?;
            }
        }

        Ok(())
    }

    fn speed_up(
        &self,
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
                    self.store.add_insufficient_funds_news(tx.compute_txid())?;
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

            self.store.save_speedup_tx(&speed_up_tx)?;

            let monitor_data = TransactionMonitor::Transactions(
                vec![speed_up_tx_id],
                tx.compute_txid().to_string(), // child txid
            );

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
                let speed_up_tx = self
                    .store
                    .get_speedup_tx(&child_txid, &tx_status.tx_id)?
                    .unwrap();

                //Confirmation in 1 means the transaction is already included in the block.
                //The new transaction funding is gonna be this a speed-up transaction.
                let funding_info = FundingTransaction {
                    tx_id: speed_up_tx.tx_id,
                    utxo_index: speed_up_tx.utxo_index,
                    utxo_output: speed_up_tx.utxo_output.clone(),
                };

                //TODO: There is something missing here. We are moving a speed-up transaction to a funding transaction.
                // The inverse should also be supported.

                self.store.update_funding(child_txid, funding_info)?;
            }

            if tx_status.is_orphan() {
                //Speed up previouly was mined, now is orphan then, we have to remove it as a funding tx.
                self.store.remove_funding(tx_status.tx_id, child_txid)?;
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

    fn process_unseen_transaction(
        &self,
        tx_data: &CoordinatedTransaction,
    ) -> Result<(), BitcoinCoordinatorError> {
        // We get all the existing speed up transaction for tx_id. Then we figure out if we should speed it up again.
        let speed_up_txs = self.store.get_last_speedup_tx(&tx_data.tx_id)?;

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
        let funding_tx = self.store.get_funding(tx_data.tx_id)?;

        if funding_tx.is_none() {
            //In case there is no funding transaction, we can't speed up the transaction.
            return Ok(());
        }

        let funding_tx = funding_tx.unwrap();

        self.speed_up(
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
        if let TransactionMonitor::Transactions(txs, _) = data.clone() {
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

    fn dispatch(&self, tx: Transaction, context: String) -> Result<(), BitcoinCoordinatorError> {
        // First we monitor the transaction if does not exist.
        let to_monitor = TransactionMonitor::Transactions(vec![tx.compute_txid()], context);
        self.monitor.monitor(to_monitor)?;

        // Save the transaction to be dispatched.
        self.store.save_tx(tx.clone())?;

        info!(
            "{} Transaction ID {} ready to be sent.",
            style("Coordinator").green(),
            style(tx.compute_txid()).yellow()
        );

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
        let txs = self.monitor.get_news()?;

        let insufficient_funds = self.store.get_insufficient_funds_news()?;

        Ok(News::new(txs, insufficient_funds))
    }

    fn ack_news(&self, news: AckNews) -> Result<(), BitcoinCoordinatorError> {
        match news {
            AckNews::Transaction(news) => self.monitor.ack_news(news)?,
            AckNews::InsufficientFunds(tx_id) => self.store.ack_insufficient_funds_news(tx_id)?,
        }
        Ok(())
    }
}

use std::rc::Rc;

use crate::{
    errors::OrchestratorError,
    storage::{OrchestratorStore, OrchestratorStoreApi},
    types::{
        AddressNew, BitvmxInstance, FundingTx, InstanceId, News, OrchestratorType, ProcessedNews,
        SpeedUpTx, TransactionBlockchainStatus, TransactionInfo, TransactionNew,
        TransactionPartialInfo, TransactionState,
    },
};

use bitcoin::{Address, Network, PublicKey, Transaction, TxOut, Txid};
use bitvmx_bitcoin_rpc::types::BlockHeight;
use bitvmx_transaction_monitor::{
    monitor::{Monitor, MonitorApi},
    types::{InstanceData, TransactionStatus},
};
use console::style;
use key_manager::{key_manager::KeyManager, keystorage::database::DatabaseKeyStore};
use log::info;
use storage_backend::storage::Storage;
use transaction_dispatcher::{
    dispatcher::{TransactionDispatcher, TransactionDispatcherApi},
    errors::DispatcherError,
    signer::Account,
};

pub struct Orchestrator<M, D, B>
where
    M: MonitorApi,
    D: TransactionDispatcherApi,
    B: OrchestratorStoreApi,
{
    monitor: M,
    dispatcher: D,
    store: B,
    current_height: BlockHeight,
    account: Account,
}

pub trait OrchestratorApi {
    fn monitor_instance(
        &self,
        instance: &BitvmxInstance<TransactionPartialInfo>,
    ) -> Result<(), OrchestratorError>;

    // Add a non-existent transaction for an existing instance.
    // This will be use in the final step.
    fn add_tx_to_instance(
        &self,
        instance_id: InstanceId,
        tx_id: &Txid,
    ) -> Result<(), OrchestratorError>;

    // The protocol requires delivering an existing transaction for an instance.
    // This is achieved by passing the full transaction.
    fn send_tx_instance(
        &self,
        instance_id: InstanceId,
        tx: &Transaction,
    ) -> Result<(), OrchestratorError>;

    fn is_ready(&mut self) -> Result<bool, OrchestratorError>;

    fn tick(&mut self) -> Result<(), OrchestratorError>;

    fn add_funding_tx(
        &self,
        instance_id: InstanceId,
        funding_tx: &FundingTx,
    ) -> Result<(), OrchestratorError>;

    fn monitor_address(&self, address: Address) -> Result<(), OrchestratorError>;

    fn get_news(&self) -> Result<News, OrchestratorError>;

    fn acknowledge_news(&self, processed_news: ProcessedNews) -> Result<(), OrchestratorError>;
}

impl OrchestratorType {
    //#[warn(clippy::too_many_arguments)]
    pub fn new_with_paths(
        rpc_url: &str,
        rpc_user: &str,
        rpc_pass: &str,
        storage: Rc<Storage>,
        key_manager: Rc<KeyManager<DatabaseKeyStore>>,
        checkpoint: Option<BlockHeight>,
        confirmation_threshold: u32,
        network: Network,
    ) -> Result<Self, OrchestratorError> {
        // We should pass node_rpc_url and that is all. Client should be removed.
        // The only one that connects with the blockchain is the dispatcher and the indexer.
        // So here should be initialized the BitcoinClient
        let monitor = Monitor::new_with_paths(
            rpc_url,
            rpc_user,
            rpc_pass,
            storage.clone(),
            checkpoint,
            confirmation_threshold,
        )?;

        let store = OrchestratorStore::new(storage)?;
        let account = Account::new(network);
        let dispatcher = TransactionDispatcher::new_with_path(rpc_url, rpc_user, rpc_pass, key_manager)?;
        let orchestrator = Orchestrator::new(monitor, store, dispatcher, account);

        Ok(orchestrator)
    }
}

impl<M, D, B> Orchestrator<M, D, B>
where
    M: MonitorApi,
    D: TransactionDispatcherApi,
    B: OrchestratorStoreApi,
{
    pub fn new(monitor: M, store: B, dispatcher: D, account: Account) -> Self {
        Self {
            monitor,
            dispatcher,
            store,
            current_height: 0,
            account: account.clone(),
        }
    }

    fn process_pending_txs(&mut self) -> Result<(), OrchestratorError> {
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

                // TODO: check atomics transactions. to perform add and remove.

                self.store.update_instance_tx_as_sent(
                    instance_id,
                    &tx_info.tx_id,
                    self.current_height,
                )?;
            }
        }

        Ok(())
    }

    fn speed_up(
        &self,
        instance_id: InstanceId,
        tx: &Transaction,
        funding_txid: Txid,
        tx_public_key: PublicKey,
        funding_utxo: (u32, TxOut, PublicKey),
    ) -> Result<(), OrchestratorError> {
        let dispatch_result =
            self.dispatcher
                .speed_up(tx, tx_public_key, funding_txid, funding_utxo.clone());

        if let Err(error) = dispatch_result {
            match error {
                DispatcherError::InsufficientFunds => {
                    self.store.add_funding_request(instance_id)?;
                    return Ok(());
                }
                e => return Err(e.into()),
            }
        }

        if dispatch_result.is_ok() {
            let (speed_up_tx_id, deliver_fee_rate) = dispatch_result.unwrap();

            let speed_up_tx = SpeedUpTx {
                tx_id: speed_up_tx_id,
                deliver_fee_rate,
                deliver_block_height: self.current_height,
                child_tx_id: tx.compute_txid(),
                utxo_index: funding_utxo.0,
                utxo_output: funding_utxo.1,
            };

            self.store.add_speed_up_tx(instance_id, &speed_up_tx)?;

            self.monitor
                .save_transaction_for_tracking(instance_id, speed_up_tx_id)?;
        }

        Ok(())
    }

    fn process_in_progress_txs(&mut self) -> Result<(), OrchestratorError> {
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
                let tx_status = self.monitor.get_instance_tx_status(&tx_info.tx_id)?;

                self.process_instance_tx_change(instance_id, &tx_status.unwrap())?;
            }
        }

        Ok(())
    }

    fn process_instance_news(&mut self) -> Result<(), OrchestratorError> {
        // Get any news in each instance that are being monitored.
        // Get instances news also returns the speed ups txs added for each instance.
        let news = self.monitor.get_instance_news()?;

        for (instance_id, txs_status) in &news {
            info!(
                "{} Instance ID: {} has new transactions: {}",
                style("News").green(),
                style(instance_id).green(),
                style(txs_status.len()).red()
            );
        }

        for (instance_id, txs_status) in news {
            for tx_status in txs_status {
                let is_speed_up = self.store.is_speed_up_tx(instance_id, &tx_status.tx_id)?;

                if is_speed_up {
                    self.process_speed_up_change(instance_id, &tx_status)?;
                } else {
                    self.process_instance_tx_change(instance_id, &tx_status)?;
                }

                // Acknowledge the transaction news to the monitor to update its state.
                // This step ensures that the monitor is aware of the transaction's completion and can update its tracking accordingly.
                self.monitor
                    .acknowledge_instance_tx_news(instance_id, &tx_status.tx_id)?;
            }
        }

        Ok(())
    }

    fn process_speed_up_change(
        &mut self,
        instance_id: InstanceId,
        tx_status: &TransactionStatus,
    ) -> Result<(), OrchestratorError> {
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

    fn process_instance_tx_change(
        &mut self,
        instance_id: InstanceId,
        tx_status: &TransactionStatus,
    ) -> Result<(), OrchestratorError> {
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
                    self.store.update_instance_tx_status(
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
                self.store.update_instance_tx_status(
                    instance_id,
                    &tx_status.tx_id,
                    TransactionState::Orphan,
                )?;

                self.store
                    .add_instance_tx_news(instance_id, tx_status.tx_id)?;
            }
        }

        Ok(())
    }

    fn handle_confirmation_transaction(
        &mut self,
        instance_id: InstanceId,
        tx_status: &TransactionStatus,
    ) -> Result<(), OrchestratorError> {
        // Update the transaction to completed given that transaction has more than the threshold confirmations
        self.store.update_instance_tx_status(
            instance_id,
            &tx_status.tx_id,
            TransactionState::Confirmed,
        )?;

        self.store
            .add_instance_tx_news(instance_id, tx_status.tx_id)?;

        Ok(())
    }

    fn handle_confirmation_speed_up_transaction(
        &mut self,
        instance_id: InstanceId,
        speed_up_tx_id: &Txid,
    ) -> Result<(), OrchestratorError> {
        let speed_up_tx = self
            .store
            .get_speed_up_tx(instance_id, speed_up_tx_id)?
            .unwrap();

        //Confirmation in 1 means the transaction is already included in the block.
        //The new transaction funding is gonna be this a speed-up transaction.
        let funding_info = FundingTx {
            tx_id: speed_up_tx.tx_id,
            utxo_index: speed_up_tx.utxo_index,
            utxo_output: speed_up_tx.utxo_output.clone(),
        };

        //TODO: There is something missing here. We are moving a speed-up transaction to a funding transaction.
        // The inverse should also be supported.
        self.store.add_funding_tx(instance_id, &funding_info)?;

        Ok(())
    }

    fn handle_orphan_speed_up_transaction(
        &mut self,
        instance_id: InstanceId,
        speed_up_tx_id: &Txid,
    ) -> Result<(), OrchestratorError> {
        //Speed up previouly was mined, now is orphan then, we have to remove it as a funding tx.
        self.store.remove_funding_tx(instance_id, speed_up_tx_id)?;

        Ok(())
    }

    fn handle_complete_transaction(
        &mut self,
        instance_id: InstanceId,
        tx_status: &TransactionStatus,
    ) -> Result<(), OrchestratorError> {
        // Transaction was mined and has sufficient confirmations to mark it as complete.

        // Update the transaction to completed given that transaction has more than the threshold confirmations
        self.store.update_instance_tx_status(
            instance_id,
            &tx_status.tx_id,
            TransactionState::Finalized,
        )?;

        self.store
            .add_instance_tx_news(instance_id, tx_status.tx_id)?;

        Ok(())
    }

    fn handle_unseen_transaction(
        &mut self,
        instance_id: InstanceId,
        tx_data: &TransactionInfo,
    ) -> Result<(), OrchestratorError> {
        // We get all the existing speed up transaction for tx_id. Then we figure out if we should speed it up again.
        let speed_up_txs = self
            .store
            .get_speed_up_txs_for_child(instance_id, &tx_data.tx_id)?;

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

        //We are gonna have a funding transaction for each Bitvmx instance.
        let funding_tx =
            self.store
                .get_funding_tx(instance_id)?
                .ok_or(OrchestratorError::OrchestratorError(
                    "No funding transaction available for speed up".to_string(),
                ))?;

        self.speed_up(
            instance_id,
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

impl<M, D, B> OrchestratorApi for Orchestrator<M, D, B>
where
    M: MonitorApi,
    D: TransactionDispatcherApi,
    B: OrchestratorStoreApi,
{
    fn tick(&mut self) -> Result<(), OrchestratorError> {
        //TODO Question: Should we handle the scenario where there are more than one instance per operator running?
        // This scenario raises concerns that the protocol should be aware of a transaction that belongs to it but was not sent by itself (was seen in the blockchain)

        // The monitor is considered ready when it has fully indexed the blockchain and is up to date with the latest block.
        // Note that if there is a significant gap in the indexing process, it may take multiple ticks for the monitor to become ready.

        if !self.monitor.is_ready()? {
            info!("Monitor is not ready yet, continuing to index blockchain.");

            self.monitor.tick()?;

            return Ok(());
        }

        //TODO QUESTION?: I think we could not recieve a tx to be send for an instance that
        //  has a pending tx be dispatch. Otherwise we could add some warning..

        // Send pending transactions that were queued.
        self.process_pending_txs()?;

        let last_block_height: u32 = self.current_height;
        self.current_height = self.monitor.get_current_height();

        // If the last block height is the same as the current one, there's no need to continue.
        if last_block_height == self.current_height {
            return Ok(());
        }

        // Handle any updates related to instances, including new information about transactions that have not been reviewed yet.
        self.process_instance_news()?;

        // Handle any updates related to instances, including new information about transactions that have not been reviewed yet.
        self.process_in_progress_txs()?;

        Ok(())
    }

    fn monitor_instance(
        &self,
        instance: &BitvmxInstance<TransactionPartialInfo>,
    ) -> Result<(), OrchestratorError> {
        if instance.txs.is_empty() {
            return Err(OrchestratorError::OrchestratorError(
                "Instance txs array is empty".to_string(),
            ));
        }

        //TODO: we could add some validation to check instance and txs existence in the storage

        self.store.add_instance(instance)?;

        let instance_new = InstanceData {
            instance_id: instance.instance_id,
            txs: instance.txs.iter().map(|tx| tx.tx_id).collect(),
        };

        // When an instance is saved in the monitor for tracking,
        // the current height of the indexer is used as the starting point for tracking.
        // This is not currently configurable.
        // It may change if we found a case where it should be configurable.

        self.monitor
            .save_instances_for_tracking(vec![instance_new])?;

        Ok(())
    }

    fn is_ready(&mut self) -> Result<bool, OrchestratorError> {
        //TODO: The orchestrator is currently considered ready when the monitor is ready.
        // However, we may decide to take into consideration pending and in progress transactions in the future.
        let result = self.monitor.is_ready()?;
        Ok(result)
    }

    fn send_tx_instance(
        &self,
        instance_id: InstanceId,
        tx: &Transaction,
    ) -> Result<(), OrchestratorError> {
        // This section of code is responsible for adding a transaction to an instance and marking it as pending.
        // First, it adds the transaction to the instance using `add_tx_to_instance`. This method updates the instance
        // to include the new transaction, ensuring it is associated with the correct instance.

        self.store.add_tx_to_instance(instance_id, tx)?;

        // Next, it marks the transaction as pending using `add_pending_instance_tx`. This method updates the storage
        // to indicate that the transaction is currently pending and needs to be processed.
        self.store.update_instance_tx_status(
            instance_id,
            &tx.compute_txid(),
            TransactionState::ReadyToSend,
        )?;

        info!(
            "{} Transaction ID {} for Instance ID {} move to Pending status to be send.",
            style("Orchestrator").green(),
            style(tx.compute_txid()).yellow(),
            style(instance_id).green()
        );
        Ok(())
    }

    fn add_tx_to_instance(
        &self,
        _instance_id: InstanceId,
        _tx: &Txid,
    ) -> Result<(), OrchestratorError> {
        // Add a non-existent transaction to an existing instance.
        // The instance should exist in the storage.
        // The transaction id should not exist in the storage.
        // Usage: This method will likely be used for the final transaction to withdraw the funds.
        Ok(())
    }

    fn add_funding_tx(
        &self,
        instance_id: InstanceId,
        funding_tx: &FundingTx,
    ) -> Result<(), OrchestratorError> {
        self.store.add_funding_tx(instance_id, funding_tx)?;
        Ok(())
    }

    fn monitor_address(&self, address: Address) -> Result<(), OrchestratorError> {
        self.monitor.save_address_for_tracking(address)?;
        Ok(())
    }

    fn get_news(&self) -> Result<News, OrchestratorError> {
        let instance_tx_news = self.store.get_instance_tx_news()?;
        let mut txs_by_id: Vec<(InstanceId, Vec<TransactionNew>)> = Vec::new();

        for (instance_id, tx_ids) in instance_tx_news {
            let mut instance_txs = Vec::new();

            for tx_id in tx_ids {
                // Transaction information is stored in both the monitor and orchastrator storage.
                // News are generated when a transaction changes state to either:
                // - Orphaned (removed from chain due to reorg)
                // - Confirmed (included in a block)
                // - Finalized (reached required confirmation threshold)
                let tx_info = self
                    .store
                    .get_instance_tx(instance_id, &tx_id)?
                    .expect("Transaction not found in instance");

                let tx_status = self
                    .monitor
                    .get_instance_tx_status(&tx_id)?
                    .expect("Transaction status not found in monitor");

                let status = match tx_info.state {
                    TransactionState::Orphan => TransactionBlockchainStatus::Orphan,
                    TransactionState::Confirmed => TransactionBlockchainStatus::Confirmed,
                    TransactionState::Finalized => TransactionBlockchainStatus::Finalized,
                    _ => continue, // Skip other states
                };

                instance_txs.push(TransactionNew {
                    tx: tx_status.tx.unwrap(),
                    block_info: tx_status.block_info.unwrap(),
                    confirmations: tx_status.confirmations,
                    status,
                });
            }

            txs_by_id.push((instance_id, instance_txs));
        }

        let address_news = self.monitor.get_address_news()?;

        let mut txs_by_address = Vec::new();

        for (address, address_statuses) in address_news {
            for address_status in address_statuses {
                let mut txs = Vec::new();

                let tx_status = self
                    .monitor
                    .get_instance_tx_status(&address_status.tx.unwrap().compute_txid())?
                    .unwrap();

                let state = if tx_status.confirmations > self.monitor.get_confirmation_threshold() {
                    TransactionBlockchainStatus::Finalized
                } else if tx_status.confirmations == 1 {
                    TransactionBlockchainStatus::Confirmed
                } else {
                    TransactionBlockchainStatus::Orphan
                };

                txs.push(AddressNew {
                    tx: tx_status.tx.unwrap(),
                    block_info: tx_status.block_info.unwrap(),
                    confirmations: tx_status.confirmations,
                    status: state,
                });

                txs_by_address.push((address.clone(), txs));
            }
        }

        let funds_requests = self.store.get_funding_requests()?;

        Ok(News {
            txs_by_id,
            txs_by_address,
            funds_requests,
        })
    }

    fn acknowledge_news(&self, processed_news: ProcessedNews) -> Result<(), OrchestratorError> {
        // Acknowledge transaction news for each instance
        for (instance_id, tx_ids) in processed_news.txs_by_id {
            for tx_id in tx_ids {
                self.store
                    .acknowledge_instance_tx_news(instance_id, tx_id)?;
            }
        }

        // Acknowledge address news
        for address in processed_news.txs_by_address {
            self.monitor.acknowledge_address_news(address)?;
        }

        // Acknowledge funding requests
        for instance_id in processed_news.funds_requests {
            self.store.acknowledge_funding_request(instance_id)?;
        }

        Ok(())
    }
}

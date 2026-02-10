use crate::{
    errors::BitcoinCoordinatorStoreError,
    types::{AckCoordinatorNews, CoordinatedTransaction, CoordinatorNews, TransactionState},
};

use bitcoin::{BlockHash, Transaction, Txid};
use bitvmx_bitcoin_rpc::types::BlockHeight;
use protocol_builder::types::output::SpeedupData;
use std::rc::Rc;
use storage_backend::storage::{KeyValueStore, Storage};
pub struct BitcoinCoordinatorStore {
    pub store: Rc<Storage>,
    pub max_unconfirmed_speedups: u32,
}
enum StoreKey {
    ActiveTransactionList,
    Transaction(Txid),
    DispatchTransactionErrorNewsList,
    DispatchSpeedUpErrorNewsList,
    InsufficientFundsNewsList,
    FundingNotFoundNews,
    EstimateFeerateTooHighNewsList,
    TransactionAlreadyInMempoolNewsList,
    MempoolRejectionNewsList,
    NetworkErrorNewsList,
}
pub trait BitcoinCoordinatorStoreApi {
    fn save_tx(
        &self,
        tx: Transaction,
        speedup_data: Option<SpeedupData>,
        target_block_height: Option<BlockHeight>,
        context: String,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn remove_tx(&self, tx_id: Txid) -> Result<(), BitcoinCoordinatorStoreError>;

    fn get_txs_in_progress(
        &self,
    ) -> Result<Vec<CoordinatedTransaction>, BitcoinCoordinatorStoreError>;

    fn get_active_transactions(
        &self,
    ) -> Result<Vec<CoordinatedTransaction>, BitcoinCoordinatorStoreError>;

    fn get_tx(&self, tx_id: &Txid) -> Result<CoordinatedTransaction, BitcoinCoordinatorStoreError>;

    fn update_tx_state(
        &self,
        tx_id: Txid,
        status: TransactionState,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn update_tx_to_dispatched(
        &self,
        tx_id: Txid,
        deliver_block_height: u32,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn update_news(
        &self,
        news: CoordinatorNews,
        current_block_hash: BlockHash,
    ) -> Result<(), BitcoinCoordinatorStoreError>;
    fn ack_news(&self, news: AckCoordinatorNews) -> Result<(), BitcoinCoordinatorStoreError>;
    fn get_news(&self) -> Result<Vec<CoordinatorNews>, BitcoinCoordinatorStoreError>;
}

impl BitcoinCoordinatorStore {
    pub fn new(
        store: Rc<Storage>,
        max_unconfirmed_speedups: u32,
    ) -> Result<Self, BitcoinCoordinatorStoreError> {
        Ok(Self {
            store,
            max_unconfirmed_speedups,
        })
    }

    fn get_key(&self, key: StoreKey) -> String {
        let prefix = "bitcoin_coordinator";
        match key {
            StoreKey::ActiveTransactionList => format!("{prefix}/tx/list"),
            StoreKey::Transaction(tx_id) => format!("{prefix}/tx/{tx_id}"),

            //NEWS
            StoreKey::InsufficientFundsNewsList => format!("{prefix}/news/insufficient_funds"),
            StoreKey::DispatchTransactionErrorNewsList => {
                format!("{prefix}/news/dispatch_transaction_error")
            }
            StoreKey::DispatchSpeedUpErrorNewsList => {
                format!("{prefix}/news/dispatch_speed_up_error")
            }
            StoreKey::FundingNotFoundNews => format!("{prefix}/news/funding_not_found"),
            StoreKey::EstimateFeerateTooHighNewsList => {
                format!("{prefix}/news/estimate_feerate_too_high")
            }
            StoreKey::TransactionAlreadyInMempoolNewsList => {
                format!("{prefix}/news/transaction_already_in_mempool")
            }
            StoreKey::MempoolRejectionNewsList => {
                format!("{prefix}/news/mempool_rejection")
            }
            StoreKey::NetworkErrorNewsList => format!("{prefix}/news/network_error"),
        }
    }

    fn get_txs(&self) -> Result<Vec<Txid>, BitcoinCoordinatorStoreError> {
        let key = self.get_key(StoreKey::ActiveTransactionList);

        let all_txs = self.store.get::<&str, Vec<Txid>>(&key)?;

        match all_txs {
            Some(txs) => Ok(txs),
            None => Ok(vec![]),
        }
    }
}

impl BitcoinCoordinatorStoreApi for BitcoinCoordinatorStore {
    fn get_tx(&self, tx_id: &Txid) -> Result<CoordinatedTransaction, BitcoinCoordinatorStoreError> {
        let key = self.get_key(StoreKey::Transaction(*tx_id));
        let tx = self.store.get::<&str, CoordinatedTransaction>(&key)?;

        if let Some(tx) = tx {
            Ok(tx)
        } else {
            let message = format!("Transaction not found: {tx_id}");
            Err(BitcoinCoordinatorStoreError::TransactionNotFound(message))
        }
    }

    fn get_txs_in_progress(
        &self,
    ) -> Result<Vec<CoordinatedTransaction>, BitcoinCoordinatorStoreError> {
        // Get all transactions in progress which are the ones are not Finalized
        let txs = self.get_txs()?;
        let mut txs_filter = Vec::new();

        for tx_id in txs {
            let tx = self.get_tx(&tx_id)?;

            if tx.state == TransactionState::ToDispatch
                || tx.state == TransactionState::InMempool
                || tx.state == TransactionState::Confirmed
            {
                txs_filter.push(tx);
            }
        }

        Ok(txs_filter)
    }

    fn get_active_transactions(
        &self,
    ) -> Result<Vec<CoordinatedTransaction>, BitcoinCoordinatorStoreError> {
        // Get all transactions in progress (ToDispatch, InMempool, Confirmed) until they are finalized
        let txs = self.get_txs()?;
        let mut txs_filter = Vec::new();

        for tx_id in txs {
            let tx = self.get_tx(&tx_id)?;

            // Include transactions that are in progress (not finalized, not failed, not replaced)
            // Failed and Replaced transactions are not active - they represent fatal errors or superseded transactions that cannot be retried
            if tx.state == TransactionState::ToDispatch
                || tx.state == TransactionState::InMempool
                || tx.state == TransactionState::Confirmed
            {
                txs_filter.push(tx);
            }
        }

        Ok(txs_filter)
    }

    fn save_tx(
        &self,
        tx: Transaction,
        speedup_data: Option<SpeedupData>,
        target_block_height: Option<BlockHeight>,
        context: String,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let key = self.get_key(StoreKey::Transaction(tx.compute_txid()));

        let tx_info = CoordinatedTransaction::new(
            tx.clone(),
            speedup_data,
            TransactionState::ToDispatch,
            target_block_height,
            context,
        );

        self.store.set(&key, &tx_info, None)?;

        let txs_key = self.get_key(StoreKey::ActiveTransactionList);
        let mut txs = self
            .store
            .get::<&str, Vec<Txid>>(&txs_key)?
            .unwrap_or_default();
        txs.push(tx.compute_txid());
        self.store.set(&txs_key, &txs, None)?;

        Ok(())
    }

    fn remove_tx(&self, tx_id: Txid) -> Result<(), BitcoinCoordinatorStoreError> {
        let tx_key = self.get_key(StoreKey::Transaction(tx_id));
        self.store.remove(&tx_key, None)?;

        let txs_key = self.get_key(StoreKey::ActiveTransactionList);
        let mut txs = self
            .store
            .get::<&str, Vec<Txid>>(&txs_key)?
            .unwrap_or_default();

        txs.retain(|id| *id != tx_id);
        self.store.set(&txs_key, &txs, None)?;

        Ok(())
    }

    fn update_tx_to_dispatched(
        &self,
        tx_id: Txid,
        deliver_block_height: u32,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let mut tx = self.get_tx(&tx_id)?;

        // Validate state transition: only ToDispatch can transition to InMempool
        if tx.state != TransactionState::ToDispatch {
            return Err(BitcoinCoordinatorStoreError::InvalidTransactionState);
        }

        tx.state = TransactionState::InMempool;

        tx.broadcast_block_height = Some(deliver_block_height);

        let key = self.get_key(StoreKey::Transaction(tx_id));
        self.store.set(key, tx, None)?;

        Ok(())
    }

    fn update_tx_state(
        &self,
        tx_id: Txid,
        new_state: TransactionState,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let mut tx = self.get_tx(&tx_id)?;

        // Validate state transitions
        let valid_transition = match (&tx.state, &new_state) {
            // Valid transitions
            (TransactionState::ToDispatch, TransactionState::InMempool) => true,
            (TransactionState::ToDispatch, TransactionState::Failed) => true,
            (TransactionState::InMempool, TransactionState::Confirmed) => true,
            (TransactionState::Confirmed, TransactionState::Finalized) => true,
            // Allow transition from Confirmed to InMempool when transaction becomes orphan (reorg)
            (TransactionState::Confirmed, TransactionState::InMempool) => true,
            (current, new) if current == new => true,
            // Invalid transitions
            _ => false,
        };

        if !valid_transition {
            return Err(BitcoinCoordinatorStoreError::InvalidStateTransition(
                tx.state.clone(),
                new_state.clone(),
                tx_id,
            ));
        }

        tx.state = new_state.clone();

        let key = self.get_key(StoreKey::Transaction(tx_id));
        self.store.set(key, tx, None)?;

        // Remove tx from the list if it is finalized
        if new_state == TransactionState::Finalized {
            let txs_key = self.get_key(StoreKey::ActiveTransactionList);
            let mut txs = self
                .store
                .get::<&str, Vec<Txid>>(&txs_key)?
                .unwrap_or_default();
            txs.retain(|id| *id != tx_id);
            self.store.set(&txs_key, &txs, None)?;
        }

        Ok(())
    }

    fn update_news(
        &self,
        news: CoordinatorNews,
        current_block_hash: BlockHash,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        match news {
            CoordinatorNews::InsufficientFunds(tx_id, amount, required) => {
                let key = self.get_key(StoreKey::InsufficientFundsNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, u64, u64, (BlockHash, bool))>>(&key)?
                    .unwrap_or_default();

                let is_new_news = news_list.iter().position(|(id, _, _, _)| id == &tx_id);

                if let Some(pos) = is_new_news {
                    let (_, _, _, (existing_block_hash, _)) = &news_list[pos];
                    if existing_block_hash == &current_block_hash {
                        // We already have this news, do not update
                        return Ok(());
                    } else {
                        // Replace the notification if the block hash is different
                        news_list[pos] = (tx_id, amount, required, (current_block_hash, false));
                    }
                } else {
                    // Insert news with current block hash and ack in false
                    news_list.push((tx_id, amount, required, (current_block_hash, false)));
                }

                self.store.set(&key, &news_list, None)?;
            }
            CoordinatorNews::DispatchTransactionError(tx_id, context, error) => {
                let key = self.get_key(StoreKey::DispatchTransactionErrorNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, String, String, (BlockHash, bool))>>(&key)?
                    .unwrap_or_default();

                let is_new_news = news_list.iter().position(|(id, _, _, _)| id == &tx_id);

                if let Some(pos) = is_new_news {
                    let (_, _, _, (last_block_hash, _)) = &news_list[pos];

                    if last_block_hash != &current_block_hash {
                        // Update the news if the block hash is different
                        news_list[pos] = (tx_id, context, error, (current_block_hash, false));
                    }
                } else {
                    // Insert news if it doesn't already exist
                    news_list.push((tx_id, context, error, (current_block_hash, false)));
                }

                self.store.set(&key, &news_list, None)?;
            }
            CoordinatorNews::DispatchSpeedUpError(tx_ids, contexts, txid, error) => {
                let key = self.get_key(StoreKey::DispatchSpeedUpErrorNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Vec<Txid>, Vec<String>, Txid, String, (BlockHash, bool))>>(
                        &key,
                    )?
                    .unwrap_or_default();

                let is_new_news = news_list
                    .iter()
                    .position(|(ids, _, id, _, _)| ids == &tx_ids && id == &txid);

                if let Some(pos) = is_new_news {
                    let (_, _, _, _, (last_block_hash, _)) = &news_list[pos];

                    if last_block_hash != &current_block_hash {
                        // Update the news if the block hash is different
                        news_list[pos] =
                            (tx_ids, contexts, txid, error, (current_block_hash, false));
                    }
                } else {
                    // Insert news if it doesn't already exist
                    news_list.push((tx_ids, contexts, txid, error, (current_block_hash, false)));
                }

                self.store.set(&key, &news_list, None)?;
            }
            CoordinatorNews::FundingNotFound => {
                let key = self.get_key(StoreKey::FundingNotFoundNews);
                let news = self.store.get::<&str, (BlockHash, bool)>(&key)?;

                if let Some((last_block_hash, _)) = news {
                    // If there is existing news, check if the block hash differs
                    if last_block_hash != current_block_hash {
                        self.store.set(&key, (current_block_hash, false), None)?;
                    }
                } else {
                    // If no existing news, set the current block hash and mark it as not acknowledged
                    self.store.set(&key, (current_block_hash, false), None)?;
                }
            }
            CoordinatorNews::EstimateFeerateTooHigh(estimate_fee, max_allowed) => {
                let key = self.get_key(StoreKey::EstimateFeerateTooHighNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(u64, u64, (BlockHash, bool))>>(&key)?
                    .unwrap_or_default();

                let is_new_news = news_list
                    .iter()
                    .position(|(fee, max, _)| *fee == estimate_fee && *max == max_allowed);

                if let Some(pos) = is_new_news {
                    let (_, _, (last_block_hash, _)) = &news_list[pos];

                    if last_block_hash != &current_block_hash {
                        // Replace the notification if the block hash is different
                        news_list[pos] = (estimate_fee, max_allowed, (current_block_hash, false));
                    }
                } else {
                    // Insert news if it doesn't already exist
                    news_list.push((estimate_fee, max_allowed, (current_block_hash, false)));
                }

                self.store.set(&key, &news_list, None)?;
            }
            CoordinatorNews::TransactionAlreadyInMempool(tx_id, context) => {
                let key = self.get_key(StoreKey::TransactionAlreadyInMempoolNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, String, (BlockHash, bool))>>(&key)?
                    .unwrap_or_default();

                let is_new_news = news_list.iter().position(|(id, _, _)| id == &tx_id);

                if let Some(pos) = is_new_news {
                    let (_, _, (last_block_hash, _)) = &news_list[pos];

                    if last_block_hash != &current_block_hash {
                        news_list[pos] = (tx_id, context, (current_block_hash, false));
                    }
                } else {
                    news_list.push((tx_id, context, (current_block_hash, false)));
                }

                self.store.set(&key, &news_list, None)?;
            }
            CoordinatorNews::MempoolRejection(tx_id, context, error) => {
                let key = self.get_key(StoreKey::MempoolRejectionNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, String, String, (BlockHash, bool))>>(&key)?
                    .unwrap_or_default();

                let is_new_news = news_list.iter().position(|(id, _, _, _)| id == &tx_id);

                if let Some(pos) = is_new_news {
                    let (_, _, _, (last_block_hash, _)) = &news_list[pos];

                    if last_block_hash != &current_block_hash {
                        news_list[pos] = (tx_id, context, error, (current_block_hash, false));
                    }
                } else {
                    news_list.push((tx_id, context, error, (current_block_hash, false)));
                }

                self.store.set(&key, &news_list, None)?;
            }
            CoordinatorNews::NetworkError(tx_id, context, error) => {
                let key = self.get_key(StoreKey::NetworkErrorNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, String, String, (BlockHash, bool))>>(&key)?
                    .unwrap_or_default();

                let is_new_news = news_list.iter().position(|(id, _, _, _)| id == &tx_id);

                if let Some(pos) = is_new_news {
                    let (_, _, _, (last_block_hash, _)) = &news_list[pos];
                    if last_block_hash != &current_block_hash {
                        news_list[pos] = (tx_id, context, error, (current_block_hash, false));
                    }
                } else {
                    news_list.push((tx_id, context, error, (current_block_hash, false)));
                }

                self.store.set(&key, &news_list, None)?;
            }
        }
        Ok(())
    }

    fn ack_news(&self, news: AckCoordinatorNews) -> Result<(), BitcoinCoordinatorStoreError> {
        match news {
            AckCoordinatorNews::InsufficientFunds(tx_id) => {
                let key = self.get_key(StoreKey::InsufficientFundsNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, u64, u64, (BlockHash, bool))>>(&key)?
                    .unwrap_or_default();

                if let Some(pos) = news_list.iter().position(|(id, _, _, _)| *id == tx_id) {
                    let (_, _, _, (_, ack)) = &mut news_list[pos];
                    *ack = true;
                    self.store.set(&key, &news_list, None)?;
                }
            }
            AckCoordinatorNews::DispatchTransactionError(tx_id) => {
                let key = self.get_key(StoreKey::DispatchTransactionErrorNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, String, String, (BlockHash, bool))>>(&key)?
                    .unwrap_or_default();

                if let Some(pos) = news_list.iter().position(|(id, _, _, _)| *id == tx_id) {
                    let (_, _, _, (_, ack)) = &mut news_list[pos];
                    *ack = true;
                    self.store.set(&key, &news_list, None)?;
                }
            }
            AckCoordinatorNews::DispatchSpeedUpError(speedup_txid) => {
                let key = self.get_key(StoreKey::DispatchSpeedUpErrorNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Vec<Txid>, Vec<String>, Txid, String, (BlockHash, bool))>>(
                        &key,
                    )?
                    .unwrap_or_default();

                if let Some(pos) = news_list
                    .iter()
                    .position(|(_, _, txid, _, _)| *txid == speedup_txid)
                {
                    let (_, _, _, _, (_, ack)) = &mut news_list[pos];
                    *ack = true;
                    self.store.set(&key, &news_list, None)?;
                }
            }
            AckCoordinatorNews::EstimateFeerateTooHigh(estimate_fee, max_allowed) => {
                let key = self.get_key(StoreKey::EstimateFeerateTooHighNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(u64, u64, (BlockHash, bool))>>(&key)?
                    .unwrap_or_default();

                if let Some(pos) = news_list
                    .iter()
                    .position(|(fee, max, _)| *fee == estimate_fee && *max == max_allowed)
                {
                    let (_, _, (_, ack)) = &mut news_list[pos];
                    *ack = true;
                    self.store.set(&key, &news_list, None)?;
                }
            }
            AckCoordinatorNews::FundingNotFound => {
                let key = self.get_key(StoreKey::FundingNotFoundNews);
                let mut news = self.store.get::<&str, (BlockHash, bool)>(&key)?;

                if let Some((block_hash, _)) = news {
                    news = Some((block_hash, true));
                    self.store.set(&key, news, None)?;
                }
            }
            AckCoordinatorNews::TransactionAlreadyInMempool(tx_id) => {
                let key = self.get_key(StoreKey::TransactionAlreadyInMempoolNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, String, (BlockHash, bool))>>(&key)?
                    .unwrap_or_default();

                if let Some(pos) = news_list.iter().position(|(id, _, _)| *id == tx_id) {
                    let (_, _, (_, ack)) = &mut news_list[pos];
                    *ack = true;
                    self.store.set(&key, &news_list, None)?;
                }
            }
            AckCoordinatorNews::MempoolRejection(tx_id) => {
                let key = self.get_key(StoreKey::MempoolRejectionNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, String, String, (BlockHash, bool))>>(&key)?
                    .unwrap_or_default();

                if let Some(pos) = news_list.iter().position(|(id, _, _, _)| *id == tx_id) {
                    let (_, _, _, (_, ack)) = &mut news_list[pos];
                    *ack = true;
                    self.store.set(&key, &news_list, None)?;
                }
            }
            AckCoordinatorNews::NetworkError(tx_id) => {
                let key = self.get_key(StoreKey::NetworkErrorNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, String, String, (BlockHash, bool))>>(&key)?
                    .unwrap_or_default();

                if let Some(pos) = news_list.iter().position(|(id, _, _, _)| *id == tx_id) {
                    let (_, _, _, (_, ack)) = &mut news_list[pos];
                    *ack = true;
                    self.store.set(&key, &news_list, None)?;
                }
            }
        }
        Ok(())
    }

    fn get_news(&self) -> Result<Vec<CoordinatorNews>, BitcoinCoordinatorStoreError> {
        let mut all_news = Vec::new();

        // Get insufficient funds news
        let insufficient_funds_key = self.get_key(StoreKey::InsufficientFundsNewsList);
        if let Some(news_list) = self
            .store
            .get::<&str, Vec<(Txid, u64, u64, (BlockHash, bool))>>(&insufficient_funds_key)?
        {
            for (txid, amount, required, (_, acked)) in news_list {
                if !acked {
                    all_news.push(CoordinatorNews::InsufficientFunds(txid, amount, required));
                }
            }
        }

        // Get dispatch error news
        let dispatch_error_key = self.get_key(StoreKey::DispatchTransactionErrorNewsList);
        if let Some(news_list) = self
            .store
            .get::<&str, Vec<(Txid, String, String, (BlockHash, bool))>>(&dispatch_error_key)?
        {
            for (tx_id, context, error, (_, acked)) in news_list {
                if !acked {
                    all_news.push(CoordinatorNews::DispatchTransactionError(
                        tx_id, context, error,
                    ));
                }
            }
        }

        // Get speed up error news
        let speed_up_error_key = self.get_key(StoreKey::DispatchSpeedUpErrorNewsList);
        if let Some(news_list) =
            self.store
                .get::<&str, Vec<(Vec<Txid>, Vec<String>, Txid, String, (BlockHash, bool))>>(
                    &speed_up_error_key,
                )?
        {
            for (tx_ids, contexts, txid, error, (_, acked)) in news_list {
                if !acked {
                    all_news.push(CoordinatorNews::DispatchSpeedUpError(
                        tx_ids, contexts, txid, error,
                    ));
                }
            }
        }

        // Get funding not found news
        let funding_not_found_key = self.get_key(StoreKey::FundingNotFoundNews);
        if let Some((_, acked)) = self
            .store
            .get::<&str, (BlockHash, bool)>(&funding_not_found_key)?
        {
            if !acked {
                all_news.push(CoordinatorNews::FundingNotFound);
            }
        }

        // Get estimate feerate too high news
        let estimate_feerate_too_high_key = self.get_key(StoreKey::EstimateFeerateTooHighNewsList);
        if let Some(news_list) = self
            .store
            .get::<&str, Vec<(u64, u64, (BlockHash, bool))>>(&estimate_feerate_too_high_key)?
        {
            for (estimate_fee, max_allowed, (_, acked)) in news_list {
                if !acked {
                    all_news.push(CoordinatorNews::EstimateFeerateTooHigh(
                        estimate_fee,
                        max_allowed,
                    ));
                }
            }
        }

        // Get transaction already in mempool news
        let already_in_mempool_key = self.get_key(StoreKey::TransactionAlreadyInMempoolNewsList);
        if let Some(news_list) = self
            .store
            .get::<&str, Vec<(Txid, String, (BlockHash, bool))>>(&already_in_mempool_key)?
        {
            for (tx_id, context, (_, acked)) in news_list {
                if !acked {
                    all_news.push(CoordinatorNews::TransactionAlreadyInMempool(tx_id, context));
                }
            }
        }

        // Get mempool rejection news
        let mempool_rejection_key = self.get_key(StoreKey::MempoolRejectionNewsList);
        if let Some(news_list) = self
            .store
            .get::<&str, Vec<(Txid, String, String, (BlockHash, bool))>>(&mempool_rejection_key)?
        {
            for (tx_id, context, error, (_, acked)) in news_list {
                if !acked {
                    all_news.push(CoordinatorNews::MempoolRejection(tx_id, context, error));
                }
            }
        }

        // Get network error news
        let network_error_key = self.get_key(StoreKey::NetworkErrorNewsList);
        if let Some(news_list) = self
            .store
            .get::<&str, Vec<(Txid, String, String, (BlockHash, bool))>>(&network_error_key)?
        {
            for (tx_id, context, error, (_, acked)) in news_list {
                if !acked {
                    all_news.push(CoordinatorNews::NetworkError(tx_id, context, error));
                }
            }
        }

        Ok(all_news)
    }
}

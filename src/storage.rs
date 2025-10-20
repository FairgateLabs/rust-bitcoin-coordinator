use crate::{
    errors::BitcoinCoordinatorStoreError,
    types::{
        AckCoordinatorNews, CoordinatedTransaction, CoordinatorNews, RetryInfo, TransactionState,
    },
};

use bitcoin::{BlockHash, Transaction, Txid};
use bitvmx_bitcoin_rpc::types::BlockHeight;
use chrono::Utc;
use protocol_builder::types::output::SpeedupData;
use std::rc::Rc;
use storage_backend::storage::{KeyValueStore, Storage};
use tracing::info;
pub struct BitcoinCoordinatorStore {
    pub store: Rc<Storage>,
    pub max_unconfirmed_speedups: u32,
    pub retry_attempts_sending_tx: u32,
    pub retry_interval_seconds: u64,
}
enum StoreKey {
    PendingTransactionList,
    Transaction(Txid),
    DispatchTransactionErrorNewsList,
    DispatchSpeedUpErrorNewsList,
    InsufficientFundsNewsList,
    FundingNotFoundNews,
    EstimateFeerateTooHighNewsList,
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

    fn get_txs_to_dispatch(
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

    fn increment_tx_retry_count(&self, txid: Txid) -> Result<(), BitcoinCoordinatorStoreError>;
}

impl BitcoinCoordinatorStore {
    pub fn new(
        store: Rc<Storage>,
        max_unconfirmed_speedups: u32,
        retry_attempts_sending_tx: u32,
        retry_interval_seconds: u64,
    ) -> Result<Self, BitcoinCoordinatorStoreError> {
        Ok(Self {
            store,
            max_unconfirmed_speedups,
            retry_attempts_sending_tx,
            retry_interval_seconds,
        })
    }

    fn get_key(&self, key: StoreKey) -> String {
        let prefix = "bitcoin_coordinator";
        match key {
            StoreKey::PendingTransactionList => format!("{prefix}/tx/list"),
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
        }
    }

    fn get_txs(&self) -> Result<Vec<Txid>, BitcoinCoordinatorStoreError> {
        let key = self.get_key(StoreKey::PendingTransactionList);

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

        if tx.is_none() {
            let message = format!("Transaction not found: {tx_id}");
            return Err(BitcoinCoordinatorStoreError::TransactionNotFound(message));
        }

        Ok(tx.unwrap())
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
                || tx.state == TransactionState::Dispatched
                || tx.state == TransactionState::Confirmed
            {
                txs_filter.push(tx);
            }
        }

        Ok(txs_filter)
    }

    fn get_txs_to_dispatch(
        &self,
    ) -> Result<Vec<CoordinatedTransaction>, BitcoinCoordinatorStoreError> {
        let txs = self.get_txs()?;
        let mut txs_filter = Vec::new();

        for tx_id in txs {
            let tx = self.get_tx(&tx_id)?;

            if tx.state == TransactionState::ToDispatch {
                if tx.retry_info.is_none() {
                    txs_filter.push(tx);
                } else {
                    let retry_info = tx.retry_info.as_ref().unwrap();
                    if retry_info.retries_count < self.retry_attempts_sending_tx
                        && Utc::now().timestamp_millis() as u64 - retry_info.last_retry_timestamp
                            > self.retry_interval_seconds * 1000
                    {
                        txs_filter.push(tx);
                    }
                }
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

        let txs_key = self.get_key(StoreKey::PendingTransactionList);
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
        self.store.delete(&tx_key)?;

        let txs_key = self.get_key(StoreKey::PendingTransactionList);
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

        // Validate state transition: only ToDispatch can transition to Dispatched
        if tx.state != TransactionState::ToDispatch {
            return Err(BitcoinCoordinatorStoreError::InvalidTransactionState);
        }

        tx.state = TransactionState::Dispatched;

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
            (TransactionState::ToDispatch, TransactionState::Dispatched) => true,
            (TransactionState::ToDispatch, TransactionState::Failed) => true,
            (TransactionState::Dispatched, TransactionState::Confirmed) => true,
            (TransactionState::Confirmed, TransactionState::Finalized) => true,
            (current, new) if current == new => true,
            // Invalid transitions
            _ => false,
        };

        if !valid_transition {
            return Err(BitcoinCoordinatorStoreError::InvalidStateTransition(
                tx.state.clone(),
                new_state.clone(),
            ));
        }

        tx.state = new_state.clone();

        let key = self.get_key(StoreKey::Transaction(tx_id));
        self.store.set(key, tx, None)?;

        // Remove tx from the list if it is finalized
        if new_state == TransactionState::Finalized {
            let txs_key = self.get_key(StoreKey::PendingTransactionList);
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

                if is_new_news.is_none() {
                    // Insert news with current block hash and ack in false
                    news_list.push((tx_id, amount, required, (current_block_hash, false)));
                } else {
                    let pos = is_new_news.unwrap();
                    let (_, _, _, (existing_block_hash, _)) = &news_list[pos];

                    if existing_block_hash == &current_block_hash {
                        // We already have this news, do not update
                        return Ok(());
                    } else {
                        // Replace the notification if the block hash is different
                        news_list[pos] = (tx_id, amount, required, (current_block_hash, false));
                    }
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

                if is_new_news.is_none() {
                    // Insert news if it doesn't already exist
                    news_list.push((tx_id, context, error, (current_block_hash, false)));
                } else {
                    let pos = is_new_news.unwrap();
                    let (_, _, _, (last_block_hash, _)) = &news_list[pos];

                    if last_block_hash != &current_block_hash {
                        // Update the news if the block hash is different
                        news_list[pos] = (tx_id, context, error, (current_block_hash, false));
                    }
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

                if is_new_news.is_none() {
                    // Insert news if it doesn't already exist
                    news_list.push((tx_ids, contexts, txid, error, (current_block_hash, false)));
                } else {
                    let pos = is_new_news.unwrap();
                    let (_, _, _, _, (last_block_hash, _)) = &news_list[pos];

                    info!("last_block_hash: {:?} ", last_block_hash);
                    info!("current_block_hash: {:?} ", current_block_hash);
                    if last_block_hash != &current_block_hash {
                        // Update the news if the block hash is different
                        news_list[pos] =
                            (tx_ids, contexts, txid, error, (current_block_hash, false));
                    }
                }

                self.store.set(&key, &news_list, None)?;
            }
            CoordinatorNews::FundingNotFound => {
                let key = self.get_key(StoreKey::FundingNotFoundNews);
                let news = self.store.get::<&str, (BlockHash, bool)>(&key)?;

                // Check if there is no existing news for "FundingNotFound"
                if news.is_none() {
                    // If no existing news, set the current block hash and mark it as not acknowledged
                    self.store.set(&key, (current_block_hash, false), None)?;
                } else {
                    // If there is existing news, unpack the block hash and acknowledgment status
                    let (last_block_hash, _) = news.unwrap();
                    // If the existing block hash is different from the current one, update the store
                    if last_block_hash != current_block_hash {
                        self.store.set(&key, (current_block_hash, false), None)?;
                    }
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

                if is_new_news.is_none() {
                    // Insert news if it doesn't already exist
                    news_list.push((estimate_fee, max_allowed, (current_block_hash, false)));
                } else {
                    let pos = is_new_news.unwrap();
                    let (_, _, (last_block_hash, _)) = &news_list[pos];

                    if last_block_hash != &current_block_hash {
                        // Replace the notification if the block hash is different
                        news_list[pos] = (estimate_fee, max_allowed, (current_block_hash, false));
                    }
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

        Ok(all_news)
    }

    fn increment_tx_retry_count(&self, txid: Txid) -> Result<(), BitcoinCoordinatorStoreError> {
        let mut tx = self.get_tx(&txid)?;
        let new_count = tx.retry_info.clone().unwrap_or_default().retries_count + 1;

        if new_count >= self.retry_attempts_sending_tx {
            tx.state = TransactionState::Failed;
        } else {
            tx.retry_info = Some(RetryInfo::new(
                new_count,
                Utc::now().timestamp_millis() as u64,
            ));
        }

        self.store
            .set(self.get_key(StoreKey::Transaction(txid)), &tx, None)?;

        Ok(())
    }
}

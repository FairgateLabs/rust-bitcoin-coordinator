use crate::{
    errors::BitcoinCoordinatorStoreError,
    types::{AckCoordinatorNews, CoordinatedTransaction, CoordinatorNews, TransactionState},
};

use bitcoin::{Transaction, Txid};
use bitvmx_bitcoin_rpc::types::BlockHeight;
use protocol_builder::types::output::SpeedupData;
use std::rc::Rc;
use storage_backend::storage::{KeyValueStore, Storage};
pub struct BitcoinCoordinatorStore {
    pub store: Rc<Storage>,
    pub max_unconfirmed_speedups: u32,
}
enum StoreKey {
    PendingTransactionList,
    Transaction(Txid),

    DispatchTransactionErrorNewsList,
    DispatchSpeedUpErrorNewsList,
    InsufficientFundsNewsList,
    NewSpeedUpNewsList,
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

    fn add_news(&self, news: CoordinatorNews) -> Result<(), BitcoinCoordinatorStoreError>;
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
            StoreKey::NewSpeedUpNewsList => format!("{prefix}/news/new_speed_up"),
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
            let message = format!("Transaction not found: {}", tx_id);
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

    fn add_news(&self, news: CoordinatorNews) -> Result<(), BitcoinCoordinatorStoreError> {
        match news {
            CoordinatorNews::InsufficientFunds(tx_id, amount, required) => {
                let key = self.get_key(StoreKey::InsufficientFundsNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, u64, u64)>>(&key)?
                    .unwrap_or_default();
                news_list.push((tx_id, amount, required));
                self.store.set(&key, &news_list, None)?;
            }
            CoordinatorNews::NewSpeedUp(tx_id, context, counting) => {
                let key = self.get_key(StoreKey::NewSpeedUpNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, String, u32)>>(&key)?
                    .unwrap_or_default();
                news_list.push((tx_id, context, counting));
                self.store.set(&key, &news_list, None)?;
            }
            CoordinatorNews::DispatchTransactionError(tx_id, context, error) => {
                let key = self.get_key(StoreKey::DispatchTransactionErrorNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, String, String)>>(&key)?
                    .unwrap_or_default();
                news_list.push((tx_id, context, error));
                self.store.set(&key, &news_list, None)?;
            }
            CoordinatorNews::DispatchSpeedUpError(tx_ids, contexts, txid, error) => {
                let key = self.get_key(StoreKey::DispatchSpeedUpErrorNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Vec<Txid>, Vec<String>, Txid, String)>>(&key)?
                    .unwrap_or_default();
                news_list.push((tx_ids, contexts, txid, error));
                self.store.set(&key, &news_list, None)?;
            }
            CoordinatorNews::FundingNotFound => {
                let key = self.get_key(StoreKey::FundingNotFoundNews);
                self.store.set(&key, true, None)?;
            }
            CoordinatorNews::EstimateFeerateTooHigh(estimate_fee, max_allowed) => {
                let key = self.get_key(StoreKey::EstimateFeerateTooHighNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(u64, u64)>>(&key)?
                    .unwrap_or_default();
                news_list.push((estimate_fee, max_allowed));
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
                    .get::<&str, Vec<(Txid, u64, u64)>>(&key)?
                    .unwrap_or_default();
                news_list.retain(|(id, _, _)| *id != tx_id);
                self.store.set(&key, &news_list, None)?;
            }
            AckCoordinatorNews::NewSpeedUp(tx_id) => {
                let key = self.get_key(StoreKey::NewSpeedUpNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, String, u32)>>(&key)?
                    .unwrap_or_default();
                news_list.retain(|(id, _, _)| *id != tx_id);
                self.store.set(&key, &news_list, None)?;
            }
            AckCoordinatorNews::DispatchTransactionError(tx_id) => {
                let key = self.get_key(StoreKey::DispatchTransactionErrorNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, String, String)>>(&key)?
                    .unwrap_or_default();
                news_list.retain(|(id, _, _)| *id != tx_id);
                self.store.set(&key, &news_list, None)?;
            }
            AckCoordinatorNews::DispatchSpeedUpError(speedup_txid) => {
                let key = self.get_key(StoreKey::DispatchSpeedUpErrorNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Vec<Txid>, Vec<String>, Txid, String)>>(&key)?
                    .unwrap_or_default();
                news_list.retain(|(_, _, txid, _)| *txid != speedup_txid);
                self.store.set(&key, &news_list, None)?;
            }
            AckCoordinatorNews::EstimateFeerateTooHigh(estimate_fee, max_allowed) => {
                let key = self.get_key(StoreKey::EstimateFeerateTooHighNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(u64, u64)>>(&key)?
                    .unwrap_or_default();
                news_list.retain(|(fee, max)| *fee != estimate_fee || *max != max_allowed);
                self.store.set(&key, &news_list, None)?;
            }
            AckCoordinatorNews::FundingNotFound => {
                let key = self.get_key(StoreKey::FundingNotFoundNews);
                self.store.set(&key, false, None)?;
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
            .get::<&str, Vec<(Txid, u64, u64)>>(&insufficient_funds_key)?
        {
            for (txid, amount, required) in news_list {
                all_news.push(CoordinatorNews::InsufficientFunds(txid, amount, required));
            }
        }

        // Get speed up news
        let speed_up_key = self.get_key(StoreKey::NewSpeedUpNewsList);
        if let Some(news_list) = self
            .store
            .get::<&str, Vec<(Txid, String, u32)>>(&speed_up_key)?
        {
            for (tx_id, context, counting) in news_list {
                all_news.push(CoordinatorNews::NewSpeedUp(tx_id, context, counting));
            }
        }

        // Get dispatch error news
        let dispatch_error_key = self.get_key(StoreKey::DispatchTransactionErrorNewsList);
        if let Some(news_list) = self
            .store
            .get::<&str, Vec<(Txid, String, String)>>(&dispatch_error_key)?
        {
            for (tx_id, context, error) in news_list {
                all_news.push(CoordinatorNews::DispatchTransactionError(
                    tx_id, context, error,
                ));
            }
        }

        // Get speed up error news
        let speed_up_error_key = self.get_key(StoreKey::DispatchSpeedUpErrorNewsList);
        if let Some(news_list) = self
            .store
            .get::<&str, Vec<(Vec<Txid>, Vec<String>, Txid, String)>>(&speed_up_error_key)?
        {
            for (tx_ids, contexts, txid, error) in news_list {
                all_news.push(CoordinatorNews::DispatchSpeedUpError(
                    tx_ids, contexts, txid, error,
                ));
            }
        }

        // Get funding not found news
        let funding_not_found_key = self.get_key(StoreKey::FundingNotFoundNews);
        if let Some(not_found) = self.store.get::<&str, bool>(&funding_not_found_key)? {
            if not_found {
                all_news.push(CoordinatorNews::FundingNotFound);
            }
        }

        // Get estimate feerate too high news
        let estimate_feerate_too_high_key = self.get_key(StoreKey::EstimateFeerateTooHighNewsList);
        if let Some(news_list) = self
            .store
            .get::<&str, Vec<(u64, u64)>>(&estimate_feerate_too_high_key)?
        {
            for (estimate_fee, max_allowed) in news_list {
                all_news.push(CoordinatorNews::EstimateFeerateTooHigh(
                    estimate_fee,
                    max_allowed,
                ));
            }
        }

        Ok(all_news)
    }
}

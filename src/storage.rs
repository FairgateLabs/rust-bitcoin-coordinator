use crate::{
    errors::BitcoinCoordinatorStoreError,
    types::{
        AckCoordinatorNews, CoordinatedTransaction, CoordinatorNews, SpeedUpTx,
        TransactionDispatchState,
    },
};

use bitcoin::{Transaction, Txid};
use bitvmx_bitcoin_rpc::types::BlockHeight;
use mockall::automock;
use protocol_builder::types::Utxo;
use std::rc::Rc;
use storage_backend::storage::{KeyValueStore, Storage};
pub struct BitcoinCoordinatorStore {
    store: Rc<Storage>,
}
enum StoreKey {
    Transaction(Txid),
    TransactionList,
    FundingList,
    SpeedUpList,

    DispatchTransactionErrorNewsList,
    DispatchSpeedUpErrorNewsList,
    InsufficientFundsNewsList,
    NewSpeedUpNewsList,
}
#[automock]
pub trait BitcoinCoordinatorStoreApi {
    fn save_tx(
        &self,
        tx: Transaction,
        speedup: Option<Utxo>,
        target_block_height: Option<BlockHeight>,
        context: String,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn remove_tx(&self, tx_id: Txid) -> Result<(), BitcoinCoordinatorStoreError>;

    fn get_txs(
        &self,
        state: TransactionDispatchState,
    ) -> Result<Vec<CoordinatedTransaction>, BitcoinCoordinatorStoreError>;

    fn update_tx(
        &self,
        tx_id: Txid,
        status: TransactionDispatchState,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn update_tx_to_dispatched(
        &self,
        tx_id: Txid,
        deliver_block_height: u32,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn save_speedup_tx(&self, speed_up_tx: &SpeedUpTx) -> Result<(), BitcoinCoordinatorStoreError>;

    fn get_last_speedup(&self) -> Result<Option<SpeedUpTx>, BitcoinCoordinatorStoreError>;

    fn get_speedup_tx(&self, tx_id: &Txid) -> Result<SpeedUpTx, BitcoinCoordinatorStoreError>;

    fn get_funding(&self) -> Result<Option<Utxo>, BitcoinCoordinatorStoreError>;

    fn add_funding(&self, utxo: Utxo) -> Result<(), BitcoinCoordinatorStoreError>;

    fn remove_funding(&self, funding_tx_id: Txid) -> Result<(), BitcoinCoordinatorStoreError>;

    fn update_funding(&self, utxo: Utxo) -> Result<(), BitcoinCoordinatorStoreError>;

    fn add_news(&self, news: CoordinatorNews) -> Result<(), BitcoinCoordinatorStoreError>;
    fn ack_news(&self, news: AckCoordinatorNews) -> Result<(), BitcoinCoordinatorStoreError>;
    fn get_news(&self) -> Result<Vec<CoordinatorNews>, BitcoinCoordinatorStoreError>;
}

impl BitcoinCoordinatorStore {
    pub fn new(store: Rc<Storage>) -> Result<Self, BitcoinCoordinatorStoreError> {
        Ok(Self { store })
    }

    fn get_key(&self, key: StoreKey) -> String {
        let prefix = "bitcoin_coordinator";
        match key {
            StoreKey::TransactionList => format!("{prefix}/tx/list"),
            StoreKey::Transaction(tx_id) => format!("{prefix}/tx/{tx_id}"),
            StoreKey::FundingList => format!("{prefix}/tx/funding/txs/list"),
            StoreKey::SpeedUpList => format!("{prefix}/speedup/list"),

            //NEWS
            StoreKey::InsufficientFundsNewsList => format!("{prefix}/news/insufficient_funds"),
            StoreKey::DispatchTransactionErrorNewsList => {
                format!("{prefix}/news/dispatch_transaction_error")
            }
            StoreKey::DispatchSpeedUpErrorNewsList => {
                format!("{prefix}/news/dispatch_speed_up_error")
            }
            StoreKey::NewSpeedUpNewsList => format!("{prefix}/news/new_speed_up"),
        }
    }

    fn get_txs(&self) -> Result<Vec<Txid>, BitcoinCoordinatorStoreError> {
        let key = self.get_key(StoreKey::TransactionList);

        let all_txs = self.store.get::<&str, Vec<Txid>>(&key)?;

        match all_txs {
            Some(txs) => Ok(txs),
            None => Ok(vec![]),
        }
    }

    fn get_tx(&self, tx_id: Txid) -> Result<CoordinatedTransaction, BitcoinCoordinatorStoreError> {
        let key = self.get_key(StoreKey::Transaction(tx_id));
        let tx = self.store.get::<&str, CoordinatedTransaction>(&key)?;

        if tx.is_none() {
            let message = format!("Transaction not found: {}", tx_id);
            return Err(BitcoinCoordinatorStoreError::TransactionNotFound(message));
        }

        Ok(tx.unwrap())
    }
}

impl BitcoinCoordinatorStoreApi for BitcoinCoordinatorStore {
    fn get_txs(
        &self,
        state: TransactionDispatchState,
    ) -> Result<Vec<CoordinatedTransaction>, BitcoinCoordinatorStoreError> {
        let txs = self.get_txs()?;
        let mut txs_filter = Vec::new();

        for tx_id in txs {
            let tx = self.get_tx(tx_id)?;

            if tx.state == state {
                txs_filter.push(tx);
            }
        }

        Ok(txs_filter)
    }
    fn save_tx(
        &self,
        tx: Transaction,
        speedup_utxo: Option<Utxo>,
        target_block_height: Option<BlockHeight>,
        context: String,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        let key = self.get_key(StoreKey::Transaction(tx.compute_txid()));

        let tx_info = CoordinatedTransaction::new(
            tx.clone(),
            speedup_utxo,
            TransactionDispatchState::PendingDispatch,
            target_block_height,
            context,
        );

        self.store.set(&key, &tx_info, None)?;

        let txs_key = self.get_key(StoreKey::TransactionList);
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

        let txs_key = self.get_key(StoreKey::TransactionList);
        let mut txs = self
            .store
            .get::<&str, Vec<Txid>>(&txs_key)?
            .unwrap_or_default();

        txs.retain(|id| *id != tx_id);
        self.store.set(&txs_key, &txs, None)?;

        Ok(())
    }

    fn get_funding(&self) -> Result<Option<Utxo>, BitcoinCoordinatorStoreError> {
        let funding_txs_key = self.get_key(StoreKey::FundingList);

        let funding_txs = self
            .store
            .get::<&str, Vec<Utxo>>(&funding_txs_key)?
            .unwrap_or_default();

        if let Some(last_funding_tx) = funding_txs.last() {
            // Funding transaction is the last one.
            Ok(Some(last_funding_tx.clone()))
        } else {
            Ok(None)
        }
    }

    fn add_funding(&self, utxo: Utxo) -> Result<(), BitcoinCoordinatorStoreError> {
        let fundings_txs_key = self.get_key(StoreKey::FundingList);

        let mut funding_info = self
            .store
            .get::<&str, Vec<Utxo>>(&fundings_txs_key)?
            .unwrap_or_default();

        funding_info.push(utxo);

        self.store.set(&fundings_txs_key, &funding_info, None)?;

        Ok(())
    }

    fn remove_funding(&self, funding_tx_id: Txid) -> Result<(), BitcoinCoordinatorStoreError> {
        let fundings_txs_key = self.get_key(StoreKey::FundingList);

        let mut funding_txs = self
            .store
            .get::<&str, Vec<Utxo>>(&fundings_txs_key)?
            .unwrap_or_default();

        funding_txs.retain(|tx| tx.txid != funding_tx_id);

        self.store.set(&fundings_txs_key, &funding_txs, None)?;

        Ok(())
    }

    fn update_funding(&self, utxo: Utxo) -> Result<(), BitcoinCoordinatorStoreError> {
        let fundings_txs_key = self.get_key(StoreKey::FundingList);

        let mut funding_txs = self
            .store
            .get::<&str, Vec<Utxo>>(&fundings_txs_key)?
            .unwrap_or_default();

        // Check if the funding transaction already exists to avoid duplicates
        if funding_txs.iter().any(|tx| tx.txid == utxo.txid) {
            return Err(BitcoinCoordinatorStoreError::FundingTransactionAlreadyExists);
        }

        // Remove the existing funding transaction before adding the updated one
        funding_txs.retain(|tx| tx.txid != utxo.txid);
        funding_txs.push(utxo);

        self.store.set(&fundings_txs_key, &funding_txs, None)?;

        Ok(())
    }

    fn get_speedup_tx(&self, tx_id: &Txid) -> Result<SpeedUpTx, BitcoinCoordinatorStoreError> {
        let speed_up_tx_key = self.get_key(StoreKey::SpeedUpList);

        // Retrieve the list of speed up transactions from storage
        let speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)?
            .unwrap_or_default();

        // Find the specific speed up transaction that matches the given tx_id
        let speed_up_tx = speed_up_txs.into_iter().find(|t| t.tx_id == *tx_id);

        if speed_up_tx.is_none() {
            return Err(BitcoinCoordinatorStoreError::SpeedupNotFound);
        }

        Ok(speed_up_tx.unwrap())
    }

    fn get_last_speedup(&self) -> Result<Option<SpeedUpTx>, BitcoinCoordinatorStoreError> {
        let speed_up_tx_key = self.get_key(StoreKey::SpeedUpList);

        // Retrieve the list of speed up transactions from storage
        let speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)?
            .unwrap_or_default();

        // Get the last speed up transaction from the list
        let speed_up_tx = speed_up_txs.into_iter().last();

        Ok(speed_up_tx)
    }

    // This function adds a new speed up transaction to the list of speed up transactions associated with an child transaction.
    // Speed up transactions are stored in a list, with the most recent transaction added to the end of the list.
    // This design ensures that if the last transaction in the list is pending, there cannot be another pending speed up transaction
    // for the same group of transactions, except for one that is specifically related to the same child transaction.
    fn save_speedup_tx(&self, speed_up_tx: &SpeedUpTx) -> Result<(), BitcoinCoordinatorStoreError> {
        let speed_up_tx_key = self.get_key(StoreKey::SpeedUpList);

        let mut speed_up_txs = self
            .store
            .get::<&str, Vec<SpeedUpTx>>(&speed_up_tx_key)?
            .unwrap_or_default();

        speed_up_txs.push(speed_up_tx.clone());

        self.store.set(&speed_up_tx_key, &speed_up_txs, None)?;

        Ok(())
    }

    fn update_tx_to_dispatched(
        &self,
        tx_id: Txid,
        deliver_block_height: u32,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        //TODO: Implement transaction status transition validation to ensure the correct sequence:
        // Pending -> InProgress -> Completed, and in reorganization scenarios, do the reverse order.

        let mut tx = self.get_tx(tx_id)?;

        if tx.state != TransactionDispatchState::PendingDispatch {
            return Err(BitcoinCoordinatorStoreError::InvalidTransactionState);
        }

        tx.state = TransactionDispatchState::BroadcastPendingConfirmation;

        tx.broadcast_block_height = Some(deliver_block_height);

        let key = self.get_key(StoreKey::Transaction(tx_id));
        self.store.set(key, tx, None)?;

        Ok(())
    }

    fn update_tx(
        &self,
        tx_id: Txid,
        tx_state: TransactionDispatchState,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        //TODO: Implement transaction status transition validation to ensure the correct sequence:
        // PendingDispatch -> BroadcastPendingConfirmation -> Finalized, and in reorganization scenarios, do the reverse order.

        let mut tx = self.get_tx(tx_id)?;
        tx.state = tx_state.clone();

        let key = self.get_key(StoreKey::Transaction(tx_id));
        self.store.set(key, tx, None)?;

        Ok(())
    }

    fn add_news(&self, news: CoordinatorNews) -> Result<(), BitcoinCoordinatorStoreError> {
        match news {
            CoordinatorNews::InsufficientFunds(tx_id) => {
                let key = self.get_key(StoreKey::InsufficientFundsNewsList);
                let mut news_list = self.store.get::<&str, Vec<Txid>>(&key)?.unwrap_or_default();
                news_list.push(tx_id);
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
            CoordinatorNews::DispatchSpeedUpError(tx_ids, contexts, error) => {
                let key = self.get_key(StoreKey::DispatchSpeedUpErrorNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Vec<Txid>, Vec<String>, String)>>(&key)?
                    .unwrap_or_default();
                news_list.push((tx_ids, contexts, error));
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
                    .get::<&str, Vec<(Txid, String, Txid, String)>>(&key)?
                    .unwrap_or_default();
                news_list.retain(|(id, _, _, _)| *id != tx_id);
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
            AckCoordinatorNews::DispatchSpeedUpError(tx_id) => {
                let key = self.get_key(StoreKey::DispatchSpeedUpErrorNewsList);
                let mut news_list = self
                    .store
                    .get::<&str, Vec<(Txid, String, String)>>(&key)?
                    .unwrap_or_default();
                news_list.retain(|(id, _, _)| *id != tx_id);
                self.store.set(&key, &news_list, None)?;
            }
        }
        Ok(())
    }

    fn get_news(&self) -> Result<Vec<CoordinatorNews>, BitcoinCoordinatorStoreError> {
        let mut all_news = Vec::new();

        // Get insufficient funds news
        let insufficient_funds_key = self.get_key(StoreKey::InsufficientFundsNewsList);
        if let Some(news_list) = self.store.get::<&str, Vec<Txid>>(&insufficient_funds_key)? {
            for txid in news_list {
                all_news.push(CoordinatorNews::InsufficientFunds(txid));
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
            .get::<&str, Vec<(Vec<Txid>, Vec<String>, String)>>(&speed_up_error_key)?
        {
            for (tx_ids, contexts, error) in news_list {
                all_news.push(CoordinatorNews::DispatchSpeedUpError(
                    tx_ids, contexts, error,
                ));
            }
        }

        Ok(all_news)
    }
}

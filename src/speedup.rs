use crate::errors::BitcoinCoordinatorStoreError;
use crate::storage::BitcoinCoordinatorStore;
use crate::types::{CoordinatedSpeedUpTransaction, SpeedupState};
use bitcoin::Txid;
use protocol_builder::types::Utxo;
use storage_backend::storage::KeyValueStore;

pub trait SpeedupStore {
    fn add_funding(&self, funding: Utxo) -> Result<(), BitcoinCoordinatorStoreError>;

    fn get_funding(&self) -> Result<Option<Utxo>, BitcoinCoordinatorStoreError>;

    fn get_pending_speedups(
        &self,
    ) -> Result<Vec<CoordinatedSpeedUpTransaction>, BitcoinCoordinatorStoreError>;

    /// Saves a speedup transaction to the list of speedups.
    fn save_speedup(
        &self,
        speedup: CoordinatedSpeedUpTransaction,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    /// Gets a speedup transaction by its txid.
    fn get_speedup(
        &self,
        txid: &Txid,
    ) -> Result<CoordinatedSpeedUpTransaction, BitcoinCoordinatorStoreError>;

    /// Gets the list of speedups that have not been confirmed.
    fn can_speedup(&self) -> Result<bool, BitcoinCoordinatorStoreError>;

    // This function will return the last speedup if is necessary to replace it. Otherwise it will return None.
    fn get_speedup_to_replace(
        &self,
    ) -> Result<Option<CoordinatedSpeedUpTransaction>, BitcoinCoordinatorStoreError>;

    /// Updates the state of a speedup transaction (e.g., confirmed or finalized).
    fn update_speedup_state(
        &self,
        txid: Txid,
        state: SpeedupState,
    ) -> Result<(), BitcoinCoordinatorStoreError>;
}

enum SpeedupStoreKey {
    PendingSpeedUpList,
    SpeedUpTransaction(Txid),
}

impl SpeedupStoreKey {
    fn get_key(&self) -> String {
        let prefix = "bitcoin_coordinator";
        match self {
            SpeedupStoreKey::PendingSpeedUpList => format!("{prefix}/speedup/pending/list"),
            SpeedupStoreKey::SpeedUpTransaction(tx_id) => {
                format!("{prefix}/speedup/{tx_id}")
            }
        }
    }
}

impl SpeedupStore for BitcoinCoordinatorStore {
    fn add_funding(&self, funding: Utxo) -> Result<(), BitcoinCoordinatorStoreError> {
        // Every time we save a funding we don't care about the preious one. From now one every speed up is done with the new funding.
        const FUNDING_UTXO_CONTEXT: &str = "FUNDING_UTXO";

        let funding_to_speedup = CoordinatedSpeedUpTransaction::new(
            funding.txid,
            vec![],
            1.0,
            funding,
            false,
            // Given we are saving the funding, the broadcast block height is 0 for now.
            0,
            SpeedupState::Finalized,
            FUNDING_UTXO_CONTEXT.to_string(),
        );

        self.save_speedup(funding_to_speedup)?;

        Ok(())
    }

    fn get_funding(&self) -> Result<Option<Utxo>, BitcoinCoordinatorStoreError> {
        // In case there are no speedups we can't get the funding.

        // Funding comes from the last speedup transaction creted.
        // This method should trigger an error in case there are no replace speedups that is confirmed.
        // Se le va a hacer replace a la ultima transaccion speedup. Una vez que se haga un replace si sigue sin minarse ninguna transaccion
        // Use the StoreKey::PendingSpeedUpList to get the list of speedups
        let key = SpeedupStoreKey::PendingSpeedUpList.get_key();
        let speedup_ids: Vec<Txid> = self.store.get(&key)?.unwrap_or_default();

        if speedup_ids.is_empty() {
            return Ok(None);
        }

        let last_speedup_txid = speedup_ids.last().unwrap();

        let last_speedup = self.get_speedup(last_speedup_txid)?;

        if last_speedup.state != SpeedupState::Finalized {
            // Funding added manually are the funding tht always keep in this array.
            return Ok(Some(last_speedup.funding));
        }

        if !last_speedup.is_replace_speedup {
            // If there are no replace speedup means that we can keep chaining speedups.
            // Then the last one is the funding.
            return Ok(Some(last_speedup.funding));
        }

        // Last one is a Replace Speedup, it means that we can not chain speedups if there is not a confirmed replace speedup.
        if last_speedup.state == SpeedupState::Confirmed {
            // Means that we can use this as a funding.
            return Ok(Some(last_speedup.funding));
        }

        // Means there are other replace speedups that is confirmed.
        if last_speedup.state == SpeedupState::Dispatched {
            // Means there are other replace speedups that is confirmed.
            for txid in speedup_ids.iter().rev() {
                let speedup = self.get_speedup(txid)?;

                if speedup.state == SpeedupState::Dispatched {
                    continue;
                }

                if speedup.is_replace_speedup && speedup.state == SpeedupState::Confirmed {
                    return Ok(Some(speedup.funding));
                } else {
                    return Ok(None);
                }
            }
        }

        Ok(None)
    }

    fn get_pending_speedups(
        &self,
    ) -> Result<Vec<CoordinatedSpeedUpTransaction>, BitcoinCoordinatorStoreError> {
        let key = SpeedupStoreKey::PendingSpeedUpList.get_key();
        let speedups = self.store.get::<&str, Vec<Txid>>(&key)?.unwrap_or_default();

        let mut pending_speedups = Vec::new();

        for txid in speedups.iter().rev() {
            let speedup = self.get_speedup(txid)?;

            if speedup.state == SpeedupState::Finalized {
                // Up to here we don't need to go back more, this is like a checkpoint. In our case is the last funding tx added.
                break;
            }

            // If the speedup is not finalized, it means that it is still pending.
            pending_speedups.push(speedup);
        }

        Ok(pending_speedups)
    }

    fn can_speedup(&self) -> Result<bool, BitcoinCoordinatorStoreError> {
        let funding = self.get_funding()?;
        Ok(funding.is_some())
    }

    fn save_speedup(
        &self,
        speedup: CoordinatedSpeedUpTransaction,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        // Whenever a speedup is created, we add it to the list of pending speedups because is not finished.
        // Also speedup should be saved at the end of the list. Because is gonna be the new way to fund next speedups.

        let key = SpeedupStoreKey::PendingSpeedUpList.get_key();
        let mut speedups = self.store.get::<&str, Vec<Txid>>(&key)?.unwrap_or_default();
        speedups.push(speedup.tx_id);
        self.store.set(&key, &speedups, None)?;

        // Save speedup to get by id.
        let key = SpeedupStoreKey::SpeedUpTransaction(speedup.tx_id).get_key();
        self.store.set(&key, &speedup, None)?;

        Ok(())
    }

    fn get_speedup(
        &self,
        txid: &Txid,
    ) -> Result<CoordinatedSpeedUpTransaction, BitcoinCoordinatorStoreError> {
        let key = SpeedupStoreKey::SpeedUpTransaction(*txid).get_key();
        let speedup = self
            .store
            .get::<&str, CoordinatedSpeedUpTransaction>(&key)?
            .ok_or(BitcoinCoordinatorStoreError::SpeedupNotFound)?;

        Ok(speedup)
    }

    fn update_speedup_state(
        &self,
        txid: Txid,
        state: SpeedupState,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        if state == SpeedupState::Finalized {
            // Means that the speedup transaction was finalized.
            // Then we need to remove it from the pending list.
            let key = SpeedupStoreKey::PendingSpeedUpList.get_key();
            let mut speedups = self
                .store
                .get::<&str, Vec<Txid>>(&key)?
                .ok_or(BitcoinCoordinatorStoreError::SpeedupNotFound)?;

            speedups.remove(speedups.iter().position(|id| *id == txid).unwrap());

            self.store.set(&key, &speedups, None)?;
        }

        // Update the new state of the transaction in transaction by id.

        let key = SpeedupStoreKey::SpeedUpTransaction(txid).get_key();

        let mut speedup = self
            .store
            .get::<&str, CoordinatedSpeedUpTransaction>(&key)?
            .ok_or(BitcoinCoordinatorStoreError::SpeedupNotFound)?;

        speedup.state = state;

        self.store.set(&key, &speedup, None)?;

        Ok(())
    }

    fn get_speedup_to_replace(
        &self,
    ) -> Result<Option<CoordinatedSpeedUpTransaction>, BitcoinCoordinatorStoreError> {
        Ok(None)
    }
}

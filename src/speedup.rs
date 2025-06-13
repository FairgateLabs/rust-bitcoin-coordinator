use crate::constants::MAX_UNCONFIRMED_SPEEDUPS;
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

    fn has_reached_max_unconfirmed_speedups(&self) -> Result<bool, BitcoinCoordinatorStoreError>;
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
        // Attempt to determine the current funding UTXO by walking the speedup transaction history in reverse.
        // The funding UTXO is derived from the most recent speedup transaction that is either:
        //   - Finalized (serves as a checkpoint, i.e., a new funding insertion), or
        //   - Confirmed (regardless of whether it's a replace speedup), or
        //   - Not a replace speedup (i.e., a regular speedup, even if unconfirmed).
        //
        // If the latest speedup is an unconfirmed replace speedup, we must look further back for a confirmed replace speedup.
        // This prevents chaining unconfirmed replace speedups, ensuring only a confirmed replace speedup can serve as funding.
        //
        // If no suitable funding is found, return None.

        // If we have reached the max number of unconfirmed speedups, we are waiting for confirmations, then there is no funding available.
        if self.has_reached_max_unconfirmed_speedups()? {
            return Ok(None);
        }

        let key = SpeedupStoreKey::PendingSpeedUpList.get_key();
        let speedup_ids: Vec<Txid> = self.store.get(&key)?.unwrap_or_default();

        let mut should_be_a_replace = false;

        for txid in speedup_ids.iter().rev() {
            let speedup = self.get_speedup(txid)?;

            if !should_be_a_replace {
                if speedup.state == SpeedupState::Finalized
                    || speedup.state == SpeedupState::Confirmed
                {
                    return Ok(Some(speedup.funding));
                }

                if !speedup.is_replace_speedup {
                    // Encountered an unconfirmed speedup. We should
                    return Ok(Some(speedup.funding));
                }

                // Encountered an unconfirmed replace speedup; must look for a previous confirmed replace.
                should_be_a_replace = true;

                continue;
            }

            // We are searching for a previous confirmed replace speedup.
            if speedup.is_replace_speedup {
                if speedup.state == SpeedupState::Confirmed {
                    // Found a confirmed replace speedup; use as funding.
                    return Ok(Some(speedup.funding));
                }

                continue;
            }

            if speedup.state == SpeedupState::Confirmed {
                // Found a confirmed regular speedup; use as funding.
                return Ok(Some(speedup.funding));
            } else {
                // Found an unconfirmed regular speedup; cannot use as funding.
                // This current speedup is the responsable we get into a chain of replacements.
                return Ok(None);
            }
        }

        // No suitable funding found in the speedup history.
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
                return Ok(pending_speedups);
            }

            // If the speedup is not finalized, it means that it is still pending.
            pending_speedups.push(speedup);
        }

        pending_speedups.reverse();

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

    fn has_reached_max_unconfirmed_speedups(&self) -> Result<bool, BitcoinCoordinatorStoreError> {
        let key = SpeedupStoreKey::PendingSpeedUpList.get_key();
        let speedups = self.store.get::<&str, Vec<Txid>>(&key)?.unwrap_or_default();

        if speedups.len() == 0 {
            return Ok(false);
        }

        // sum up all consecutive unconfirmed speedups, and if sum is greater than MAX_UNCONFIRMED_SPEEDUPS, return true.
        let mut sum = 0;

        for txid in speedups.iter() {
            let speedup = self.get_speedup(txid)?;
            if speedup.state == SpeedupState::Dispatched {
                sum += 1;
            } else {
                break;
            }
        }

        Ok(sum >= MAX_UNCONFIRMED_SPEEDUPS)
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

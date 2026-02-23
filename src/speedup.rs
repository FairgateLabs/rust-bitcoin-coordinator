use crate::errors::BitcoinCoordinatorStoreError;
use crate::settings::{MAX_LIMIT_UNCONFIRMED_PARENTS, MIN_UNCONFIRMED_TXS_FOR_CPFP};
use crate::storage::BitcoinCoordinatorStore;
use crate::types::{CoordinatedSpeedUpTransaction, TransactionState};
use bitcoin::Txid;
use protocol_builder::types::Utxo;
use storage_backend::storage::KeyValueStore;

pub trait SpeedupStore {
    fn add_funding(&self, funding: Utxo) -> Result<(), BitcoinCoordinatorStoreError>;

    fn get_funding(&self) -> Result<Option<Utxo>, BitcoinCoordinatorStoreError>;

    fn get_active_speedups(
        &self,
    ) -> Result<Vec<CoordinatedSpeedUpTransaction>, BitcoinCoordinatorStoreError>;

    fn get_unconfirmed_speedups(
        &self,
    ) -> Result<Vec<CoordinatedSpeedUpTransaction>, BitcoinCoordinatorStoreError>;

    fn get_all_active_speedups(
        &self,
    ) -> Result<Vec<CoordinatedSpeedUpTransaction>, BitcoinCoordinatorStoreError>;

    fn save_speedup(
        &self,
        speedup: CoordinatedSpeedUpTransaction,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn get_speedup(
        &self,
        txid: &Txid,
    ) -> Result<CoordinatedSpeedUpTransaction, BitcoinCoordinatorStoreError>;

    fn is_funding_available(&self) -> Result<bool, BitcoinCoordinatorStoreError>;

    fn has_enough_unconfirmed_txs_for_cpfp(&self) -> Result<bool, BitcoinCoordinatorStoreError>;

    // This function will return the last speedup (CPFP) transaction to be bumped with RBF + the last replacement speedup.
    fn get_last_pending_speedup(
        &self,
    ) -> Result<
        Option<(
            CoordinatedSpeedUpTransaction,
            Option<CoordinatedSpeedUpTransaction>,
        )>,
        BitcoinCoordinatorStoreError,
    >;

    /// Updates the state of a speedup transaction (e.g., confirmed or finalized).
    fn update_speedup_state(
        &self,
        txid: Txid,
        state: TransactionState,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn has_reached_max_unconfirmed_speedups(&self) -> Result<bool, BitcoinCoordinatorStoreError>;

    fn get_available_unconfirmed_txs(&self) -> Result<u32, BitcoinCoordinatorStoreError>;
}

enum SpeedupStoreKey {
    ActiveSpeedUpList,
    // SpeedupInfo,
    SpeedUpTransaction(Txid),
}

impl SpeedupStoreKey {
    fn get_key(&self) -> String {
        let prefix = "bitcoin_coordinator";
        match self {
            SpeedupStoreKey::ActiveSpeedUpList => format!("{prefix}/speedup/active/list"),
            SpeedupStoreKey::SpeedUpTransaction(tx_id) => {
                format!("{prefix}/speedup/{tx_id}")
            }
        }
    }
}

impl SpeedupStore for BitcoinCoordinatorStore {
    fn add_funding(&self, next_funding: Utxo) -> Result<(), BitcoinCoordinatorStoreError> {
        // When saving a new funding UTXO, we ignore any previous funding.
        // From this point onward, next speedup transaction will use the new funding.
        // Since this is a new funding, there is no previous funding UTXO; we use the same UTXO for both previous and next funding fields to avoid introducing an Option type.
        // The broadcast block height is set to 0 and Finalized because funding should be confirmed on chain.
        let funding_to_speedup = CoordinatedSpeedUpTransaction::new(
            next_funding.txid,
            next_funding.clone(),
            next_funding,
            None, // Funding is not an RBF replacement
            0,
            TransactionState::Finalized,
            1.0,
            vec![],
            1,
        );

        self.save_speedup(funding_to_speedup)?;

        Ok(())
    }

    fn get_available_unconfirmed_txs(&self) -> Result<u32, BitcoinCoordinatorStoreError> {
        let speedups = self.get_all_active_speedups()?;

        let mut available_utxos = MAX_LIMIT_UNCONFIRMED_PARENTS;

        let mut is_rbf_active = false;

        for speedup in speedups.iter() {
            // In case there is a RBF at the top, we necessary need to find a confirmed RBF
            // to be able to fund otherwise there is no capacity for funding unconfirmed txs.
            if is_rbf_active && !speedup.is_replacing() {
                return Ok(0);
            }

            if speedup.state == TransactionState::Confirmed
                || speedup.state == TransactionState::Finalized
            {
                return Ok(available_utxos);
            }

            if speedup.is_replacing() && speedup.state == TransactionState::InMempool {
                is_rbf_active = true;
                continue;
            }

            if is_rbf_active && speedup.is_replacing() {
                return Ok(0);
            }

            let cpfp_tx = 1;
            let to_subtract = speedup.speedup_tx_data.len() as u32 + cpfp_tx;
            available_utxos = available_utxos.saturating_sub(to_subtract);
        }

        Ok(available_utxos)
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

        let speedups = self.get_all_active_speedups()?;

        let mut should_be_a_replace = false;

        for speedup in speedups.iter() {
            if !should_be_a_replace {
                if speedup.state == TransactionState::Finalized
                    || speedup.state == TransactionState::Confirmed
                {
                    // This is the last funding
                    return Ok(Some(speedup.next_funding.clone()));
                }

                if !speedup.is_replacing() {
                    //This is the case where is not a RBF, then we should use that.
                    return Ok(Some(speedup.next_funding.clone()));
                }

                // Encountered an unconfirmed replace speedup; must look for a previous confirmed replace.
                should_be_a_replace = true;

                continue;
            }

            // We are searching for a previous confirmed replace speedup.
            if speedup.is_replacing() {
                if speedup.state == TransactionState::Confirmed {
                    // Found a confirmed replace speedup; use as funding.
                    return Ok(Some(speedup.next_funding.clone()));
                }

                continue;
            }

            if speedup.state == TransactionState::Confirmed {
                // Found a confirmed regular speedup; use as funding.
                return Ok(Some(speedup.next_funding.clone()));
            } else {
                // Found an unconfirmed regular speedup; cannot use as funding.
                // This current speedup is responsible for getting into a chain of replacements.
                return Ok(None);
            }
        }

        // No suitable funding found in the speedup history.
        Ok(None)
    }

    // Returns the list of active speedups (InMempool, Error, Confirmed) until they are finalized.
    // Similar to get_active_transactions(), this includes speedups that are in progress.
    fn get_active_speedups(
        &self,
    ) -> Result<Vec<CoordinatedSpeedUpTransaction>, BitcoinCoordinatorStoreError> {
        let key = SpeedupStoreKey::ActiveSpeedUpList.get_key();
        let speedups = self.store.get::<&str, Vec<Txid>>(&key)?.unwrap_or_default();

        let mut active_speedups = Vec::new();

        for txid in speedups.iter().rev() {
            let speedup = self.get_speedup(txid)?;

            if speedup.state == TransactionState::Finalized {
                // Up to here we don't need to go back more, this is like a checkpoint.
                // In our case is the last funding tx added (Finalized)
                return Ok(active_speedups);
            }

            // Include speedups that are in progress (InMempool, Error, Confirmed)
            // Failed speedups are not active - they represent errors that cannot be retried
            if speedup.state != TransactionState::Finalized
                && speedup.state != TransactionState::Failed
            {
                active_speedups.push(speedup);
            }
        }

        active_speedups.reverse();

        Ok(active_speedups)
    }

    fn get_unconfirmed_speedups(
        &self,
    ) -> Result<Vec<CoordinatedSpeedUpTransaction>, BitcoinCoordinatorStoreError> {
        let key = SpeedupStoreKey::ActiveSpeedUpList.get_key();
        let speedups = self.store.get::<&str, Vec<Txid>>(&key)?.unwrap_or_default();

        let mut active_speedups = Vec::new();

        for txid in speedups.iter().rev() {
            let speedup = self.get_speedup(txid)?;

            if speedup.state == TransactionState::Confirmed
                || speedup.state == TransactionState::Finalized
            {
                // No need to check further; confirmed, finalized, or replaced speedup found.
                return Ok(active_speedups);
            }

            // If the speedup is not finalized or confirmed, it means that it is still unconfirmed.
            active_speedups.push(speedup);
        }

        Ok(active_speedups)
    }

    fn get_all_active_speedups(
        &self,
    ) -> Result<Vec<CoordinatedSpeedUpTransaction>, BitcoinCoordinatorStoreError> {
        let key = SpeedupStoreKey::ActiveSpeedUpList.get_key();
        let speedup_ids = self.store.get::<&str, Vec<Txid>>(&key)?.unwrap_or_default();

        let mut active_speedups = Vec::new();

        for txid in speedup_ids.iter() {
            let speedup = self.get_speedup(txid)?;
            active_speedups.push(speedup);
        }

        active_speedups.reverse();

        Ok(active_speedups)
    }

    fn is_funding_available(&self) -> Result<bool, BitcoinCoordinatorStoreError> {
        let funding = self.get_funding()?;
        let is_funding_available = funding.is_some();
        Ok(is_funding_available)
    }

    fn has_enough_unconfirmed_txs_for_cpfp(&self) -> Result<bool, BitcoinCoordinatorStoreError> {
        let available_unconfirmed_txs = self.get_available_unconfirmed_txs()?;
        let is_enough_unconfirmed_txs = available_unconfirmed_txs >= MIN_UNCONFIRMED_TXS_FOR_CPFP;
        Ok(is_enough_unconfirmed_txs)
    }

    fn save_speedup(
        &self,
        speedup: CoordinatedSpeedUpTransaction,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        // Whenever a speedup is created, we add it to the list of active speedups because is not finished.
        // Also speedup should be saved at the end of the list. Because is gonna be the new way to fund next speedups.
        // However, if the speedup already exists in the list (e.g., when updating replaced_by_tx_id),
        // we don't add it again to avoid duplicates.

        let key = SpeedupStoreKey::ActiveSpeedUpList.get_key();
        let mut speedups = self.store.get::<&str, Vec<Txid>>(&key)?.unwrap_or_default();

        // Only add to the list if it's not already present
        if !speedups.contains(&speedup.tx_id) {
            speedups.push(speedup.tx_id);
            self.store.set(&key, speedups, None)?;
        }

        // Save speedup to get by id.
        let key = SpeedupStoreKey::SpeedUpTransaction(speedup.tx_id).get_key();
        self.store.set(&key, speedup, None)?;

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
        let speedups = self.get_active_speedups()?;

        // sum up all consecutive unconfirmed speedups, and if sum is greater than MAX_UNCONFIRMED_SPEEDUPS, return true.
        let mut sum = 0;

        for speedup in speedups.iter() {
            if speedup.state == TransactionState::InMempool {
                sum += 1;
            } else {
                break;
            }
        }

        Ok(sum >= self.max_unconfirmed_speedups)
    }

    fn update_speedup_state(
        &self,
        txid: Txid,
        state: TransactionState,
    ) -> Result<(), BitcoinCoordinatorStoreError> {
        if state == TransactionState::Finalized {
            // Means that the speedup transaction was finalized or replaced.
            // Then we need to remove it from the active list.
            let key = SpeedupStoreKey::ActiveSpeedUpList.get_key();
            let mut speedups = self.store.get::<&str, Vec<Txid>>(&key)?.unwrap_or_default();

            let index = speedups
                .iter()
                .position(|id| *id == txid)
                .ok_or(BitcoinCoordinatorStoreError::SpeedupNotFound)?;

            // Iterate over all previous speedup transactions (before the current index)
            // to find any that have reached the Finalized or Replaced state and remove them from the active list.
            // This cleanup prevents the active speedup list from growing indefinitely with finalized/replaced entries.
            for (i, txid) in speedups[0..index].iter().enumerate() {
                let speedup_state = self.get_speedup(txid)?.state;
                if speedup_state == TransactionState::Finalized {
                    // If a finalized or replaced transaction is found, remove it from the list and update the store.
                    speedups.remove(i);
                    self.store.set(&key, &speedups, None)?;
                    break;
                }
            }
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

    fn get_last_pending_speedup(
        &self,
    ) -> Result<
        Option<(
            CoordinatedSpeedUpTransaction,
            Option<CoordinatedSpeedUpTransaction>,
        )>,
        BitcoinCoordinatorStoreError,
    > {
        let speedups = self.get_active_speedups()?;

        let mut last_rbf_tx = None;

        for speedup in speedups.iter() {
            if speedup.is_replacing() && speedup.state == TransactionState::InMempool {
                if last_rbf_tx.is_none() {
                    last_rbf_tx = Some(speedup.clone());
                }

                continue;
            }

            if speedup.state == TransactionState::Confirmed {
                // If the last speedup is confirmed, we don't need to replace it. It is already confirmed.
                return Ok(None);
            }

            return Ok(Some((speedup.clone(), last_rbf_tx)));
        }

        Ok(None)
    }
}

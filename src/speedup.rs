use crate::errors::BitcoinCoordinatorStoreError;
use crate::settings::{MAX_LIMIT_UNCONFIRMED_PARENTS, MIN_UNCONFIRMED_TXS_FOR_CPFP};
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

    fn get_unconfirmed_speedups(
        &self,
    ) -> Result<Vec<CoordinatedSpeedUpTransaction>, BitcoinCoordinatorStoreError>;

    fn get_all_pending_speedups(
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

    fn can_speedup(&self) -> Result<bool, BitcoinCoordinatorStoreError>;

    fn is_funding_available(&self) -> Result<bool, BitcoinCoordinatorStoreError>;

    // This function will return the last speedup (CPFP) transaction to be bumped with RBF + the last replacement speedup.
    fn get_last_speedup(
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
        state: SpeedupState,
    ) -> Result<(), BitcoinCoordinatorStoreError>;

    fn has_reached_max_unconfirmed_speedups(&self) -> Result<bool, BitcoinCoordinatorStoreError>;

    fn get_available_unconfirmed_txs(&self) -> Result<u32, BitcoinCoordinatorStoreError>;
}

enum SpeedupStoreKey {
    PendingSpeedUpList,
    // SpeedupInfo,
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
    fn add_funding(&self, next_funding: Utxo) -> Result<(), BitcoinCoordinatorStoreError> {
        // When saving a new funding UTXO, we ignore any previous funding.
        // From this point onward, next speedup transaction will use the new funding.
        // Since this is a new funding, there is no previous funding UTXO; we use the same UTXO for both previous and next funding fields to avoid introducing an Option type.
        // The broadcast block height is set to 0 and Finalized because funding should be confirmed on chain.
        let funding_to_speedup = CoordinatedSpeedUpTransaction::new(
            next_funding.txid,
            next_funding.clone(),
            next_funding,
            false,
            0,
            SpeedupState::Finalized,
            1.0,
            vec![],
            1,
        );

        self.save_speedup(funding_to_speedup)?;

        Ok(())
    }

    fn get_available_unconfirmed_txs(&self) -> Result<u32, BitcoinCoordinatorStoreError> {
        let speedups = self.get_all_pending_speedups()?;

        let mut available_utxos = MAX_LIMIT_UNCONFIRMED_PARENTS;

        let mut is_rbf_active = false;

        for speedup in speedups.iter() {
            // In case there is a RBF at the top, we necessary need to find a confirmed RBF
            // to be able to fund otherwise there is no capacity for funding unconfirmed txs.
            if is_rbf_active && !speedup.is_rbf {
                return Ok(0);
            }

            if speedup.state == SpeedupState::Confirmed || speedup.state == SpeedupState::Finalized
            {
                return Ok(available_utxos);
            }

            if speedup.is_rbf && speedup.state == SpeedupState::Dispatched {
                is_rbf_active = true;
                continue;
            }

            if is_rbf_active && speedup.is_rbf {
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

        let speedups = self.get_all_pending_speedups()?;

        let mut should_be_a_replace = false;

        for speedup in speedups.iter() {
            if !should_be_a_replace {
                if speedup.state == SpeedupState::Finalized
                    || speedup.state == SpeedupState::Confirmed
                {
                    return Ok(Some(speedup.next_funding.clone()));
                }

                if !speedup.is_rbf {
                    // Encountered an unconfirmed regular speedup. We can use this as funding.
                    return Ok(Some(speedup.next_funding.clone()));
                }

                // Encountered an unconfirmed replace speedup; must look for a previous confirmed replace.
                should_be_a_replace = true;

                continue;
            }

            // We are searching for a previous confirmed replace speedup.
            if speedup.is_rbf {
                if speedup.state == SpeedupState::Confirmed {
                    // Found a confirmed replace speedup; use as funding.
                    return Ok(Some(speedup.next_funding.clone()));
                }

                continue;
            }

            if speedup.state == SpeedupState::Confirmed {
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

    // Returns the list of pending speedups in reverse order until the last finalized speedup.
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

    fn get_unconfirmed_speedups(
        &self,
    ) -> Result<Vec<CoordinatedSpeedUpTransaction>, BitcoinCoordinatorStoreError> {
        let key = SpeedupStoreKey::PendingSpeedUpList.get_key();
        let speedups = self.store.get::<&str, Vec<Txid>>(&key)?.unwrap_or_default();

        let mut pending_speedups = Vec::new();

        for txid in speedups.iter().rev() {
            let speedup = self.get_speedup(txid)?;

            if speedup.state == SpeedupState::Confirmed || speedup.state == SpeedupState::Finalized
            {
                // No need to check further; confirmed or finalized speedup found.
                return Ok(pending_speedups);
            }

            // If the speedup is not finalized or confirmed, it means that it is still unconfirmed.
            pending_speedups.push(speedup);
        }

        Ok(pending_speedups)
    }

    fn get_all_pending_speedups(
        &self,
    ) -> Result<Vec<CoordinatedSpeedUpTransaction>, BitcoinCoordinatorStoreError> {
        let key = SpeedupStoreKey::PendingSpeedUpList.get_key();
        let speedup_ids = self.store.get::<&str, Vec<Txid>>(&key)?.unwrap_or_default();

        let mut pending_speedups = Vec::new();

        for txid in speedup_ids.iter() {
            let speedup = self.get_speedup(txid)?;
            pending_speedups.push(speedup);
        }

        pending_speedups.reverse();

        Ok(pending_speedups)
    }

    /// Determines if a speedup (CPFP) transaction can be created and dispatched.
    ///
    /// Returns `true` if:
    ///   - There is a funding transaction available to pay for the speedup.
    ///   - There are enough available unconfirmed transaction slots to satisfy Bitcoin's mempool chain limit policy.
    ///     (At least `MIN_UNCONFIRMED_TXS_FOR_CPFP` unconfirmed transactions are required: one for the CPFP itself and at least one unconfirmed output to spend.)
    fn can_speedup(&self) -> Result<bool, BitcoinCoordinatorStoreError> {
        let is_funding_available = self.is_funding_available()?;
        let available_unconfirmed_txs = self.get_available_unconfirmed_txs()?;
        let is_enough_unconfirmed_txs = available_unconfirmed_txs >= MIN_UNCONFIRMED_TXS_FOR_CPFP;

        Ok(is_funding_available && is_enough_unconfirmed_txs)
    }

    fn is_funding_available(&self) -> Result<bool, BitcoinCoordinatorStoreError> {
        let funding = self.get_funding()?;
        let is_funding_available = funding.is_some();
        Ok(is_funding_available)
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

        self.store.set(&key, speedups, None)?;

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
        let speedups = self.get_pending_speedups()?;

        // sum up all consecutive unconfirmed speedups, and if sum is greater than MAX_UNCONFIRMED_SPEEDUPS, return true.
        let mut sum = 0;

        for speedup in speedups.iter() {
            if speedup.state == SpeedupState::Dispatched {
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

            let index = speedups
                .iter()
                .position(|id| *id == txid)
                .ok_or(BitcoinCoordinatorStoreError::SpeedupNotFound)?;

            // Create a vector of speedup transactions that precede the current transaction in the list.
            let prev_speedups = speedups[0..index].to_vec();

            // Iterate over the previous speedup transactions in reverse order to find any finalized transaction.
            for (index, txid) in prev_speedups.iter().rev().enumerate() {
                if self.get_speedup(txid)?.state == SpeedupState::Finalized {
                    // If a finalized transaction is found, remove it from the list and update the store.
                    speedups.remove(index);
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

    fn get_last_speedup(
        &self,
    ) -> Result<
        Option<(
            CoordinatedSpeedUpTransaction,
            Option<CoordinatedSpeedUpTransaction>,
        )>,
        BitcoinCoordinatorStoreError,
    > {
        let speedups = self.get_pending_speedups()?;

        let mut last_rbf_tx = None;

        for speedup in speedups.iter() {
            if speedup.is_rbf && speedup.state == SpeedupState::Dispatched {
                if last_rbf_tx.is_none() {
                    last_rbf_tx = Some(speedup.clone());
                }

                continue;
            }

            if speedup.state == SpeedupState::Confirmed {
                // If the last speedup is confirmed, we don't need to replace it. It is already confirmed.
                return Ok(None);
            }

            return Ok(Some((speedup.clone(), last_rbf_tx)));
        }

        Ok(None)
    }
}

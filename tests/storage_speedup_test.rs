use bitcoin::{absolute::LockTime, transaction::Version, PublicKey, Transaction, Txid};
use bitcoin_coordinator::{
    errors::BitcoinCoordinatorStoreError,
    settings::MAX_LIMIT_UNCONFIRMED_PARENTS,
    speedup::SpeedupStore,
    types::{CoordinatedSpeedUpTransaction, SpeedupState},
};
use protocol_builder::types::{output::SpeedupData, Utxo};
use rand::Rng;
use std::str::FromStr;
use utils::clear_output;

use crate::utils::create_store;
mod utils;

fn dummy_utxo_with(txid: &Txid, vout: u32, sats: u64) -> Utxo {
    Utxo::new(
        *txid,
        vout,
        sats,
        &PublicKey::from_str("032e58afe51f9ed8ad3cc7897f634d881fdbe49a81564629ded8156bebd2ffd1af")
            .unwrap(),
    )
}

fn dummy_utxo(txid: &Txid) -> Utxo {
    dummy_utxo_with(txid, 0, 1000)
}

fn dummy_speedup_tx(
    txid: &Txid,
    state: SpeedupState,
    is_replace: bool,
    block_height: u32,
) -> CoordinatedSpeedUpTransaction {
    let tx_1 = generate_random_tx();
    let tx_2 = generate_random_tx();
    let tx_3 = generate_random_tx();

    let speedup_data_1 = SpeedupData::new(dummy_utxo(&tx_1.compute_txid()));
    let speedup_data_2 = SpeedupData::new(dummy_utxo(&tx_2.compute_txid()));
    let speedup_data_3 = SpeedupData::new(dummy_utxo(&tx_3.compute_txid()));

    CoordinatedSpeedUpTransaction::new(
        *txid,
        dummy_utxo(&txid),
        dummy_utxo(&txid),
        is_replace,
        block_height,
        state,
        0.0,
        vec![
            (speedup_data_1, tx_1, "Context 1".to_string()),
            (speedup_data_2, tx_2, "Context 2".to_string()),
            (speedup_data_3, tx_3, "Context 3".to_string()),
        ],
        1,
    )
}

fn generate_random_tx() -> Transaction {
    Transaction {
        version: Version::TWO,
        lock_time: random_locktime(),
        input: vec![],
        output: vec![],
    }
}

fn random_locktime() -> LockTime {
    let min_time = 500_000_000; // Earliest possible Unix time-based locktime
    let max_time = 2_000_000_000; // Some arbitrary future time (2033+)

    let random_time = rand::rng().random_range(min_time..=max_time);

    LockTime::from_time(random_time).unwrap()
}

#[test]
fn test_add_and_get_funding() -> Result<(), anyhow::Error> {
    let store = create_store();

    // No funding at first
    let funding = store.get_funding()?;
    assert!(funding.is_none());

    // Add funding
    let tx = generate_random_tx();
    let utxo = dummy_utxo(&tx.compute_txid());
    store.add_funding(utxo.clone())?;

    // Funding should now be present
    let funding2 = store.get_funding()?;
    assert!(funding2.is_some());
    assert_eq!(funding2.unwrap().txid, tx.compute_txid());

    // Add a new funding will replace the old one
    let tx2 = generate_random_tx();
    let utxo2 = dummy_utxo(&tx2.compute_txid());
    store.add_funding(utxo2.clone())?;

    // Funding should be the new one
    let funding3 = store.get_funding()?;
    assert!(funding3.is_some());
    assert_eq!(funding3.unwrap().txid, tx2.compute_txid());

    clear_output();
    Ok(())
}

#[test]
fn test_save_and_get_speedup() -> Result<(), anyhow::Error> {
    let store = create_store();

    // Save a speedup tx
    let tx = generate_random_tx();
    let speedup = dummy_speedup_tx(&tx.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.save_speedup(speedup.clone())?;

    // Get by id
    let fetched = store.get_speedup(&tx.compute_txid())?;
    assert_eq!(fetched.tx_id, tx.compute_txid());
    assert_eq!(fetched.state, SpeedupState::Dispatched);

    // Get pending speedups
    let pending = store.get_pending_speedups()?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].tx_id, tx.compute_txid());

    // can_speedup should be true (funding exists)
    assert!(store.can_speedup()?);

    clear_output();
    Ok(())
}

#[test]
fn test_pending_speedups_break_on_finalized() -> Result<(), anyhow::Error> {
    let store = create_store();

    // Add a finalized speedup (should act as checkpoint)
    let tx1 = generate_random_tx();
    let s1 = dummy_speedup_tx(&tx1.compute_txid(), SpeedupState::Confirmed, false, 0);
    store.save_speedup(s1.clone())?;

    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.save_speedup(s2.clone())?;

    // Only the last (pending) speedup should be returned, up to the finalized checkpoint
    let pending = store.get_pending_speedups()?;
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].tx_id, tx1.compute_txid());
    assert_eq!(pending[1].tx_id, tx2.compute_txid());

    // Insert a new speedup finalized, wich means that is a checkpoint.
    let tx3 = generate_random_tx();
    let s3 = dummy_speedup_tx(&tx3.compute_txid(), SpeedupState::Finalized, false, 0);
    store.save_speedup(s3.clone())?;

    let pending = store.get_pending_speedups()?;
    assert_eq!(pending.len(), 0);

    // Insert 10 speedups, and check that are 10 pending in total
    for i in 0..10 {
        let tx = generate_random_tx();
        let speedup = dummy_speedup_tx(
            &tx.compute_txid(),
            if i % 2 == 0 {
                SpeedupState::Confirmed
            } else {
                SpeedupState::Dispatched
            },
            false,
            0,
        );
        store.save_speedup(speedup)?;
    }

    let pending = store.get_pending_speedups()?;
    assert_eq!(pending.len(), 10);

    clear_output();
    Ok(())
}

#[test]
fn test_get_funding_with_replace_speedup_confirmed() -> Result<(), anyhow::Error> {
    let store = create_store();

    // Add a replace speedup, confirmed
    let tx1 = generate_random_tx();
    let speedup1 = dummy_speedup_tx(&tx1.compute_txid(), SpeedupState::Confirmed, true, 0);
    store.save_speedup(speedup1.clone())?;

    // Funding should be present
    let funding = store.get_funding()?;
    assert_eq!(funding.unwrap().txid, tx1.compute_txid());

    // Add speed replace unconfirmed and check that speed up is the previous one
    let tx2 = generate_random_tx();
    let speedup2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Dispatched, true, 0);
    store.save_speedup(speedup2.clone())?;

    let funding = store.get_funding()?;
    assert_eq!(funding.unwrap().txid, tx1.compute_txid());

    // Add 3 more speedups with replace unconfirmed and check that funding is the confirmed one
    for _ in 0..3 {
        let tx = generate_random_tx();
        let s = dummy_speedup_tx(&tx.compute_txid(), SpeedupState::Dispatched, true, 0);
        store.save_speedup(s.clone())?;
    }

    let funding = store.get_funding()?;
    assert_eq!(funding.unwrap().txid, tx1.compute_txid());

    clear_output();

    Ok(())
}

#[test]
fn test_get_funding_with_replace_speedup_dispatched_and_no_confirmed() -> Result<(), anyhow::Error>
{
    let store = create_store();

    // Add a replace speedup, dispatched
    let tx1 = generate_random_tx();
    let s1 = dummy_speedup_tx(&tx1.compute_txid(), SpeedupState::Dispatched, true, 0);
    store.save_speedup(s1.clone())?;

    // Add a replace speedup, dispatched (no confirmed in chain)
    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Dispatched, true, 0);
    store.save_speedup(s2.clone())?;

    let funding = store.get_funding()?;
    assert!(funding.is_none());

    clear_output();
    Ok(())
}

#[test]
fn test_can_speedup_none() -> Result<(), anyhow::Error> {
    let store = create_store();
    assert!(!store.can_speedup()?);

    // Add 10 dispatched speedups (none are finalized or confirmed)
    for _ in 0..10 {
        let tx = generate_random_tx();
        let s = dummy_speedup_tx(&tx.compute_txid(), SpeedupState::Dispatched, false, 0);
        store.save_speedup(s)?;
    }
    // After only dispatched speedups, can_speedup should still be false
    assert!(!store.can_speedup()?);
    clear_output();
    Ok(())
}

#[test]
fn test_update_speedup_state_and_remove_from_pending() -> Result<(), anyhow::Error> {
    let store = create_store();

    // Add a speedup tx
    let tx1 = generate_random_tx();
    let s = dummy_speedup_tx(&tx1.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.save_speedup(s.clone())?;

    // Update to Confirmed
    store.update_speedup_state(tx1.compute_txid(), SpeedupState::Confirmed)?;

    // Should not be in pending speedups
    let pending = store.get_pending_speedups()?;
    assert_eq!(pending.len(), 1);

    let funding = store.get_funding()?;
    assert!(funding.is_some());
    assert_eq!(funding.unwrap().txid, tx1.compute_txid());

    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.save_speedup(s2.clone())?;

    // Update to Confirmed
    store.update_speedup_state(tx2.compute_txid(), SpeedupState::Confirmed)?;

    // Should not be in pending speedups
    let pending = store.get_pending_speedups()?;
    assert_eq!(pending.len(), 2);

    let funding = store.get_funding()?;
    assert!(funding.is_some());
    assert_eq!(funding.unwrap().txid, tx2.compute_txid());

    // Update to Finalized
    store.update_speedup_state(tx1.compute_txid(), SpeedupState::Finalized)?;

    // Should not be in pending speedups
    let funding = store.get_funding()?;
    assert!(funding.is_some());
    assert_eq!(funding.unwrap().txid, tx2.compute_txid());

    // Update to Finalized
    store.update_speedup_state(tx2.compute_txid(), SpeedupState::Finalized)?;

    // Should not be in pending speedups
    let funding = store.get_funding()?;
    assert!(funding.is_some());
    assert_eq!(funding.unwrap().txid, tx2.compute_txid());

    // Should still be able to fetch by id, and state should be Finalized
    let fetched = store.get_speedup(&tx1.compute_txid())?;
    assert_eq!(fetched.state, SpeedupState::Finalized);

    // Should not be in pending speedups
    let fetched = store.get_speedup(&tx2.compute_txid())?;
    assert_eq!(fetched.state, SpeedupState::Finalized);

    // Add a speedup tx
    let tx3 = generate_random_tx();
    let s = dummy_speedup_tx(&tx3.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.save_speedup(s.clone())?;

    // Add a speedup tx
    let tx4 = generate_random_tx();
    let s = dummy_speedup_tx(&tx4.compute_txid(), SpeedupState::Confirmed, false, 0);
    store.save_speedup(s.clone())?;

    // Add a speedup tx
    let tx5: Transaction = generate_random_tx();
    let s = dummy_speedup_tx(&tx5.compute_txid(), SpeedupState::Confirmed, false, 0);
    store.save_speedup(s.clone())?;
    store.update_speedup_state(tx5.compute_txid(), SpeedupState::Finalized)?;

    // Only the Confirmed and last Finalized speedups should be returned, pending speedups comes
    // in reverse order.
    let all = store.get_all_pending_speedups()?;
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].state, SpeedupState::Finalized);
    assert_eq!(all[1].state, SpeedupState::Confirmed);
    assert_eq!(all[2].state, SpeedupState::Dispatched);
    assert_eq!(all[0].tx_id, tx5.compute_txid());
    assert_eq!(all[1].tx_id, tx4.compute_txid());
    assert_eq!(all[2].tx_id, tx3.compute_txid());

    clear_output();
    Ok(())
}

#[test]
fn test_update_speedup_state_not_found() -> Result<(), anyhow::Error> {
    let store = create_store();
    let tx = generate_random_tx();
    let res = store.update_speedup_state(tx.compute_txid(), SpeedupState::Finalized);
    assert!(matches!(
        res,
        Err(BitcoinCoordinatorStoreError::SpeedupNotFound)
    ));
    clear_output();
    Ok(())
}

#[test]
fn test_get_speedup_not_found() -> Result<(), anyhow::Error> {
    let store = create_store();
    let tx = generate_random_tx();
    let res = store.get_speedup(&tx.compute_txid());
    assert!(matches!(
        res,
        Err(BitcoinCoordinatorStoreError::SpeedupNotFound)
    ));
    clear_output();
    Ok(())
}

#[test]
fn test_save_speedup_overwrites() -> Result<(), anyhow::Error> {
    let store = create_store();
    let tx = generate_random_tx();
    let s1 = dummy_speedup_tx(&tx.compute_txid(), SpeedupState::Dispatched, false, 0);
    let mut s2 = s1.clone();
    s2.state = SpeedupState::Dispatched;
    // s2.block_height = 999;

    store.save_speedup(s1.clone())?;
    let fetched = store.get_speedup(&tx.compute_txid())?;
    assert_eq!(fetched.state, SpeedupState::Dispatched);

    // Overwrite
    store.save_speedup(s2.clone())?;
    let fetched2 = store.get_speedup(&tx.compute_txid())?;
    assert_eq!(fetched2.state, SpeedupState::Dispatched);
    // assert_eq!(fetched2.block_height, 999);

    clear_output();
    Ok(())
}

#[test]
fn test_get_unconfirmed_txs_count() -> Result<(), anyhow::Error> {
    let store = create_store();
    let tx = generate_random_tx();
    // It has 3 child txs.
    let max_unconfirmed_parents = MAX_LIMIT_UNCONFIRMED_PARENTS;

    let s = dummy_speedup_tx(&tx.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.save_speedup(s)?;

    let tx3 = generate_random_tx();
    let s3 = dummy_speedup_tx(&tx3.compute_txid(), SpeedupState::Confirmed, false, 0);
    store.save_speedup(s3)?;
    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    let coordinated_speedup_tx =
        dummy_speedup_tx(&tx.compute_txid(), SpeedupState::Dispatched, false, 0);
    let child_tx_ids = coordinated_speedup_tx.speedup_tx_data.len() as u32;
    store.save_speedup(coordinated_speedup_tx)?;

    let count = store.get_available_unconfirmed_txs()?;
    let mut count_to_validate = max_unconfirmed_parents - (child_tx_ids + 1);
    assert_eq!(count, count_to_validate);

    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    count_to_validate -= child_tx_ids + 1;
    assert_eq!(count, count_to_validate);

    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Confirmed, false, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents - (child_tx_ids + 1));

    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Dispatched, true, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, 0);

    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Confirmed, true, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Dispatched, true, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Dispatched, true, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Finalized, true, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Confirmed, true, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    clear_output();
    Ok(())
}

#[test]
fn test_get_speedups_for_retry() -> Result<(), anyhow::Error> {
    let store = create_store();
    let max_retries = 3;
    let interval_seconds = 2;

    // No speedups initially
    let speedups = store.get_speedups_for_retry(max_retries, interval_seconds)?;
    assert!(speedups.is_empty(), "Expected no speedups initially");

    // Add a speedup with retries less than max_retries
    let tx1 = generate_random_tx();
    let s1 = dummy_speedup_tx(&tx1.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.queue_speedup_for_retry(s1.clone())?;

    // Add a speedup with retries equal to max_retries
    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.queue_speedup_for_retry(s2.clone())?;

    // Add a speedup with 0 retries
    let tx3 = generate_random_tx();
    let s3 = dummy_speedup_tx(&tx3.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.queue_speedup_for_retry(s3.clone())?;

    std::thread::sleep(std::time::Duration::from_secs(1));
    // After 1 seconds, no speedups should be eligible for retry
    let speedups = store.get_speedups_for_retry(max_retries, interval_seconds)?;
    assert_eq!(
        speedups.len(),
        0,
        "Expected no speedups to be returned after 1 seconds"
    );

    std::thread::sleep(std::time::Duration::from_secs(1));

    // Add a speedup with 1 retry
    let tx4 = generate_random_tx();
    let s4 = dummy_speedup_tx(&tx4.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.queue_speedup_for_retry(s4.clone())?;

    // Add another speedup with retries equal to max_retries
    let tx5 = generate_random_tx();
    let s5 = dummy_speedup_tx(&tx5.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.queue_speedup_for_retry(s5.clone())?;

    // After a total of 2 seconds, the speedups with retries less than max_retries should be returned
    let speedups = store.get_speedups_for_retry(max_retries, interval_seconds)?;

    assert_eq!(
        speedups.len(),
        3,
        "Expected three speedups to be returned after 2 seconds"
    );
    assert!(
        speedups.iter().any(|s| s.tx_id == s1.tx_id),
        "Expected the first speedup to be returned"
    );
    assert!(
        speedups.iter().any(|s| s.tx_id == s2.tx_id),
        "Expected the third speedup to be returned"
    );
    assert!(
        speedups.iter().any(|s| s.tx_id == s3.tx_id),
        "Expected the fourth speedup to be returned"
    );

    std::thread::sleep(std::time::Duration::from_secs(2 * interval_seconds));
    let speedups = store.get_speedups_for_retry(max_retries, interval_seconds)?;
    assert_eq!(
        speedups.len(),
        5,
        "Expected five speedups to be returned after 4 seconds"
    );

    clear_output();
    Ok(())
}

#[test]
fn test_queue_and_enqueue_speedup_for_retry() -> Result<(), anyhow::Error> {
    let store = create_store();
    let interval_seconds = 1;

    // Add three speedups to the retry queue
    let tx1 = generate_random_tx();
    let s1 = dummy_speedup_tx(&tx1.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.queue_speedup_for_retry(s1.clone())?;

    let tx2 = generate_random_tx();
    let s2 = dummy_speedup_tx(&tx2.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.queue_speedup_for_retry(s2.clone())?;

    let tx3 = generate_random_tx();
    let s3 = dummy_speedup_tx(&tx3.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.queue_speedup_for_retry(s3.clone())?;

    // Wait for interval_seconds seconds to ensure the speedups are in the queue
    std::thread::sleep(std::time::Duration::from_secs(interval_seconds));
    // Verify all three are in the queue
    let speedups = store.get_speedups_for_retry(10, interval_seconds)?;
    assert_eq!(speedups.len(), 3, "Expected three speedups in the queue");
    assert!(
        speedups.iter().any(|s| s.tx_id == s1.tx_id),
        "Expected the first speedup to be in the queue"
    );
    assert!(
        speedups.iter().any(|s| s.tx_id == s2.tx_id),
        "Expected the second speedup to be in the queue"
    );
    assert!(
        speedups.iter().any(|s| s.tx_id == s3.tx_id),
        "Expected the third speedup to be in the queue"
    );

    // Enqueue (remove) the first speedup from the retry queue
    store.enqueue_speedup_for_retry(s1.tx_id)?;

    std::thread::sleep(std::time::Duration::from_secs(interval_seconds));
    // Verify the first speedup is no longer in the queue
    let speedups = store.get_speedups_for_retry(10, interval_seconds)?;
    assert_eq!(
        speedups.len(),
        2,
        "Expected two speedups in the queue after removing the first"
    );
    assert!(
        !speedups.iter().any(|s| s.tx_id == s1.tx_id),
        "Did not expect the first speedup to be in the queue"
    );

    // Enqueue (remove) the second speedup from the retry queue
    store.enqueue_speedup_for_retry(s2.tx_id)?;

    // Verify the second speedup is no longer in the queue
    let speedups = store.get_speedups_for_retry(10, interval_seconds)?;
    assert_eq!(
        speedups.len(),
        1,
        "Expected one speedup in the queue after removing the second"
    );
    assert!(
        !speedups.iter().any(|s| s.tx_id == s2.tx_id),
        "Did not expect the second speedup to be in the queue"
    );

    // Enqueue (remove) the third speedup from the retry queue
    store.enqueue_speedup_for_retry(s3.tx_id)?;

    // Verify the queue is empty
    let speedups = store.get_speedups_for_retry(10, interval_seconds)?;
    assert!(
        speedups.is_empty(),
        "Expected no speedups in the queue after removing all"
    );

    clear_output();
    Ok(())
}

#[test]
fn test_increment_speedup_retry_count() -> Result<(), anyhow::Error> {
    let store = create_store();
    let interval_seconds = 1;

    // Add a speedup to the retry queue
    let tx1 = generate_random_tx();
    let s1 = dummy_speedup_tx(&tx1.compute_txid(), SpeedupState::Dispatched, false, 0);
    store.queue_speedup_for_retry(s1.clone())?;

    // Increment the retry count
    store.increment_speedup_retry_count(s1.tx_id)?;

    // Wait for interval_seconds seconds to ensure the speedups are eligible for retry
    std::thread::sleep(std::time::Duration::from_secs(interval_seconds));

    // Verify the retry count has been incremented
    let speedups = store.get_speedups_for_retry(10, interval_seconds)?;
    assert_eq!(speedups.len(), 1, "Expected one speedup in the queue");

    assert_eq!(
        speedups[0].retry_info.clone().unwrap().retries_count,
        1,
        "Expected the retry count to be incremented"
    );

    // Increment the retry count three more times
    for _ in 0..3 {
        store.increment_speedup_retry_count(s1.tx_id)?;
    }

    // Wait for interval_seconds seconds to ensure the speedups are eligible for retry
    std::thread::sleep(std::time::Duration::from_secs(interval_seconds));

    // Verify the retry count has been incremented to 4
    let speedups = store.get_speedups_for_retry(10, interval_seconds)?;
    assert_eq!(speedups.len(), 1, "Expected one speedup in the queue");
    assert_eq!(
        speedups[0].retry_info.clone().unwrap().retries_count,
        4,
        "Expected the retry count to be incremented to 4"
    );

    // Attempt to increment the retry count for a non-existent transaction
    let non_existent_tx_id = generate_random_tx().compute_txid();
    let result = store.increment_speedup_retry_count(non_existent_tx_id);

    // Verify that incrementing a non-existent transaction does not cause an error
    assert!(
        result.is_ok(),
        "Expected no error when incrementing a non-existent transaction"
    );

    clear_output();
    Ok(())
}

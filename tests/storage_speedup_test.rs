use bitcoin::{
    hashes::{sha256d, Hash},
    PublicKey, Txid,
};
use bitcoin_coordinator::{
    errors::BitcoinCoordinatorStoreError,
    settings::MAX_LIMIT_UNCONFIRMED_PARENTS,
    speedup::SpeedupStore,
    types::{CoordinatedSpeedUpTransaction, SpeedupState},
};
use protocol_builder::types::Utxo;
use std::str::FromStr;
use utils::{clear_output, generate_random_string};

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
    let tx_id_2 = generate_random_txid();
    let tx_id_3 = generate_random_txid();
    let tx_id_4 = generate_random_txid();

    CoordinatedSpeedUpTransaction::new(
        *txid,
        vec![tx_id_2, tx_id_3, tx_id_4],
        dummy_utxo(&txid),
        dummy_utxo(&txid),
        is_replace,
        block_height,
        state,
        0.0,
        0,
    )
}

fn generate_random_txid() -> Txid {
    let str = generate_random_string();
    let str: &[u8] = str.as_bytes();
    Txid::from_slice(&sha256d::Hash::hash(str).to_byte_array()).unwrap()
}

#[test]
fn test_add_and_get_funding() -> Result<(), anyhow::Error> {
    let store = create_store();

    // No funding at first
    let funding = store.get_funding()?;
    assert!(funding.is_none());

    // Add funding
    let txid = generate_random_txid();
    let utxo = dummy_utxo(&txid);
    store.add_funding(utxo.clone())?;

    // Funding should now be present
    let funding2 = store.get_funding()?;
    assert!(funding2.is_some());
    assert_eq!(funding2.unwrap().txid, txid);

    // Add a new funding will replace the old one
    let txid2 = generate_random_txid();
    let utxo2 = dummy_utxo(&txid2);
    store.add_funding(utxo2.clone())?;

    // Funding should be the new one
    let funding3 = store.get_funding()?;
    assert!(funding3.is_some());
    assert_eq!(funding3.unwrap().txid, txid2);

    clear_output();
    Ok(())
}

#[test]
fn test_save_and_get_speedup() -> Result<(), anyhow::Error> {
    let store = create_store();

    // Save a speedup tx
    let txid = generate_random_txid();
    let speedup = dummy_speedup_tx(&txid, SpeedupState::Dispatched, false, 0);
    store.save_speedup(speedup.clone())?;

    // Get by id
    let fetched = store.get_speedup(&txid)?;
    assert_eq!(fetched.tx_id, txid);
    assert_eq!(fetched.state, SpeedupState::Dispatched);

    // Get pending speedups
    let pending = store.get_pending_speedups()?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].tx_id, txid);

    // can_speedup should be true (funding exists)
    assert!(store.can_speedup()?);

    clear_output();
    Ok(())
}

#[test]
fn test_pending_speedups_break_on_finalized() -> Result<(), anyhow::Error> {
    let store = create_store();

    // Add a finalized speedup (should act as checkpoint)
    let txid1 = generate_random_txid();
    let s1 = dummy_speedup_tx(&txid1, SpeedupState::Confirmed, false, 0);
    store.save_speedup(s1.clone())?;

    let txid2 = generate_random_txid();
    let s2 = dummy_speedup_tx(&txid2, SpeedupState::Dispatched, false, 0);
    store.save_speedup(s2.clone())?;

    // Only the last (pending) speedup should be returned, up to the finalized checkpoint
    let pending = store.get_pending_speedups()?;
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].tx_id, txid1);
    assert_eq!(pending[1].tx_id, txid2);

    // Insert a new speedup finalized, wich means that is a checkpoint.
    let txid3 = generate_random_txid();
    let s3 = dummy_speedup_tx(&txid3, SpeedupState::Finalized, false, 0);
    store.save_speedup(s3.clone())?;

    let pending = store.get_pending_speedups()?;
    assert_eq!(pending.len(), 0);

    // Insert 10 speedups, and check that are 10 pending in total
    for i in 0..10 {
        let txid = generate_random_txid();
        let speedup = dummy_speedup_tx(
            &txid,
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
    let txid1 = generate_random_txid();
    let speedup1 = dummy_speedup_tx(&txid1, SpeedupState::Confirmed, true, 0);
    store.save_speedup(speedup1.clone())?;

    // Funding should be present
    let funding = store.get_funding()?;
    assert_eq!(funding.unwrap().txid, txid1);

    // Add speed replace unconfirmed and check that speed up is the previous one
    let txid2 = generate_random_txid();
    let speedup2 = dummy_speedup_tx(&txid2, SpeedupState::Dispatched, true, 0);
    store.save_speedup(speedup2.clone())?;

    let funding = store.get_funding()?;
    assert_eq!(funding.unwrap().txid, txid1);

    // Add 3 more speedups with replace unconfirmed and check that funding is the confirmed one
    for _ in 0..3 {
        let txid = generate_random_txid();
        let s = dummy_speedup_tx(&txid, SpeedupState::Dispatched, true, 0);
        store.save_speedup(s.clone())?;
    }

    let funding = store.get_funding()?;
    assert_eq!(funding.unwrap().txid, txid1);

    clear_output();

    Ok(())
}

#[test]
fn test_get_funding_with_replace_speedup_dispatched_and_no_confirmed() -> Result<(), anyhow::Error>
{
    let store = create_store();

    // Add a replace speedup, dispatched
    let txid1 = generate_random_txid();
    let s1 = dummy_speedup_tx(&txid1, SpeedupState::Dispatched, true, 0);
    store.save_speedup(s1.clone())?;

    // Add a replace speedup, dispatched (no confirmed in chain)
    let txid2 = generate_random_txid();
    let s2 = dummy_speedup_tx(&txid2, SpeedupState::Dispatched, true, 0);
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
        let txid = generate_random_txid();
        let s = dummy_speedup_tx(&txid, SpeedupState::Dispatched, false, 0);
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
    let txid = generate_random_txid();
    let s = dummy_speedup_tx(&txid, SpeedupState::Dispatched, false, 0);
    store.save_speedup(s.clone())?;

    // Update to Finalized (should remove from pending list)
    store.update_speedup_state(txid, SpeedupState::Finalized)?;

    // Should not be in pending speedups
    let pending = store.get_pending_speedups()?;
    assert!(pending.is_empty());

    // Should still be able to fetch by id, and state should be Finalized
    let fetched = store.get_speedup(&txid)?;
    assert_eq!(fetched.state, SpeedupState::Finalized);

    clear_output();
    Ok(())
}

#[test]
fn test_update_speedup_state_not_found() -> Result<(), anyhow::Error> {
    let store = create_store();
    let txid = generate_random_txid();
    let res = store.update_speedup_state(txid, SpeedupState::Finalized);
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
    let txid = generate_random_txid();
    let res = store.get_speedup(&txid);
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
    let txid = generate_random_txid();
    let s1 = dummy_speedup_tx(&txid, SpeedupState::Dispatched, false, 0);
    let mut s2 = s1.clone();
    s2.state = SpeedupState::Dispatched;
    // s2.block_height = 999;

    store.save_speedup(s1.clone())?;
    let fetched = store.get_speedup(&txid)?;
    assert_eq!(fetched.state, SpeedupState::Dispatched);

    // Overwrite
    store.save_speedup(s2.clone())?;
    let fetched2 = store.get_speedup(&txid)?;
    assert_eq!(fetched2.state, SpeedupState::Dispatched);
    // assert_eq!(fetched2.block_height, 999);

    clear_output();
    Ok(())
}

#[test]
fn test_get_unconfirmed_txs_count() -> Result<(), anyhow::Error> {
    let store = create_store();
    let txid = generate_random_txid();
    // It has 3 child txs.
    let max_unconfirmed_parents = MAX_LIMIT_UNCONFIRMED_PARENTS;

    let s = dummy_speedup_tx(&txid, SpeedupState::Dispatched, false, 0);
    store.save_speedup(s)?;

    let txid3 = generate_random_txid();
    let s3 = dummy_speedup_tx(&txid3, SpeedupState::Confirmed, false, 0);
    store.save_speedup(s3)?;
    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    let s = dummy_speedup_tx(&txid, SpeedupState::Dispatched, false, 0);
    let child_tx_ids = s.child_tx_ids.len() as u32;
    store.save_speedup(s)?;

    let count = store.get_available_unconfirmed_txs()?;
    let mut count_to_validate = max_unconfirmed_parents - (child_tx_ids + 1);
    assert_eq!(count, count_to_validate);

    let txid2 = generate_random_txid();
    let s2 = dummy_speedup_tx(&txid2, SpeedupState::Dispatched, false, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    count_to_validate -= child_tx_ids + 1;
    assert_eq!(count, count_to_validate);

    let txid2 = generate_random_txid();
    let s2 = dummy_speedup_tx(&txid2, SpeedupState::Confirmed, false, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    let txid2 = generate_random_txid();
    let s2 = dummy_speedup_tx(&txid2, SpeedupState::Dispatched, false, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents - (child_tx_ids + 1));

    let txid2 = generate_random_txid();
    let s2 = dummy_speedup_tx(&txid2, SpeedupState::Dispatched, true, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, 0);

    let txid2 = generate_random_txid();
    let s2 = dummy_speedup_tx(&txid2, SpeedupState::Confirmed, true, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    let txid2 = generate_random_txid();
    let s2 = dummy_speedup_tx(&txid2, SpeedupState::Dispatched, true, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    let txid2 = generate_random_txid();
    let s2 = dummy_speedup_tx(&txid2, SpeedupState::Dispatched, true, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    let txid2 = generate_random_txid();
    let s2 = dummy_speedup_tx(&txid2, SpeedupState::Finalized, true, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    let txid2 = generate_random_txid();
    let s2 = dummy_speedup_tx(&txid2, SpeedupState::Confirmed, true, 0);
    store.save_speedup(s2)?;

    let count = store.get_available_unconfirmed_txs()?;
    assert_eq!(count, max_unconfirmed_parents);

    clear_output();
    Ok(())
}

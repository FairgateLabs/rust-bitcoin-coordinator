use bitcoin::{absolute::LockTime, hashes::Hash, Amount, PublicKey, Transaction, Txid};
use bitcoin_coordinator::{
    errors::BitcoinCoordinatorStoreError,
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::{SpeedUpTx, TransactionDispatchState},
};
use protocol_builder::types::Utxo;
use std::{rc::Rc, str::FromStr};
use storage_backend::{storage::Storage, storage_config::StorageConfig};
use utils::{clear_output, generate_random_string};
mod utils;

#[test]
fn test_save_and_get_tx() -> Result<(), anyhow::Error> {
    let storage_config = StorageConfig::new(
        format!("test_output/test/{}", generate_random_string()),
        None,
    );
    let storage = Rc::new(Storage::new(&storage_config)?);

    let store = BitcoinCoordinatorStore::new(storage)?;

    // Storage is empty, so all states should return empty vectors
    let empty_txs = store.get_txs(TransactionDispatchState::PendingDispatch)?;
    assert_eq!(empty_txs.len(), 0);

    let empty_sent_txs = store.get_txs(TransactionDispatchState::BroadcastPendingConfirmation)?;
    assert_eq!(empty_sent_txs.len(), 0);

    let empty_finalized_txs = store.get_txs(TransactionDispatchState::Finalized)?;
    assert_eq!(empty_finalized_txs.len(), 0);

    let tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195600).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx_id = tx.compute_txid();

    // Save transaction
    store.save_tx(tx.clone(), None, None, "context_tx".to_string())?;

    // Get transactions by state
    let txs = store.get_txs(TransactionDispatchState::PendingDispatch)?;
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].tx_id, tx_id);
    assert_eq!(txs[0].state, TransactionDispatchState::PendingDispatch);

    // Update transaction state
    store.update_tx(
        tx_id,
        TransactionDispatchState::BroadcastPendingConfirmation,
    )?;

    // Verify state was updated
    let sent_txs = store.get_txs(TransactionDispatchState::BroadcastPendingConfirmation)?;
    assert_eq!(sent_txs.len(), 1);
    assert_eq!(sent_txs[0].tx_id, tx_id);
    assert_eq!(
        sent_txs[0].state,
        TransactionDispatchState::BroadcastPendingConfirmation
    );

    // Verify no transactions in ReadyToSend state
    let ready_txs = store.get_txs(TransactionDispatchState::PendingDispatch)?;
    assert_eq!(ready_txs.len(), 0);

    // Update to confirmed state
    store.update_tx(tx_id, TransactionDispatchState::Finalized)?;

    // Verify state was updated
    let finalized_txs = store.get_txs(TransactionDispatchState::Finalized)?;
    assert_eq!(finalized_txs.len(), 1);
    assert_eq!(finalized_txs[0].tx_id, tx_id);
    assert_eq!(finalized_txs[0].state, TransactionDispatchState::Finalized);

    clear_output();

    Ok(())
}

#[test]
fn test_multiple_transactions() -> Result<(), anyhow::Error> {
    let storage_config = StorageConfig::new(
        format!("test_output/test/{}", generate_random_string()),
        None,
    );
    let storage = Rc::new(Storage::new(&storage_config)?);
    let store = BitcoinCoordinatorStore::new(storage)?;

    // Create a transaction
    let tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195600).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx_id = tx.compute_txid();

    // Save transaction
    store.save_tx(tx.clone(), None, None, "context_tx".to_string())?;

    // Test adding multiple transactions and verifying transaction list

    // Create additional transactions
    let tx2 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195700).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx3 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195800).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx2_id = tx2.compute_txid();
    let tx3_id = tx3.compute_txid();

    // Save additional transactions
    store.save_tx(tx2.clone(), None, None, "context_tx2".to_string())?;
    store.save_tx(tx3.clone(), None, None, "context_tx3".to_string())?;

    // Get all transactions in ReadyToSend state (should be all three)
    let ready_txs = store.get_txs(TransactionDispatchState::PendingDispatch)?;
    assert_eq!(ready_txs.len(), 3);

    // Verify all transactions are in the list
    let tx_ids: Vec<Txid> = ready_txs.iter().map(|tx| tx.tx_id).collect();
    assert!(tx_ids.contains(&tx_id));
    assert!(tx_ids.contains(&tx2_id));
    assert!(tx_ids.contains(&tx3_id));

    // Update states of transactions to different states
    store.update_tx(tx_id, TransactionDispatchState::Finalized)?;
    store.update_tx(
        tx2_id,
        TransactionDispatchState::BroadcastPendingConfirmation,
    )?;
    store.update_tx(tx3_id, TransactionDispatchState::Finalized)?;

    // Verify each state has the correct transactions
    let sent_txs = store.get_txs(TransactionDispatchState::BroadcastPendingConfirmation)?;
    assert_eq!(sent_txs.len(), 1);
    assert_eq!(sent_txs[0].tx_id, tx2_id);

    let finalized_txs = store.get_txs(TransactionDispatchState::Finalized)?;
    assert_eq!(finalized_txs.len(), 2);
    assert_eq!(finalized_txs[0].tx_id, tx_id);
    assert_eq!(finalized_txs[1].tx_id, tx3_id);
    // Verify no transactions in PendingDispatch state
    let ready_txs = store.get_txs(TransactionDispatchState::PendingDispatch)?;
    assert_eq!(ready_txs.len(), 0);

    clear_output();
    Ok(())
}

#[test]
fn test_speed_up_tx_operations() -> Result<(), anyhow::Error> {
    let storage_config = StorageConfig::new(
        format!("test_output/test/{}", generate_random_string()),
        None,
    );
    let storage = Rc::new(Storage::new(&storage_config)?);
    let store = BitcoinCoordinatorStore::new(storage)?;

    // Create a transaction to be used in the test
    let tx_to_speedup = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195600).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx_id = tx_to_speedup.compute_txid();

    // Save the transaction first
    store.save_tx(
        tx_to_speedup.clone(),
        None,
        None,
        "context_speedup".to_string(),
    )?;

    // Initially, there should be no speed-up transactions
    let speed_up_tx = store.get_last_speedup()?;
    assert!(speed_up_tx.is_none());

    let speed_up_tx_id =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200a").unwrap();

    let pub_key =
        PublicKey::from_str("032e58afe51f9ed8ad3cc7897f634d881fdbe49a81564629ded8156bebd2ffd1af")?;
    let funding_utxo = Utxo::new(tx_id, 0, Amount::ZERO.to_sat(), &pub_key);

    // Create and add a speed-up transaction
    let speed_up_tx = SpeedUpTx {
        tx_id: speed_up_tx_id,
        deliver_block_height: 100,
        child_tx_ids: vec![tx_id],
        utxo: funding_utxo.clone(),
    };

    // Add the speed-up transaction
    store.save_speedup_tx(&speed_up_tx)?;

    // Verify the speed-up transaction was added using get_last_speedup_tx
    let retrieved_speed_up_tx = store.get_last_speedup()?;
    assert!(retrieved_speed_up_tx.is_some());
    let retrieved = retrieved_speed_up_tx.unwrap();
    assert_eq!(retrieved.tx_id, speed_up_tx_id);

    // Test get_speedup_tx to retrieve a specific speed-up transaction
    let specific_speed_up = store.get_speedup_tx(&speed_up_tx_id)?;
    assert_eq!(specific_speed_up.tx_id, speed_up_tx_id);
    assert_eq!(specific_speed_up.child_tx_ids, vec![tx_id]);

    // Create another speed-up transaction with higher fee
    let second_speed_up_tx_id =
        Txid::from_str("f8b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200b").unwrap();
    let speed_up_tx2 = SpeedUpTx {
        tx_id: second_speed_up_tx_id,
        deliver_block_height: 110,
        child_tx_ids: vec![tx_id],
        utxo: funding_utxo,
    };

    // Add the second speed-up transaction using save_speedup_tx
    store.save_speedup_tx(&speed_up_tx2)?;

    // Test get_last_speedup_tx which should return the latest speed-up transaction
    let latest_speed_up = store.get_last_speedup()?;
    assert!(latest_speed_up.is_some());
    let latest = latest_speed_up.unwrap();
    assert_eq!(latest.tx_id, second_speed_up_tx_id);

    // Test get_speedup_tx with the first transaction
    let first_specific = store.get_speedup_tx(&speed_up_tx_id)?;
    assert_eq!(first_specific.tx_id, speed_up_tx_id);

    // Test get_speedup_tx with the second transaction
    let second_specific = store.get_speedup_tx(&second_speed_up_tx_id)?;
    assert_eq!(second_specific.tx_id, second_speed_up_tx_id);
    // Test with a non-existent transaction ID
    let non_existent_tx_id = Txid::from_slice(&[3; 32]).unwrap();
    let non_existent_speed_up = store.get_last_speedup()?;
    assert!(non_existent_speed_up.is_none());

    // Test get_speedup_tx with non-existent IDs
    let non_existent_specific = store.get_speedup_tx(&non_existent_tx_id);
    assert!(matches!(
        non_existent_specific,
        Err(BitcoinCoordinatorStoreError::SpeedupNotFound)
    ));

    clear_output();
    Ok(())
}

#[test]
fn test_funding_transactions() -> Result<(), anyhow::Error> {
    // Create a temporary directory for testing
    let storage_config = StorageConfig::new(
        format!("test_output/test/{}", generate_random_string()),
        None,
    );
    let storage = Rc::new(Storage::new(&storage_config)?);
    let coordinator = BitcoinCoordinatorStore::new(storage)?;

    let funding_txid_1 =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f2001")?;

    // Test that get_funding returns None for a transaction ID that hasn't been added yet
    let initial_funding = coordinator.get_funding()?;
    assert!(initial_funding.is_none());

    let funding_tx_id_2 =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f2002")?;

    // Create a funding transaction
    let funding_tx_utxo = Utxo::new(
        funding_txid_1,
        3,
        Amount::from_sat(10000).to_sat(),
        &PublicKey::from_str("032e58afe51f9ed8ad3cc7897f634d881fdbe49a81564629ded8156bebd2ffd1af")?,
    );

    coordinator.add_funding(funding_tx_utxo.clone())?;

    // Test get_funding
    let retrieved_funding = coordinator.get_funding()?;
    assert!(retrieved_funding.is_some());
    let utxo = retrieved_funding.unwrap();
    assert_eq!(utxo.txid, funding_txid_1);
    assert_eq!(utxo.vout, 3);
    assert_eq!(utxo.amount, funding_tx_utxo.amount);
    assert_eq!(utxo.pub_key, funding_tx_utxo.pub_key);

    // Test add_funding & Verify that the new funding is added
    let mut updated_funding = funding_tx_utxo.clone();
    updated_funding.vout = 1;
    updated_funding.txid = funding_tx_id_2;
    coordinator.add_funding(updated_funding)?;
    let updated_funding = coordinator.get_funding()?.unwrap();
    assert_eq!(updated_funding.vout, 1);
    assert_eq!(updated_funding.txid, funding_tx_id_2);

    // Check that funding_tx_id_2 is removed
    coordinator.remove_funding(funding_tx_id_2)?;
    let funding = coordinator.get_funding()?;
    assert!(funding.is_some());
    assert_eq!(funding.unwrap().txid, funding_txid_1);

    // Check that funding_tx_id_1 is removed
    coordinator.remove_funding(funding_txid_1)?;
    let funding = coordinator.get_funding()?;
    assert!(funding.is_none());

    // Clean up
    clear_output();
    Ok(())
}

#[test]
fn test_cancel_monitor() -> Result<(), anyhow::Error> {
    let storage_config = StorageConfig::new(
        format!(
            "test_output/test_cancel_monitor/{}",
            generate_random_string()
        ),
        None,
    );
    let storage = Rc::new(Storage::new(&storage_config)?);
    let coordinator = BitcoinCoordinatorStore::new(storage)?;
    // Create first transaction
    let tx1 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195600).unwrap(),
        input: vec![],
        output: vec![],
    };
    let tx_id_1 = tx1.compute_txid();

    // Create second transaction
    let tx2 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195700).unwrap(),
        input: vec![],
        output: vec![],
    };
    let tx_id_2 = tx2.compute_txid();

    // Save transaction to be monitored, this will be mark as pending dispatch
    coordinator.save_tx(tx1.clone(), None, None, "context_tx1".to_string())?;
    coordinator.save_tx(tx2.clone(), None, None, "context_tx2".to_string())?;

    // Remove one of the transactions
    coordinator.remove_tx(tx_id_1)?;
    let txs = coordinator.get_txs(TransactionDispatchState::PendingDispatch)?;
    assert_eq!(txs.len(), 1);

    // Remove the last transaction
    coordinator.remove_tx(tx_id_2)?;
    let txs = coordinator.get_txs(TransactionDispatchState::PendingDispatch)?;
    assert_eq!(txs.len(), 0);

    clear_output();

    Ok(())
}

#[test]
fn test_funding_tx_operations() -> Result<(), anyhow::Error> {
    let storage_config = StorageConfig::new(
        format!("test_output/test_funding/{}", generate_random_string()),
        None,
    );
    let storage = Rc::new(Storage::new(&storage_config)?);

    let store = BitcoinCoordinatorStore::new(storage)?;

    // Create a transaction that will need funding
    let tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195600).unwrap(),
        input: vec![],
        output: vec![],
    };
    let tx_id = tx.compute_txid();

    // Save the transaction
    store.save_tx(
        tx.clone(),
        None,
        None,
        "transaction_needing_funding".to_string(),
    )?;

    // Create a funding transaction
    let funding_tx = Utxo::new(
        Txid::from_str("000102030405060708090a0b0c0d0e0f000102030405060708090a0b0c0d0e0f").unwrap(),
        0,
        Amount::from_sat(100000).to_sat(),
        &PublicKey::from_str("032e58afe51f9ed8ad3cc7897f634d881fdbe49a81564629ded8156bebd2ffd1af")?,
    );

    // Initially, there should be no funding for the transaction
    let initial_funding = store.get_funding()?;
    assert!(initial_funding.is_none());

    // Add funding for the transaction
    store.add_funding(funding_tx.clone())?;

    // Verify funding was added
    let retrieved_funding = store.get_funding()?;
    assert!(retrieved_funding.is_some());
    let retrieved_tx = retrieved_funding.unwrap();
    assert_eq!(retrieved_tx.txid, funding_tx.txid);
    assert_eq!(retrieved_tx.vout, funding_tx.vout);
    assert_eq!(retrieved_tx.amount, funding_tx.amount);
    assert_eq!(retrieved_tx.pub_key, funding_tx.pub_key);

    // Remove the funding
    store.remove_funding(funding_tx.txid)?;

    // Verify funding was removed
    let removed_funding = store.get_funding()?;
    assert!(removed_funding.is_none());

    // Test adding funding for multiple transactions
    let tx2 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195700).unwrap(),
        input: vec![],
        output: vec![],
    };
    let tx2_id = tx2.compute_txid();

    let tx3 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195800).unwrap(),
        input: vec![],
        output: vec![],
    };
    let tx3_id = tx3.compute_txid();

    // Save the transactions
    store.save_tx(tx2.clone(), None, None, "second_tx".to_string())?;
    store.save_tx(tx3.clone(), None, None, "third_tx".to_string())?;

    // Create a new funding transaction
    let funding_tx2 = Utxo::new(
        Txid::from_str("1111111111111111111111111111111111111111111111111111111111111111").unwrap(),
        1,
        Amount::from_sat(200000).to_sat(),
        &PublicKey::from_str("032e58afe51f9ed8ad3cc7897f634d881fdbe49a81564629ded8156bebd2ffd1af")?,
    );

    // Add funding for multiple transactions
    store.add_funding(funding_tx2.clone())?;

    // Verify funding was added for all transactions
    for &id in &[tx_id, tx2_id, tx3_id] {
        let funding = store.get_funding()?;
        assert!(funding.is_some());
        let retrieved_tx = funding.unwrap();
        assert_eq!(retrieved_tx.txid, funding_tx2.txid);
        assert_eq!(retrieved_tx.vout, funding_tx2.vout);
        assert_eq!(retrieved_tx.amount, funding_tx2.amount);
    }

    // Test error handling when removing non-existent funding
    let random_txid =
        Txid::from_str("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff").unwrap();
    let result = store.remove_funding(random_txid);

    assert!(matches!(
        result,
        Err(BitcoinCoordinatorStoreError::FundingNotFound)
    ));

    // Test adding a second funding transaction (should replace the first one)
    let funding_tx3 = Utxo::new(
        Txid::from_str("2222222222222222222222222222222222222222222222222222222222222222").unwrap(),
        2,
        Amount::from_sat(300000).to_sat(),
        &PublicKey::from_str("032e58afe51f9ed8ad3cc7897f634d881fdbe49a81564629ded8156bebd2ffd1af")?,
    );

    store.add_funding(funding_tx3.clone())?;

    // Verify the funding was replaced
    let updated_funding = store.get_funding()?;
    assert!(updated_funding.is_some());
    let retrieved_tx = updated_funding.unwrap();
    assert_eq!(retrieved_tx.txid, funding_tx3.txid);
    assert_eq!(retrieved_tx.vout, funding_tx3.vout);
    assert_eq!(retrieved_tx.amount, funding_tx3.amount);

    // Test removing funding for one transaction doesn't affect others
    store.remove_funding(funding_tx2.txid)?;

    // tx2 should have no funding
    assert!(store.get_funding()?.is_none());

    // Clean up
    clear_output();

    Ok(())
}

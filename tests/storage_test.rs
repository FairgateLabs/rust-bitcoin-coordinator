use bitcoin::{absolute::LockTime, hashes::Hash, Amount, Transaction, Txid};
use bitcoin_coordinator::{
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::{FundingTransaction, SpeedUpTx, TransactionState},
};
use std::{path::PathBuf, rc::Rc, str::FromStr};
use storage_backend::storage::Storage;
use utils::{clear_output, generate_random_string};
mod utils;

#[test]
fn test_save_and_get_tx() -> Result<(), anyhow::Error> {
    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(format!(
        "test_output/test/{}",
        generate_random_string()
    )))?);

    let store = BitcoinCoordinatorStore::new(storage)?;

    // Storage is empty, so all states should return empty vectors
    let empty_txs = store.get_tx(TransactionState::ReadyToSend)?;
    assert_eq!(empty_txs.len(), 0);

    let empty_sent_txs = store.get_tx(TransactionState::Sent)?;
    assert_eq!(empty_sent_txs.len(), 0);

    let empty_confirmed_txs = store.get_tx(TransactionState::Confirmed)?;
    assert_eq!(empty_confirmed_txs.len(), 0);

    let empty_finalized_txs = store.get_tx(TransactionState::Finalized)?;
    assert_eq!(empty_finalized_txs.len(), 0);

    let tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195600).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx_id = tx.compute_txid();

    // Save transaction
    store.save_tx(tx.clone())?;

    // Get transactions by state
    let txs = store.get_tx(TransactionState::ReadyToSend)?;
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].tx_id, tx_id);
    assert_eq!(txs[0].state, TransactionState::ReadyToSend);

    // Update transaction state
    store.update_tx(tx_id, TransactionState::Sent)?;

    // Verify state was updated
    let sent_txs = store.get_tx(TransactionState::Sent)?;
    assert_eq!(sent_txs.len(), 1);
    assert_eq!(sent_txs[0].tx_id, tx_id);
    assert_eq!(sent_txs[0].state, TransactionState::Sent);

    // Verify no transactions in ReadyToSend state
    let ready_txs = store.get_tx(TransactionState::ReadyToSend)?;
    assert_eq!(ready_txs.len(), 0);

    // Update to confirmed state
    store.update_tx(tx_id, TransactionState::Confirmed)?;

    // Verify state was updated
    let confirmed_txs = store.get_tx(TransactionState::Confirmed)?;
    assert_eq!(confirmed_txs.len(), 1);
    assert_eq!(confirmed_txs[0].tx_id, tx_id);
    assert_eq!(confirmed_txs[0].state, TransactionState::Confirmed);

    // Update to finalized state
    store.update_tx(tx_id, TransactionState::Finalized)?;

    // Verify state was updated
    let finalized_txs = store.get_tx(TransactionState::Finalized)?;
    assert_eq!(finalized_txs.len(), 1);
    assert_eq!(finalized_txs[0].tx_id, tx_id);
    assert_eq!(finalized_txs[0].state, TransactionState::Finalized);

    clear_output();

    Ok(())
}

#[test]
fn test_multiple_transactions() -> Result<(), Box<dyn std::error::Error>> {
    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(format!(
        "test_output/test/{}",
        generate_random_string()
    )))?);
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
    store.save_tx(tx.clone())?;

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
    store.save_tx(tx2.clone())?;
    store.save_tx(tx3.clone())?;

    // Get all transactions in ReadyToSend state (should be all three)
    let ready_txs = store.get_tx(TransactionState::ReadyToSend)?;
    assert_eq!(ready_txs.len(), 3);

    // Verify all transactions are in the list
    let tx_ids: Vec<Txid> = ready_txs.iter().map(|tx| tx.tx_id).collect();
    assert!(tx_ids.contains(&tx_id));
    assert!(tx_ids.contains(&tx2_id));
    assert!(tx_ids.contains(&tx3_id));

    // Update states of transactions to different states
    store.update_tx(tx_id, TransactionState::Finalized)?;
    store.update_tx(tx2_id, TransactionState::Sent)?;
    store.update_tx(tx3_id, TransactionState::Confirmed)?;

    // Verify each state has the correct transactions
    let sent_txs = store.get_tx(TransactionState::Sent)?;
    assert_eq!(sent_txs.len(), 1);
    assert_eq!(sent_txs[0].tx_id, tx2_id);

    let confirmed_txs = store.get_tx(TransactionState::Confirmed)?;
    assert_eq!(confirmed_txs.len(), 1);
    assert_eq!(confirmed_txs[0].tx_id, tx3_id);

    let finalized_txs = store.get_tx(TransactionState::Finalized)?;
    assert_eq!(finalized_txs.len(), 1);
    assert_eq!(finalized_txs[0].tx_id, tx_id);

    // Verify no transactions in ReadyToSend state
    let ready_txs = store.get_tx(TransactionState::ReadyToSend)?;
    assert_eq!(ready_txs.len(), 0);

    clear_output();
    Ok(())
}

#[test]
fn test_speed_up_tx_operations() -> Result<(), Box<dyn std::error::Error>> {
    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(format!(
        "test_output/test/{}",
        generate_random_string()
    )))?);
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
    store.save_tx(tx_to_speedup.clone())?;

    // Initially, there should be no speed-up transactions
    let speed_up_tx = store.get_last_speedup_tx(&tx_id)?;
    println!("speed_up_tx: {:?}", speed_up_tx);
    assert!(speed_up_tx.is_none());

    let speed_up_tx_id =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200a").unwrap();
    // Create and add a speed-up transaction
    let speed_up_tx = SpeedUpTx {
        tx_id: speed_up_tx_id,
        deliver_block_height: 100,
        deliver_fee_rate: Amount::from_sat(1000),
        child_tx_id: tx_id,
        utxo_index: 0,
        utxo_output: bitcoin::TxOut {
            value: Amount::ZERO,
            script_pubkey: bitcoin::ScriptBuf::new(),
        },
    };

    // Add the speed-up transaction
    store.save_speedup_tx(&speed_up_tx)?;

    // Verify the speed-up transaction was added using get_last_speedup_tx
    let retrieved_speed_up_tx = store.get_last_speedup_tx(&tx_id)?;
    assert!(retrieved_speed_up_tx.is_some());
    let retrieved = retrieved_speed_up_tx.unwrap();
    assert_eq!(retrieved.tx_id, speed_up_tx_id);
    assert_eq!(retrieved.deliver_fee_rate, Amount::from_sat(1000));

    // Test get_speedup_tx to retrieve a specific speed-up transaction
    let specific_speed_up = store.get_speedup_tx(&speed_up_tx_id, &tx_id)?;
    assert!(specific_speed_up.is_some());
    let specific = specific_speed_up.unwrap();
    assert_eq!(specific.tx_id, speed_up_tx_id);
    assert_eq!(specific.child_tx_id, tx_id);
    assert_eq!(specific.deliver_fee_rate, Amount::from_sat(1000));

    // Create another speed-up transaction with higher fee
    let second_speed_up_tx_id =
        Txid::from_str("f8b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200b").unwrap();
    let speed_up_tx2 = SpeedUpTx {
        tx_id: second_speed_up_tx_id,
        deliver_block_height: 110,
        deliver_fee_rate: Amount::from_sat(1500),
        child_tx_id: tx_id,
        utxo_index: 1,
        utxo_output: bitcoin::TxOut {
            value: Amount::from_sat(5000),
            script_pubkey: bitcoin::ScriptBuf::new(),
        },
    };

    // Add the second speed-up transaction using save_speedup_tx
    store.save_speedup_tx(&speed_up_tx2)?;

    // Test get_last_speedup_tx which should return the latest speed-up transaction
    let latest_speed_up = store.get_last_speedup_tx(&tx_id)?;
    assert!(latest_speed_up.is_some());
    let latest = latest_speed_up.unwrap();
    assert_eq!(latest.tx_id, second_speed_up_tx_id);
    assert_eq!(latest.deliver_fee_rate, Amount::from_sat(1500));

    // Test get_speedup_tx with the first transaction
    let first_specific = store.get_speedup_tx(&speed_up_tx_id, &tx_id)?;
    assert!(first_specific.is_some());
    assert_eq!(
        first_specific.unwrap().deliver_fee_rate,
        Amount::from_sat(1000)
    );

    // Test get_speedup_tx with the second transaction
    let second_specific = store.get_speedup_tx(&second_speed_up_tx_id, &tx_id)?;
    assert!(second_specific.is_some());
    assert_eq!(
        second_specific.unwrap().deliver_fee_rate,
        Amount::from_sat(1500)
    );

    // Test with a non-existent transaction ID
    let non_existent_tx_id = Txid::from_slice(&[3; 32]).unwrap();
    let non_existent_speed_up = store.get_last_speedup_tx(&non_existent_tx_id)?;
    assert!(non_existent_speed_up.is_none());

    // Test get_speedup_tx with non-existent IDs
    let non_existent_specific = store.get_speedup_tx(&non_existent_tx_id, &tx_id)?;
    assert!(non_existent_specific.is_none());

    clear_output();
    Ok(())
}

#[test]
fn test_funding_transactions() -> Result<(), Box<dyn std::error::Error>> {
    clear_output();

    // Create a temporary directory for testing
    let store = Rc::new(Storage::new_with_path(&PathBuf::from(format!(
        "test_output/test/{}",
        generate_random_string()
    )))?);
    let bitcoin_store = BitcoinCoordinatorStore::new(store)?;

    let funding_tx_id_1 =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f2001")?;

    // Test that get_funding returns None for a transaction ID that hasn't been added yet
    let initial_funding = bitcoin_store.get_funding(funding_tx_id_1)?;
    assert!(initial_funding.is_none());

    // Test add_funding
    let tx_id_1 =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f2003")?;
    let tx_id_2 =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f2004")?;

    let funding_tx_id_2 =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f2002")?;

    // Create a funding transaction
    let funding_tx = FundingTransaction {
        tx_id: funding_tx_id_1,
        utxo_index: 3,
        utxo_output: bitcoin::TxOut {
            value: Amount::from_sat(10000),
            script_pubkey: bitcoin::ScriptBuf::new(),
        },
    };

    let tx_ids = vec![tx_id_1, tx_id_2];
    let context = "Test funding context".to_string();
    bitcoin_store.add_funding(tx_ids, funding_tx.clone(), context.clone())?;

    // Test get_funding
    let retrieved_funding = bitcoin_store.get_funding(tx_id_1)?;
    assert!(retrieved_funding.is_some());
    let retrieved = retrieved_funding.unwrap();
    assert_eq!(retrieved.tx_id, funding_tx_id_1);
    assert_eq!(retrieved.utxo_index, 3);
    assert_eq!(retrieved.utxo_output.value, Amount::from_sat(10000));

    // Test update_funding
    let mut updated_funding = funding_tx.clone();
    updated_funding.utxo_index = 1;
    updated_funding.tx_id = funding_tx_id_2;
    bitcoin_store.update_funding(tx_id_1, updated_funding)?;

    // Verify the update
    let updated_retrieved = bitcoin_store.get_funding(tx_id_1)?.unwrap();
    assert_eq!(updated_retrieved.utxo_index, 1);
    assert_eq!(updated_retrieved.tx_id, funding_tx_id_2);

    // Test remove_funding
    bitcoin_store.remove_funding(funding_tx_id_1, tx_id_1)?;

    // Verify the funding was removed
    let removed_funding = bitcoin_store.get_funding(funding_tx_id_1)?;
    assert!(removed_funding.is_none());

    // Test with non-existent transaction ID
    let non_existent_tx_id = Txid::from_slice(&[4; 32]).unwrap();
    let non_existent_funding = bitcoin_store.get_funding(non_existent_tx_id)?;
    assert!(non_existent_funding.is_none());

    // Clean up
    clear_output();
    Ok(())
}

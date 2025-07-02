use bitcoin::{absolute::LockTime, Transaction, Txid};
use bitcoin_coordinator::{
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::TransactionState,
};
use std::rc::Rc;
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

    let store = BitcoinCoordinatorStore::new(storage, 1)?;

    // Storage is empty, so all states should return empty vectors
    let empty_txs = store.get_txs_in_progress()?;
    assert_eq!(empty_txs.len(), 0);

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
    let txs = store.get_txs_in_progress()?;
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].tx_id, tx_id);
    assert_eq!(txs[0].state, TransactionState::ToDispatch);

    // Update transaction state
    store.update_tx_state(tx_id, TransactionState::Dispatched)?;

    // Verify no transactions in ReadyToSend state
    let ready_txs = store.get_txs_in_progress()?;
    assert_eq!(ready_txs.len(), 1);

    // Update to confirmed state
    store.update_tx_state(tx_id, TransactionState::Confirmed)?;
    store.update_tx_state(tx_id, TransactionState::Finalized)?;

    // Verify state was updated
    let finalized_txs = store.get_txs_in_progress()?;
    assert_eq!(finalized_txs.len(), 0);

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
    let store = BitcoinCoordinatorStore::new(storage, 1)?;

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
    let ready_txs = store.get_txs_in_progress()?;
    assert_eq!(ready_txs.len(), 3);

    // Verify all transactions are in the list
    let tx_ids: Vec<Txid> = ready_txs.iter().map(|tx| tx.tx_id).collect();
    assert!(tx_ids.contains(&tx_id));
    assert!(tx_ids.contains(&tx2_id));
    assert!(tx_ids.contains(&tx3_id));

    // Update states of transactions to different states
    store.update_tx_state(tx_id, TransactionState::Dispatched)?;
    store.update_tx_state(tx_id, TransactionState::Confirmed)?;
    store.update_tx_state(tx_id, TransactionState::Finalized)?;
    store.update_tx_state(tx2_id, TransactionState::Dispatched)?;
    store.update_tx_state(tx2_id, TransactionState::Confirmed)?;
    store.update_tx_state(tx3_id, TransactionState::Dispatched)?;
    store.update_tx_state(tx3_id, TransactionState::Confirmed)?;
    store.update_tx_state(tx3_id, TransactionState::Finalized)?;

    // Verify each state has the correct transactions
    let sent_txs = store.get_txs_in_progress()?;
    assert_eq!(sent_txs.len(), 1);
    assert_eq!(sent_txs[0].tx_id, tx2_id);

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
    let coordinator = BitcoinCoordinatorStore::new(storage, 1)?;
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
    let txs = coordinator.get_txs_in_progress()?;
    assert_eq!(txs.len(), 1);

    // Remove the last transaction
    coordinator.remove_tx(tx_id_2)?;
    let txs = coordinator.get_txs_in_progress()?;
    assert_eq!(txs.len(), 0);

    clear_output();

    Ok(())
}

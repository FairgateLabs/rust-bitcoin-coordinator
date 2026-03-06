use bitcoin::{absolute::LockTime, transaction::Version, BlockHash, Transaction, Txid};
use bitcoin_coordinator::{
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::{AckCoordinatorNews, CoordinatorNews, TransactionState},
};
use std::{rc::Rc, str::FromStr};
use storage_backend::{storage::Storage, storage_config::StorageConfig};
use utils::{clear_output, generate_random_string};
mod utils;

#[test]
fn coordinator_news_test() -> Result<(), anyhow::Error> {
    let path = format!(
        "test_output/coordinator_news_test/{}",
        generate_random_string()
    );

    let storage_config = StorageConfig::new(path, None);
    let storage = Rc::new(Storage::new(&storage_config)?);

    let current_block_hash =
        BlockHash::from_str("0000000000000000000000000000000000000000000000000000000000000000")
            .unwrap();

    let store = BitcoinCoordinatorStore::new(storage, 1)?;

    // Initially, there should be no news
    let news_list = store.get_news()?;
    assert_eq!(news_list.len(), 0);

    // Create test transaction IDs
    let tx_id_1 =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200a").unwrap();
    let tx_id_2 =
        Txid::from_str("f9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200b").unwrap();
    let tx_id_3 =
        Txid::from_str("f9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f2000").unwrap();

    // Add different types of news
    let insufficient_funds_news = CoordinatorNews::InsufficientFunds(tx_id_1, 1000, 2000);
    let speed_up_error_news = CoordinatorNews::DispatchSpeedUpError(
        vec![tx_id_2],
        vec!["tx_2".to_string()],
        tx_id_1,
        "error".to_string(),
    );

    let transaction_error_news =
        CoordinatorNews::DispatchTransactionError(tx_id_3, "tx_3".to_string(), "error".to_string());

    let estimate_feerate_news = CoordinatorNews::EstimateFeerateTooHigh(12345, 10000);

    let funding_not_found_news = CoordinatorNews::FundingNotFound;

    // Add news
    store.update_news(insufficient_funds_news.clone(), current_block_hash)?;
    store.update_news(speed_up_error_news.clone(), current_block_hash)?;
    store.update_news(transaction_error_news.clone(), current_block_hash)?;
    store.update_news(estimate_feerate_news.clone(), current_block_hash)?;
    store.update_news(funding_not_found_news.clone(), current_block_hash)?;

    // Get all news and verify
    let all_news = store.get_news()?;

    assert_eq!(all_news.len(), 5);
    assert!(all_news.contains(&insufficient_funds_news));
    assert!(all_news.contains(&transaction_error_news));
    assert!(all_news.contains(&speed_up_error_news));
    assert!(all_news.contains(&estimate_feerate_news));
    assert!(all_news.contains(&funding_not_found_news));

    // Acknowledge one news item
    let ack_news = AckCoordinatorNews::DispatchSpeedUpError(tx_id_1);
    store.ack_news(ack_news)?;

    // Verify the news was removed
    let remaining_news = store.get_news()?;
    assert_eq!(remaining_news.len(), 4);
    assert!(remaining_news.contains(&insufficient_funds_news));
    assert!(remaining_news.contains(&transaction_error_news));
    assert!(remaining_news.contains(&estimate_feerate_news));
    assert!(remaining_news.contains(&funding_not_found_news));
    assert!(!remaining_news.contains(&speed_up_error_news));

    //Acknowledge another news
    let ack_news = AckCoordinatorNews::InsufficientFunds(tx_id_1);
    store.ack_news(ack_news)?;
    // Verify the news was removed
    let remaining_news = store.get_news()?;
    assert_eq!(remaining_news.len(), 3);
    assert!(remaining_news.contains(&transaction_error_news));
    assert!(remaining_news.contains(&estimate_feerate_news));
    assert!(remaining_news.contains(&funding_not_found_news));
    assert!(!remaining_news.contains(&insufficient_funds_news));
    // Acknowledge the last news
    let ack_news = AckCoordinatorNews::DispatchTransactionError(tx_id_3);
    store.ack_news(ack_news)?;

    // Verify all news are removed except speed_up_news, estimate_feerate_news, and funding_not_found_news
    let remaining_news = store.get_news()?;
    assert_eq!(remaining_news.len(), 2);
    assert!(remaining_news.contains(&estimate_feerate_news));
    assert!(remaining_news.contains(&funding_not_found_news));
    // Acknowledge the last news

    // Acknowledge the EstimateFeerateTooHigh news
    let ack_news = AckCoordinatorNews::EstimateFeerateTooHigh(12345, 10000);
    store.ack_news(ack_news)?;
    // Verify only funding_not_found_news remains
    let remaining_news = store.get_news()?;
    assert_eq!(remaining_news.len(), 1);
    assert!(remaining_news.contains(&funding_not_found_news));
    // Acknowledge the FundingNotFound news
    // Since FundingNotFound does not take any argument, we need to add it to AckCoordinatorNews and implement its removal.
    // For now, let's assume AckCoordinatorNews::FundingNotFound exists.
    store.ack_news(AckCoordinatorNews::FundingNotFound)?;
    // Verify all news are removed
    let remaining_news = store.get_news()?;
    assert_eq!(remaining_news.len(), 0);

    // Add 2 news of each type (except FundingNotFound, which is a singleton)
    let tx_id_4 =
        Txid::from_str("4444444444444444444444444444444444444444444444444444444444444444").unwrap();
    let tx_id_5 =
        Txid::from_str("5555555555555555555555555555555555555555555555555555555555555555").unwrap();
    let tx_id_6 =
        Txid::from_str("6666666666666666666666666666666666666666666666666666666666666666").unwrap();
    let tx_id_7 =
        Txid::from_str("7777777777777777777777777777777777777777777777777777777777777777").unwrap();
    let tx_id_8 =
        Txid::from_str("8888888888888888888888888888888888888888888888888888888888888888").unwrap();

    // Create 2 news of each type
    let insufficient_funds_news_1 = CoordinatorNews::InsufficientFunds(tx_id_4, 1000, 2000);
    let insufficient_funds_news_2 = CoordinatorNews::InsufficientFunds(tx_id_5, 1000, 2000);

    let transaction_error_news_1 = CoordinatorNews::DispatchTransactionError(
        tx_id_6,
        "Test context 6".to_string(),
        "Test error 6".to_string(),
    );
    let transaction_error_news_2 = CoordinatorNews::DispatchTransactionError(
        tx_id_7,
        "Test context 7".to_string(),
        "Test error 7".to_string(),
    );

    let speed_up_error_news_1 = CoordinatorNews::DispatchSpeedUpError(
        vec![tx_id_6],
        vec!["Test context 6".to_string()],
        tx_id_6,
        "Test error 6".to_string(),
    );
    let speed_up_error_news_2 = CoordinatorNews::DispatchSpeedUpError(
        vec![tx_id_8],
        vec!["Test context 8".to_string()],
        tx_id_8,
        "Test error 8".to_string(),
    );

    let estimate_feerate_news_1 = CoordinatorNews::EstimateFeerateTooHigh(22222, 11111);
    let estimate_feerate_news_2 = CoordinatorNews::EstimateFeerateTooHigh(33333, 22222);

    let funding_not_found_news = CoordinatorNews::FundingNotFound;

    let next_block_hash =
        BlockHash::from_str("1111111111111111111111111111111111111111111111111111111111111111")
            .unwrap();
    // Add all news
    store.update_news(insufficient_funds_news_1.clone(), current_block_hash)?;
    store.update_news(insufficient_funds_news_2.clone(), current_block_hash)?;
    store.update_news(transaction_error_news_1.clone(), current_block_hash)?;
    store.update_news(transaction_error_news_2.clone(), current_block_hash)?;
    store.update_news(speed_up_error_news_1.clone(), current_block_hash)?;
    store.update_news(speed_up_error_news_2.clone(), current_block_hash)?;
    store.update_news(estimate_feerate_news_1.clone(), current_block_hash)?;
    store.update_news(estimate_feerate_news_2.clone(), current_block_hash)?;
    store.update_news(funding_not_found_news.clone(), current_block_hash)?;
    store.update_news(funding_not_found_news.clone(), next_block_hash)?;

    // Verify all news were added
    let all_news = store.get_news()?;
    assert_eq!(all_news.len(), 9);
    assert!(all_news.contains(&insufficient_funds_news_1));
    assert!(all_news.contains(&insufficient_funds_news_2));
    assert!(all_news.contains(&transaction_error_news_1));
    assert!(all_news.contains(&transaction_error_news_2));
    assert!(all_news.contains(&speed_up_error_news_1));
    assert!(all_news.contains(&speed_up_error_news_2));
    assert!(all_news.contains(&estimate_feerate_news_1));
    assert!(all_news.contains(&estimate_feerate_news_2));
    assert!(all_news.contains(&funding_not_found_news));

    // Acknowledge all news
    store.ack_news(AckCoordinatorNews::InsufficientFunds(tx_id_4))?;
    store.ack_news(AckCoordinatorNews::InsufficientFunds(tx_id_5))?;
    store.ack_news(AckCoordinatorNews::DispatchTransactionError(tx_id_6))?;
    store.ack_news(AckCoordinatorNews::DispatchTransactionError(tx_id_7))?;
    store.ack_news(AckCoordinatorNews::DispatchSpeedUpError(tx_id_6))?;
    store.ack_news(AckCoordinatorNews::DispatchSpeedUpError(tx_id_8))?;
    store.ack_news(AckCoordinatorNews::EstimateFeerateTooHigh(22222, 11111))?;
    store.ack_news(AckCoordinatorNews::EstimateFeerateTooHigh(33333, 22222))?;
    store.ack_news(AckCoordinatorNews::FundingNotFound)?;

    // Verify all news were removed
    let remaining_news = store.get_news()?;
    assert_eq!(remaining_news.len(), 0);

    clear_output();
    Ok(())
}

#[test]
fn test_transaction_already_in_mempool_news() -> Result<(), anyhow::Error> {
    let path = format!("test_output/storage_news_test/{}", generate_random_string());

    let storage_config = StorageConfig::new(path, None);
    let storage = Rc::new(Storage::new(&storage_config)?);

    let current_block_hash =
        BlockHash::from_str("0000000000000000000000000000000000000000000000000000000000000000")
            .unwrap();

    let store = BitcoinCoordinatorStore::new(storage, 1)?;

    let tx_id =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200a").unwrap();
    let context = "test_context".to_string();

    // Initially, there should be no news
    let news_list = store.get_news()?;
    assert_eq!(news_list.len(), 0);

    // Add TransactionAlreadyInMempool news
    let news = CoordinatorNews::TransactionAlreadyInMempool(tx_id, context.clone());
    store.update_news(news, current_block_hash)?;

    // Verify the news is stored
    let news_list = store.get_news()?;
    assert_eq!(news_list.len(), 1);
    match &news_list[0] {
        CoordinatorNews::TransactionAlreadyInMempool(id, ctx) => {
            assert_eq!(*id, tx_id);
            assert_eq!(ctx, &context);
        }
        _ => panic!("Expected TransactionAlreadyInMempool news"),
    }

    // Acknowledge the news
    store.ack_news(AckCoordinatorNews::TransactionAlreadyInMempool(tx_id))?;

    // Verify the news is acknowledged and no longer returned
    let news_list = store.get_news()?;
    assert_eq!(news_list.len(), 0);

    clear_output();
    Ok(())
}

#[test]
fn test_mempool_rejection_news() -> Result<(), anyhow::Error> {
    let path = format!("test_output/storage_news_test/{}", generate_random_string());

    let storage_config = StorageConfig::new(path, None);
    let storage = Rc::new(Storage::new(&storage_config)?);

    let current_block_hash =
        BlockHash::from_str("0000000000000000000000000000000000000000000000000000000000000000")
            .unwrap();

    let store = BitcoinCoordinatorStore::new(storage, 1)?;

    let tx_id =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200a").unwrap();
    let context = "test_context".to_string();
    let error_msg = "mempool full".to_string();

    // Add MempoolRejection news
    let news = CoordinatorNews::MempoolRejection(tx_id, context.clone(), error_msg.clone());
    store.update_news(news, current_block_hash)?;

    // Verify the news is stored
    let news_list = store.get_news()?;
    assert_eq!(news_list.len(), 1);
    match &news_list[0] {
        CoordinatorNews::MempoolRejection(id, ctx, err) => {
            assert_eq!(*id, tx_id);
            assert_eq!(ctx, &context);
            assert_eq!(err, &error_msg);
        }
        _ => panic!("Expected MempoolRejection news"),
    }

    // Acknowledge the news
    store.ack_news(AckCoordinatorNews::MempoolRejection(tx_id))?;

    // Verify the news is acknowledged
    let news_list = store.get_news()?;
    assert_eq!(news_list.len(), 0);

    clear_output();
    Ok(())
}

#[test]
fn test_network_error_news() -> Result<(), anyhow::Error> {
    let path = format!("test_output/storage_news_test/{}", generate_random_string());

    let storage_config = StorageConfig::new(path, None);
    let storage = Rc::new(Storage::new(&storage_config)?);

    let current_block_hash =
        BlockHash::from_str("0000000000000000000000000000000000000000000000000000000000000000")
            .unwrap();

    let store = BitcoinCoordinatorStore::new(storage, 1)?;

    let tx_id =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200a").unwrap();
    let context = "test_context".to_string();
    let error_msg = "network connection timeout".to_string();

    // Add NetworkError news
    let news = CoordinatorNews::NetworkError(tx_id, context.clone(), error_msg.clone());
    store.update_news(news, current_block_hash)?;

    // Verify the news is stored
    let news_list = store.get_news()?;
    assert_eq!(news_list.len(), 1);
    match &news_list[0] {
        CoordinatorNews::NetworkError(id, ctx, err) => {
            assert_eq!(*id, tx_id);
            assert_eq!(ctx, &context);
            assert_eq!(err, &error_msg);
        }
        _ => panic!("Expected NetworkError news"),
    }

    // Acknowledge the news
    store.ack_news(AckCoordinatorNews::NetworkError(tx_id))?;

    // Verify the news is acknowledged
    let news_list = store.get_news()?;
    assert_eq!(news_list.len(), 0);

    clear_output();
    Ok(())
}

#[test]
fn test_dispatch_transaction_error_news() -> Result<(), anyhow::Error> {
    let path = format!("test_output/storage_news_test/{}", generate_random_string());

    let storage_config = StorageConfig::new(path, None);
    let storage = Rc::new(Storage::new(&storage_config)?);

    let current_block_hash =
        BlockHash::from_str("0000000000000000000000000000000000000000000000000000000000000000")
            .unwrap();

    let store = BitcoinCoordinatorStore::new(storage, 1)?;

    let tx_id =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200a").unwrap();
    let context = "test_context".to_string();
    let error_msg = "invalid transaction format".to_string();

    // Add DispatchTransactionError news
    let news = CoordinatorNews::DispatchTransactionError(tx_id, context.clone(), error_msg.clone());
    store.update_news(news, current_block_hash)?;

    // Verify the news is stored
    let news_list = store.get_news()?;
    assert_eq!(news_list.len(), 1);
    match &news_list[0] {
        CoordinatorNews::DispatchTransactionError(id, ctx, err) => {
            assert_eq!(*id, tx_id);
            assert_eq!(ctx, &context);
            assert_eq!(err, &error_msg);
        }
        _ => panic!("Expected DispatchTransactionError news"),
    }

    // Acknowledge the news
    store.ack_news(AckCoordinatorNews::DispatchTransactionError(tx_id))?;

    // Verify the news is acknowledged
    let news_list = store.get_news()?;
    assert_eq!(news_list.len(), 0);

    clear_output();
    Ok(())
}

#[test]
fn test_all_error_types_together() -> Result<(), anyhow::Error> {
    let path = format!("test_output/storage_news_test/{}", generate_random_string());

    let storage_config = StorageConfig::new(path, None);
    let storage = Rc::new(Storage::new(&storage_config)?);

    let current_block_hash =
        BlockHash::from_str("0000000000000000000000000000000000000000000000000000000000000000")
            .unwrap();

    let store = BitcoinCoordinatorStore::new(storage, 1)?;

    let tx_id_1 =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200a").unwrap();
    let tx_id_2 =
        Txid::from_str("f9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200b").unwrap();
    let tx_id_3 =
        Txid::from_str("09b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200c").unwrap();
    let tx_id_4 =
        Txid::from_str("19b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200d").unwrap();

    // Add all different types of error news
    store.update_news(
        CoordinatorNews::TransactionAlreadyInMempool(tx_id_1, "context1".to_string()),
        current_block_hash,
    )?;
    store.update_news(
        CoordinatorNews::MempoolRejection(
            tx_id_2,
            "context2".to_string(),
            "mempool full".to_string(),
        ),
        current_block_hash,
    )?;
    store.update_news(
        CoordinatorNews::NetworkError(
            tx_id_3,
            "context3".to_string(),
            "network timeout".to_string(),
        ),
        current_block_hash,
    )?;
    store.update_news(
        CoordinatorNews::DispatchTransactionError(
            tx_id_4,
            "context4".to_string(),
            "invalid tx".to_string(),
        ),
        current_block_hash,
    )?;

    // Verify all news are stored
    let news_list = store.get_news()?;
    assert_eq!(news_list.len(), 4);

    // Verify each type is present
    let mut found_already_in_mempool = false;
    let mut found_mempool_rejection = false;
    let mut found_network_error = false;
    let mut found_dispatch_error = false;

    for news in &news_list {
        match news {
            CoordinatorNews::TransactionAlreadyInMempool(id, _) => {
                assert_eq!(*id, tx_id_1);
                found_already_in_mempool = true;
            }
            CoordinatorNews::MempoolRejection(id, _, _) => {
                assert_eq!(*id, tx_id_2);
                found_mempool_rejection = true;
            }
            CoordinatorNews::NetworkError(id, _, _) => {
                assert_eq!(*id, tx_id_3);
                found_network_error = true;
            }
            CoordinatorNews::DispatchTransactionError(id, _, _) => {
                assert_eq!(*id, tx_id_4);
                found_dispatch_error = true;
            }
            _ => {}
        }
    }

    assert!(
        found_already_in_mempool,
        "TransactionAlreadyInMempool news not found"
    );
    assert!(found_mempool_rejection, "MempoolRejection news not found");
    assert!(found_network_error, "NetworkError news not found");
    assert!(
        found_dispatch_error,
        "DispatchTransactionError news not found"
    );

    // Acknowledge all news
    store.ack_news(AckCoordinatorNews::TransactionAlreadyInMempool(tx_id_1))?;
    store.ack_news(AckCoordinatorNews::MempoolRejection(tx_id_2))?;
    store.ack_news(AckCoordinatorNews::NetworkError(tx_id_3))?;
    store.ack_news(AckCoordinatorNews::DispatchTransactionError(tx_id_4))?;

    // Verify all news are acknowledged
    let news_list = store.get_news()?;
    assert_eq!(news_list.len(), 0);

    clear_output();
    Ok(())
}

#[test]
fn test_transaction_state_failed_on_fatal_error() -> Result<(), anyhow::Error> {
    let path = format!("test_output/storage_news_test/{}", generate_random_string());

    let storage_config = StorageConfig::new(path, None);
    let storage = Rc::new(Storage::new(&storage_config)?);

    let store = BitcoinCoordinatorStore::new(storage, 1)?;

    let tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![],
        output: vec![],
    };

    let tx_id = tx.compute_txid();

    // Save the transaction
    store.save_tx(
        tx.clone(),
        None,
        None,
        "test_context".to_string(),
        None,
        None,
    )?;

    // Mark transaction as failed (simulating fatal error handling)
    store.update_tx_state(tx_id, TransactionState::Failed)?;

    // Verify the transaction is marked as failed
    let saved_tx = store.get_tx(&tx_id)?;
    assert_eq!(saved_tx.state, TransactionState::Failed);

    clear_output();
    Ok(())
}

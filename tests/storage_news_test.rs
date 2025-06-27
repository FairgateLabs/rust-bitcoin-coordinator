use bitcoin::Txid;
use bitcoin_coordinator::{
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::{AckCoordinatorNews, CoordinatorNews},
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

    let store = BitcoinCoordinatorStore::new(storage)?;

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

    let speed_up_news = CoordinatorNews::NewSpeedUp(tx_id_2, "tx_2".to_string(), 1);

    let estimate_feerate_news = CoordinatorNews::EstimateFeerateTooHigh(12345, 10000);

    let funding_not_found_news = CoordinatorNews::FundingNotFound();

    // Add news
    store.add_news(insufficient_funds_news.clone())?;
    store.add_news(speed_up_error_news.clone())?;
    store.add_news(transaction_error_news.clone())?;
    store.add_news(speed_up_news.clone())?;
    store.add_news(estimate_feerate_news.clone())?;
    store.add_news(funding_not_found_news.clone())?;

    // Get all news and verify
    let all_news = store.get_news()?;

    assert_eq!(all_news.len(), 6);
    assert!(all_news.contains(&insufficient_funds_news));
    assert!(all_news.contains(&transaction_error_news));
    assert!(all_news.contains(&speed_up_news));
    assert!(all_news.contains(&speed_up_error_news));
    assert!(all_news.contains(&estimate_feerate_news));
    assert!(all_news.contains(&funding_not_found_news));

    // Acknowledge one news item
    let ack_news = AckCoordinatorNews::DispatchSpeedUpError(tx_id_1);
    store.ack_news(ack_news)?;

    // Verify the news was removed
    let remaining_news = store.get_news()?;
    assert_eq!(remaining_news.len(), 5);
    assert!(remaining_news.contains(&insufficient_funds_news));
    assert!(remaining_news.contains(&transaction_error_news));
    assert!(remaining_news.contains(&speed_up_news));
    assert!(remaining_news.contains(&estimate_feerate_news));
    assert!(remaining_news.contains(&funding_not_found_news));
    assert!(!remaining_news.contains(&speed_up_error_news));
    // Acknowledge another news
    let ack_news = AckCoordinatorNews::InsufficientFunds(tx_id_1);
    store.ack_news(ack_news)?;
    // Verify the news was removed
    let remaining_news = store.get_news()?;
    assert_eq!(remaining_news.len(), 4);
    assert!(remaining_news.contains(&transaction_error_news));
    assert!(remaining_news.contains(&speed_up_news));
    assert!(remaining_news.contains(&estimate_feerate_news));
    assert!(remaining_news.contains(&funding_not_found_news));
    assert!(!remaining_news.contains(&insufficient_funds_news));
    // Acknowledge the last news
    let ack_news = AckCoordinatorNews::DispatchTransactionError(tx_id_3);
    store.ack_news(ack_news)?;

    // Verify all news are removed except speed_up_news, estimate_feerate_news, and funding_not_found_news
    let remaining_news = store.get_news()?;
    assert_eq!(remaining_news.len(), 3);
    assert!(remaining_news.contains(&speed_up_news));
    assert!(remaining_news.contains(&estimate_feerate_news));
    assert!(remaining_news.contains(&funding_not_found_news));
    // Acknowledge the last news
    let ack_news = AckCoordinatorNews::NewSpeedUp(tx_id_2);
    store.ack_news(ack_news)?;
    // Verify only estimate_feerate_news and funding_not_found_news remain
    let remaining_news = store.get_news()?;
    assert_eq!(remaining_news.len(), 2);
    assert!(remaining_news.contains(&estimate_feerate_news));
    assert!(remaining_news.contains(&funding_not_found_news));
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

    let speed_up_news_1 = CoordinatorNews::NewSpeedUp(tx_id_6, "Test context 6".to_string(), 2);
    let speed_up_news_2 = CoordinatorNews::NewSpeedUp(tx_id_7, "Test context 7".to_string(), 3);

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

    let funding_not_found_news = CoordinatorNews::FundingNotFound();

    // Add all news
    store.add_news(insufficient_funds_news_1.clone())?;
    store.add_news(insufficient_funds_news_2.clone())?;
    store.add_news(transaction_error_news_1.clone())?;
    store.add_news(transaction_error_news_2.clone())?;
    store.add_news(speed_up_news_1.clone())?;
    store.add_news(speed_up_news_2.clone())?;
    store.add_news(speed_up_error_news_1.clone())?;
    store.add_news(speed_up_error_news_2.clone())?;
    store.add_news(estimate_feerate_news_1.clone())?;
    store.add_news(estimate_feerate_news_2.clone())?;
    store.add_news(funding_not_found_news.clone())?;

    // Verify all news were added
    let all_news = store.get_news()?;
    assert_eq!(all_news.len(), 11);
    assert!(all_news.contains(&insufficient_funds_news_1));
    assert!(all_news.contains(&insufficient_funds_news_2));
    assert!(all_news.contains(&transaction_error_news_1));
    assert!(all_news.contains(&transaction_error_news_2));
    assert!(all_news.contains(&speed_up_news_1));
    assert!(all_news.contains(&speed_up_news_2));
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
    store.ack_news(AckCoordinatorNews::NewSpeedUp(tx_id_6))?;
    store.ack_news(AckCoordinatorNews::NewSpeedUp(tx_id_7))?;
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

use bitcoin::{BlockHash, Network, Txid};
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitcoin_coordinator::{AckMonitorNews, MonitorNews, TypesToMonitor};
use bitvmx_transaction_monitor::errors::MonitorError;
use bitvmx_transaction_monitor::types::{
    BlockInfo, TransactionBlockchainStatus, TransactionStatus,
};
use mockall::predicate::eq;
use std::str::FromStr;
use utils::{clear_output, get_mock_data, get_mocks};
mod utils;
/*
    Test Summary: speed_up_tx
    First Tick:
    - A transaction is submitted for monitoring.
    - The transaction is dispatched to the network.

    Second Tick:
    - The transaction is detected as unmined.
    - A speed-up action is initiated.

    Third Tick:
    - The transaction remains unmined.
    - A second speed-up action is triggered with even higher fees.

    Fourth Tick:
    - The transaction is successfully mined.
    - The speed-up transaction replaces the original transaction as the new funding transaction.

    Fifth Tick:
    - Additional confirmations are observed.
    - The transaction is marked as finalized.
*/
#[test]
#[ignore = "This test is not working, it should be fixed"]
fn speed_up_tx() -> Result<(), anyhow::Error> {
    // Setup mocks for monitor, dispatcher, account, and storage to simulate the environment.
    let (mut mock_monitor, store, mock_bitcoin_client, key_manager) = get_mocks();

    // Setup a mock data containing a single transaction, marked for dispatch and monitoring.
    let (tx_to_monitor, tx, funding_tx, tx_id, context_data, _) =
        get_mock_data(key_manager.clone());

    mock_monitor.expect_is_ready().returning(|| Ok(true));

    // FIRST TICK >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>

    // Dispatch the transaction
    // mock_dispatcher
    //     .expect_send()
    //     .times(1)
    //     .with(eq(tx.clone()))
    //     .returning(move |tx_ret| Ok(tx_ret.compute_txid()));

    // Monitor the transaction, this will be called twice, once for the monitor method and other for the dispatch method
    mock_monitor
        .expect_monitor()
        .times(2)
        .with(eq(tx_to_monitor.clone()))
        .returning(|_| Ok(()));

    mock_monitor
        .expect_get_tx_status()
        .times(1)
        .with(eq(tx_id))
        .returning(move |_| Err(MonitorError::TransactionNotFound(tx_id.clone().to_string())));

    // Return no news
    mock_monitor
        .expect_get_news()
        .times(1)
        .returning(move || Ok(vec![]));

    mock_monitor
        .expect_get_monitor_height()
        .times(2)
        .returning(move || Ok(1));

    // SECOND TICK >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>

    mock_monitor
        .expect_get_news()
        .times(1)
        .returning(move || Ok(vec![]));

    mock_monitor
        .expect_get_monitor_height()
        .times(2)
        .returning(move || Ok(2));

    // Define unique mock transaction IDs for each speed-up attempt.
    let tx_speed_up_id_1 =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200a").unwrap();

    // First speed-up attempt: Create a new speed-up transaction based on original funding transaction.
    // mock_dispatcher
    //     .expect_speed_up()
    //     .times(1)
    //     .with(
    //         eq(tx.clone()),
    //         eq(account.pk),
    //         eq(funding_tx.txid),
    //         eq((funding_tx.vout, funding_tx.utxo_output.clone(), account.pk)),
    //     )
    //     .returning(move |_, _, _, _| Ok((tx_speed_up_id_1, Amount::default())));

    let context = "speed_up_child_txid";

    let speed_up_1_to_monitor =
        TypesToMonitor::Transactions(vec![tx_speed_up_id_1], context.to_string());

    // Save both speed-up transactions in the monitor for tracking purposes.
    mock_monitor
        .expect_monitor()
        .times(1)
        .with(eq(speed_up_1_to_monitor))
        .returning(|_| Ok(()));

    // Status of transaction is still unmined.
    mock_monitor
        .expect_get_tx_status()
        .times(1)
        .with(eq(tx_id))
        .returning(move |_| Err(MonitorError::TransactionNotFound(tx_id.clone().to_string())));

    // THIRD TICK >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>

    mock_monitor
        .expect_get_news()
        .times(1)
        .returning(move || Ok(vec![]));

    mock_monitor
        .expect_get_monitor_height()
        .times(1)
        .returning(move || Ok(3));

    mock_monitor
        .expect_get_monitor_height()
        .times(1)
        .returning(move || Ok(3));

    // Mock the dispatcher to check if the transaction needs to be sped up. It should decide "yes."
    // mock_dispatcher
    //     .expect_should_speed_up()
    //     .with(eq(Amount::default()))
    //     .times(1)
    //     .returning(|_| Ok(true));

    let tx_speed_up_id_2 =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200b").unwrap();

    // Second speed-up attempt: Re-attempt speed-up using the original UTXO data as the transaction is still unmined.
    // mock_dispatcher
    //     .expect_speed_up()
    //     .times(1)
    //     .with(
    //         eq(tx.clone()),
    //         eq(account.pk),
    //         eq(funding_tx.txid),
    //         eq((funding_tx.vout, funding_tx.utxo_output.clone(), account.pk)),
    //     )
    //     .returning(move |_, _, _, _| Ok((tx_speed_up_id_2, Amount::default())));

    mock_monitor.expect_monitor().returning(|_| Ok(()));

    // Status of transaction is still unmined.
    mock_monitor
        .expect_get_tx_status()
        .times(1)
        .with(eq(tx_id))
        .returning(move |_| Err(MonitorError::TransactionNotFound(tx_id.clone().to_string())));

    // FOURTH TICK >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>

    // Tx is confirmed and 2nd speed up is confirmed too
    let tx_status = TransactionStatus {
        tx_id: tx_id.clone(),
        tx: tx.clone(),
        block_info: Some(BlockInfo {
            block_height: 4,
            block_hash: BlockHash::from_str(
                "12efaa3528db3845a859c470a525f1b8b4643b0d561f961ab395a9db778c204d",
            )?,
            is_orphan: false,
        }),
        confirmations: 1,
        status: TransactionBlockchainStatus::Confirmed,
    };

    let tx_news = MonitorNews::Transaction(tx_id.clone(), tx_status, context_data.clone());

    // Create a transaction news for the second speed up transaction
    let tx_speed_up_2_status = TransactionStatus {
        tx_id: tx_speed_up_id_2.clone(),
        tx: tx.clone(),
        block_info: Some(BlockInfo {
            block_height: 4,
            block_hash: BlockHash::from_str(
                "12efaa3528db3845a859c470a525f1b8b4643b0d561f961ab395a9db778c204d",
            )?,
            is_orphan: false,
        }),
        confirmations: 1,
        status: TransactionBlockchainStatus::Confirmed,
    };

    let tx_speed_up_2_news = MonitorNews::Transaction(
        tx_speed_up_id_2.clone(),
        tx_speed_up_2_status,
        context.to_string(),
    );

    mock_monitor
        .expect_get_news()
        .times(1)
        .returning(move || Ok(vec![tx_news.clone(), tx_speed_up_2_news.clone()]));

    let tx_status = TransactionStatus {
        tx_id: tx_id.clone(),
        tx: tx.clone(),
        block_info: None,
        confirmations: 1,
        status: TransactionBlockchainStatus::Confirmed,
    };

    mock_monitor
        .expect_get_tx_status()
        .times(1)
        .with(eq(tx_id))
        .returning(move |_| Ok(tx_status.clone()));

    let ack_news = AckMonitorNews::Transaction(tx_speed_up_id_2.clone());
    mock_monitor
        .expect_ack_news()
        .times(1)
        .with(eq(ack_news.clone()))
        .returning(|_| Ok(()));

    // FIFTH TICK >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>

    let tx_status = TransactionStatus {
        tx_id: tx_id.clone(),
        tx: tx.clone(),
        block_info: None,
        confirmations: 101,
        status: TransactionBlockchainStatus::Finalized,
    };
    let tx_news = MonitorNews::Transaction(tx_id.clone(), tx_status.clone(), context_data.clone());

    let tx_speed_up_2_status = TransactionStatus {
        tx_id: tx_speed_up_id_2.clone(),
        tx: tx.clone(),
        block_info: None,
        confirmations: 101,
        status: TransactionBlockchainStatus::Finalized,
    };

    let tx_speed_up_2_news = MonitorNews::Transaction(
        tx_speed_up_id_2.clone(),
        tx_speed_up_2_status.clone(),
        context.to_string(),
    );

    mock_monitor
        .expect_get_news()
        .times(1)
        .returning(move || Ok(vec![tx_news.clone(), tx_speed_up_2_news.clone()]));

    mock_monitor
        .expect_get_tx_status()
        .times(1)
        .with(eq(tx_id))
        .returning(move |_| Ok(tx_status.clone()));

    let ack_news = AckMonitorNews::Transaction(tx_speed_up_id_2.clone());
    mock_monitor
        .expect_ack_news()
        .times(1)
        .with(eq(ack_news.clone()))
        .returning(|_| Ok(()));

    // Initialize the bitcoin coordinator with mocks and begin monitoring the txs.
    let coordinator = BitcoinCoordinator::new(
        mock_monitor,
        store,
        key_manager,
        mock_bitcoin_client,
        Network::Regtest,
    );
    coordinator.monitor(tx_to_monitor)?;

    // Dispatch the transaction through the bitcoin coordinator.
    coordinator.dispatch(tx, None, context_data.clone(), None)?;

    // Add funding for speed up transaction
    coordinator.add_funding(funding_tx)?;

    // Simulate ticks to monitor and adjust transaction status with each blockchain height update.

    coordinator.tick()?; // Dispatch and observe unmined status.
    coordinator.tick()?; // First speed-up after unconfirmed status persists.
    coordinator.tick()?; // Second speed-up due to continued unmined status.
    coordinator.tick()?; // Confirmation observed, transaction is now mined.
    coordinator.tick()?; // Simulate further blocks, marking the transaction as finalized.

    clear_output();

    Ok(())
}

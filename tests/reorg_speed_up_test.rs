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
    - After the first tick, the transaction is unmined. A speed-up action is initiated.

    Second Tick:
    - The transaction remains unmined. A second speed-up action is triggered.

    Third Tick:
    - The transaction is successfully mined.
    - The speed-up transaction replaces the original transaction as the new funding transaction.

    Forth Tick - Chain Reorganization:
    - A chain reorganization occurs, resulting in both the original and the speed-up transactions being excluded from the blockchain.
    - The orphaned speed-up transaction is removed from the new funding transaction pool.
*/
#[test]
fn reorg_speed_up_tx() -> Result<(), anyhow::Error> {
    // Setup mocks for monitor, dispatcher, account, and storage to simulate the environment.
    let (mut mock_monitor, store, mut mock_bitcoin_client, key_manager) = get_mocks();

    // Setup a mock data containing a single transaction, marked for dispatch and monitoring.
    let (tx_to_monitor, tx, funding_tx, tx_id, context_data) = get_mock_data();

    mock_monitor.expect_is_ready().returning(|| Ok(true));

    // FIRST TICK >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>

    // Dispatch the transaction
    mock_bitcoin_client
        .expect_send_transaction()
        .times(1)
        .with(eq(tx.clone()))
        .returning(move |tx_ret| Ok(tx_ret.compute_txid()));

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
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200b").unwrap();

    // // First speed-up attempt: Create a new speed-up transaction based on original funding transaction.
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

    let context_child_txid = format!("speed_up_child_txid:{}", tx.compute_txid());

    let speed_up_1_to_monitor =
        TypesToMonitor::Transactions(vec![tx_speed_up_id_1], context_child_txid.clone());

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

    // Tx is confirmed with 1 confirmation and speed up is confirmed with 1 confirmation too
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
    let tx_speed_up_1_status = TransactionStatus {
        tx_id: tx_speed_up_id_1.clone(),
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

    let tx_speed_up_1_news = MonitorNews::Transaction(
        tx_speed_up_id_1.clone(),
        tx_speed_up_1_status,
        context_child_txid.clone(),
    );

    mock_monitor
        .expect_get_news()
        .times(1)
        .returning(move || Ok(vec![tx_news.clone(), tx_speed_up_1_news.clone()]));

    let tx_status = TransactionStatus {
        tx_id: tx_id.clone(),
        tx: tx.clone(),
        block_info: None,
        confirmations: 1,
        status: TransactionBlockchainStatus::Confirmed,
    };

    let ack_news = AckMonitorNews::Transaction(tx_speed_up_id_1.clone());

    mock_monitor
        .expect_ack_news()
        .times(1)
        .with(eq(ack_news.clone()))
        .returning(|_| Ok(()));

    mock_monitor
        .expect_get_tx_status()
        .times(1)
        .with(eq(tx_id))
        .returning(move |_| Ok(tx_status.clone()));

    // FORTH TICK >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>

    let tx_status = TransactionStatus {
        tx_id: tx_id.clone(),
        tx: tx.clone(),
        block_info: Some(BlockInfo::new(
            100,
            BlockHash::from_str("0000000000000000000000000000000000000000000000000000000000000000")
                .unwrap(),
            true,
        )),
        confirmations: 0,
        status: TransactionBlockchainStatus::Orphan,
    };
    let tx_news = MonitorNews::Transaction(tx_id.clone(), tx_status.clone(), context_data.clone());

    let tx_speed_up_1_status = TransactionStatus {
        tx_id: tx_speed_up_id_1.clone(),
        tx: tx.clone(),
        block_info: Some(BlockInfo::new(
            100,
            BlockHash::from_str("0000000000000000000000000000000000000000000000000000000000000000")
                .unwrap(),
            true,
        )),
        confirmations: 0,
        status: TransactionBlockchainStatus::Orphan,
    };

    let tx_speed_up_1_news = MonitorNews::Transaction(
        tx_speed_up_id_1.clone(),
        tx_speed_up_1_status.clone(),
        context_child_txid.clone(),
    );

    mock_monitor
        .expect_get_news()
        .times(1)
        .returning(move || Ok(vec![tx_news.clone(), tx_speed_up_1_news.clone()]));

    mock_monitor
        .expect_get_tx_status()
        .times(1)
        .with(eq(tx_id))
        .returning(move |_| Ok(tx_status.clone()));

    mock_monitor
        .expect_get_monitor_height()
        .times(1)
        .returning(move || Ok(100));

    mock_monitor
        .expect_ack_news()
        .times(1)
        .with(eq(ack_news))
        .returning(|_| Ok(()));

    // mock_dispatcher
    //     .expect_should_speed_up()
    //     .times(1)
    //     .returning(move |_| Ok(false));

    // Initialize the bitcoin coordinator with mocks and begin monitoring the txs.
    let coordinator = BitcoinCoordinator::new(
        mock_monitor,
        store,
        key_manager,
        mock_bitcoin_client,
        Network::Regtest,
    );
    coordinator.monitor(tx_to_monitor)?;

    // Dispatch the transaction through the bitcoin coordinator at the current block height
    coordinator.dispatch(tx, None, context_data.clone(), None)?;

    // Add funding for speed up transaction
    coordinator.add_funding(funding_tx)?;

    // Simulate ticks to monitor and adjust transaction status with each blockchain height update.
    coordinator.tick()?; // Dispatch and observe unmined status.
    coordinator.tick()?; // First speed-up after unconfirmed status persists.
    coordinator.tick()?; // Confirmation observed, transaction is now mined.
    coordinator.tick()?; // Simulate further blocks, marking the transaction as finalized.

    clear_output();

    Ok(())
}

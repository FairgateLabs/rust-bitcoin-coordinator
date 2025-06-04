use bitcoin::{BlockHash, Network, Txid};
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitcoin_coordinator::storage::BitcoinCoordinatorStoreApi;
use bitcoin_coordinator::{AckMonitorNews, MonitorNews, TypesToMonitor};
use bitvmx_transaction_monitor::errors::MonitorError;
use bitvmx_transaction_monitor::types::{
    BlockInfo, TransactionBlockchainStatus, TransactionStatus,
};
use mockall::predicate::eq;
use protocol_builder::types::Utxo;
use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tracing::info;
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
#[ignore = "This test is not working, it should be fixed"]
fn reorg_speed_up_tx() -> Result<(), anyhow::Error> {
    // Setup mocks for monitor, dispatcher, account, and storage to simulate the environment.
    let (mut mock_monitor, store, mut mock_bitcoin_client, key_manager) = get_mocks();

    // Setup a mock data containing a single transaction, marked for dispatch and monitoring.
    let (tx_to_monitor, tx, funding_tx, tx_id, context_data, speedup_utxo) =
        get_mock_data(key_manager.clone());

    mock_monitor.expect_tick().returning(|| Ok(()));
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

    // Monitor the speed up transaction , we don't know the txid.
    mock_monitor
        .expect_monitor()
        .times(1)
        .returning(move |_| Ok(()));

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

    let tx_speed_up_1_news = MonitorNews::Transaction(
        tx_id.clone(), // This should have the txid of the speed up created  internally by the coordinator.
        tx_speed_up_1_status,
        "SPEED_UP_TRANSACTION".to_string(),
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

    let ack_news = AckMonitorNews::Transaction(tx_id.clone());

    let a = store.get_last_speedup()?;

    // mock_monitor
    //     .expect_ack_news()
    //     .times(1)
    //     .with(eq(ack_news.clone()))
    //     .returning(|_| Ok(()));

    // mock_monitor
    //     .expect_get_tx_status()
    //     .times(1)
    //     .with(eq(tx_id))
    //     .returning(move |_| Ok(tx_status.clone()));

    mock_bitcoin_client
        .expect_send_transaction()
        .times(1)
        .returning(move |tx_ret| Ok(tx_ret.compute_txid()));

    mock_monitor
        .expect_get_monitor_height()
        .times(1)
        .returning(move || Ok(3));

    // // FORTH TICK >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>

    // let tx_status = TransactionStatus {
    //     tx_id: tx_id.clone(),
    //     tx: tx.clone(),
    //     block_info: Some(BlockInfo::new(
    //         100,
    //         BlockHash::from_str("0000000000000000000000000000000000000000000000000000000000000000")
    //             .unwrap(),
    //         true,
    //     )),
    //     confirmations: 0,
    //     status: TransactionBlockchainStatus::Orphan,
    // };
    // let tx_news = MonitorNews::Transaction(tx_id.clone(), tx_status.clone(), context_data.clone());

    // let tx_speed_up_1_status = TransactionStatus {
    //     tx_id: tx_speed_up_id_1.clone(),
    //     tx: tx.clone(),
    //     block_info: Some(BlockInfo::new(
    //         100,
    //         BlockHash::from_str("0000000000000000000000000000000000000000000000000000000000000000")
    //             .unwrap(),
    //         true,
    //     )),
    //     confirmations: 0,
    //     status: TransactionBlockchainStatus::Orphan,
    // };

    // let tx_speed_up_1_news = MonitorNews::Transaction(
    //     tx_speed_up_id_1.clone(),
    //     tx_speed_up_1_status.clone(),
    //     context_child_txid.clone(),
    // );

    // mock_monitor
    //     .expect_get_news()
    //     .times(1)
    //     .returning(move || Ok(vec![tx_news.clone(), tx_speed_up_1_news.clone()]));

    // mock_monitor
    //     .expect_get_tx_status()
    //     .times(1)
    //     .with(eq(tx_id))
    //     .returning(move |_| Ok(tx_status.clone()));

    // mock_monitor
    //     .expect_get_monitor_height()
    //     .times(1)
    //     .returning(move || Ok(100));

    // mock_monitor
    //     .expect_ack_news()
    //     .times(1)
    //     .with(eq(ack_news))
    //     .returning(|_| Ok(()));

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
    coordinator.dispatch(tx, Some(speedup_utxo), context_data.clone(), None)?;

    // Add funding for speed up transaction
    coordinator.add_funding(funding_tx)?;

    // Simulate ticks to monitor and adjust transaction status with each blockchain height update.
    coordinator.tick()?; // Dispatch and observe unmined status.
    coordinator.tick()?; // First speed-up after unconfirmed status persists.
    coordinator.tick()?; // Confirmation observed, transaction is now mined.
                         // coordinator.tick()?; // Simulate further blocks, marking the transaction as finalized.

    clear_output();

    Ok(())
}

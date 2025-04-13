use bitcoin::absolute::LockTime;
use bitcoin::{Amount, BlockHash, Transaction};
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitcoin_coordinator::AckTransactionNews;
use bitvmx_transaction_monitor::types::{
    BlockInfo, TransactionBlockchainStatus, TransactionNews, TransactionStatus,
};
use mockall::predicate::eq;
use std::str::FromStr;
use utils::{get_mock_data, get_mocks};
mod utils;
/*
Test Summary:

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
fn reorg_speed_up_tx_test() -> Result<(), anyhow::Error> {
    // Setup mocks for monitor, dispatcher, account, and storage to simulate the environment.
    let (mut mock_monitor, store, account, mut mock_dispatcher) = get_mocks();
    // Setup a mock instance containing a single transaction, marked for dispatch and monitoring.
    let (tx_to_monitor, tx, funding_tx, tx_id, context_data) = get_mock_data();

    // Indicate the monitor is ready to track the instance.
    mock_monitor.expect_is_ready().returning(|| Ok(true));

    // The dispatcher is expected to attempt sending the transaction `T` once initially.
    mock_dispatcher
        .expect_send()
        .times(1)
        .with(eq(tx.clone()))
        .returning(move |tx_ret| Ok(tx_ret.compute_txid()));

    // Simulate that the monitor's instance news initially has no updates about the transaction, and the second tick also did not mined the transaction.
    mock_monitor
        .expect_get_news()
        .times(1)
        .returning(move || Ok(vec![]));

    // Define unique mock transaction IDs for each speed-up attempt.
    let tx_speed_up = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195600).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx_speed_up_id = tx_speed_up.compute_txid();

    let tx_status = TransactionStatus {
        tx_id,
        tx: tx.clone(),
        block_info: Some(BlockInfo {
            block_height: 50,
            block_hash: BlockHash::from_str(
                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
            )
            .unwrap(),
            is_orphan: false,
        }),
        confirmations: 1,
        status: TransactionBlockchainStatus::Confirmed,
    };

    let tx_status_speed_up = TransactionStatus {
        tx_id: tx_speed_up.compute_txid(),
        tx: tx_speed_up.clone(),
        block_info: Some(BlockInfo {
            block_height: 100,
            block_hash: BlockHash::from_str(
                "0000000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap(),
            is_orphan: false,
        }),
        confirmations: 1,
        status: TransactionBlockchainStatus::Confirmed,
    };

    let tx_news = TransactionNews::Transaction(tx_id, tx_status, String::new());
    let tx_news_speed_up =
        TransactionNews::Transaction(tx_speed_up_id, tx_status_speed_up, String::new());

    mock_monitor
        .expect_get_news()
        .times(1)
        .returning(move || Ok(vec![tx_news.clone(), tx_news_speed_up.clone()]));

    let tx_status = TransactionStatus {
        tx_id,
        tx: tx.clone(),
        block_info: Some(BlockInfo {
            block_height: 50,
            block_hash: BlockHash::from_str(
                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
            )
            .unwrap(),
            is_orphan: true,
        }),
        confirmations: 0,
        status: TransactionBlockchainStatus::Confirmed,
    };

    let tx_status_speed_up = TransactionStatus {
        tx_id: tx_speed_up.compute_txid(),
        tx: tx.clone(),
        block_info: Some(BlockInfo {
            block_height: 100,
            block_hash: BlockHash::from_str(
                "0000000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap(),
            is_orphan: true,
        }),
        confirmations: 0,
        status: TransactionBlockchainStatus::Confirmed,
    };

    let tx_news = TransactionNews::Transaction(tx_id, tx_status, String::new());
    let tx_news_speed_up =
        TransactionNews::Transaction(tx_speed_up_id, tx_status_speed_up, String::new());

    mock_monitor
        .expect_get_news()
        .times(1)
        .returning(move || Ok(vec![tx_news.clone(), tx_news_speed_up.clone()]));

    //Last tick no news
    mock_monitor.expect_get_news().returning(move || Ok(vec![]));

    // Mock the dispatcher to check if the transaction needs to be sped up. It should decide "no."
    mock_dispatcher
        .expect_should_speed_up()
        .with(eq(Amount::default()))
        .returning(|_| Ok(false));

    // Transaction status initially shows it as unmined (not seen).
    let tx_status_first_time = TransactionStatus {
        tx_id,
        tx: tx.clone(),
        block_info: None,
        confirmations: 0,
        status: TransactionBlockchainStatus::Finalized,
    };

    mock_monitor
        .expect_get_tx_status()
        .times(1)
        .with(eq(tx_id))
        .returning(move |_| Ok(tx_status_first_time.clone()));

    let tx_status = TransactionStatus {
        tx_id,
        tx: tx.clone(),
        block_info: Some(BlockInfo {
            block_height: 50,
            block_hash: BlockHash::from_str(
                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
            )
            .unwrap(),
            is_orphan: true,
        }),
        confirmations: 0,
        status: TransactionBlockchainStatus::Finalized,
    };

    mock_monitor
        .expect_get_tx_status()
        .with(eq(tx_id.clone()))
        .returning(move |_| Ok(tx_status.clone()));

    // First speed-up attempt: Create a new speed-up transaction based on original funding transaction.
    mock_dispatcher
        .expect_speed_up()
        .times(1)
        .with(
            eq(tx.clone()),
            eq(account.pk),
            eq(funding_tx.tx_id),
            eq((
                funding_tx.utxo_index,
                funding_tx.utxo_output.clone(),
                account.pk,
            )),
        )
        .returning(move |_, _, _, _| Ok((tx_speed_up.compute_txid(), Amount::default())));

    // Configure monitor to begin tracking the instance containing the transaction.
    mock_monitor
        .expect_monitor()
        .with(eq(tx_to_monitor.clone()))
        .returning(|_| Ok(()));

    // Simulate blockchain height, it does not change with each tick, for this test.
    mock_monitor.expect_get_monitor_height().returning(|| Ok(1));

    // Acknowledge transaction updates twice to notify the monitor of our awareness of changes.

    let ack_transaction_news = AckTransactionNews::Transaction(tx_id);
    let ack_transaction_news_speed_up = AckTransactionNews::Transaction(tx_speed_up_id);
    mock_monitor
        .expect_ack_news()
        .times(2)
        .with(eq(ack_transaction_news))
        .returning(|_| Ok(()));

    mock_monitor
        .expect_ack_news()
        .times(2)
        .with(eq(ack_transaction_news_speed_up))
        .returning(|_| Ok(()));

    // Acknowledge transaction twice to notify the monitor of our awareness of changes.
    let ack_transaction_news = AckTransactionNews::Transaction(tx_id);

    mock_monitor
        .expect_ack_news()
        .times(2)
        .with(eq(ack_transaction_news))
        .returning(|_| Ok(()));

    // Save both speed-up transactions in the monitor for tracking purposes.
    // mock_monitor
    //     .expect_monitor()
    //     .times(1)
    //     .with(eq(transaction_monitor))
    //     .returning(|_| Ok(()));

    // Initialize the bitcoin coordinator with mocks and begin monitoring the instance.
    let coordinator = BitcoinCoordinator::new(mock_monitor, store, mock_dispatcher, account);
    coordinator.monitor(tx_to_monitor)?;

    // Dispatch the transaction through the bitcoin coordinator.
    coordinator.dispatch(tx, context_data)?;

    // Simulate ticks to monitor and adjust transaction status with each blockchain height update.

    coordinator.tick()?; // Dispatch and observe unmined status.
    coordinator.tick()?; // Speed-up after unconfirmed status persists.
    coordinator.tick()?; // Transaction should be mined.
    coordinator.tick()?; // Found a reorg, then Transaction and Speed up are not mined.

    Ok(())
}

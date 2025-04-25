use bitcoin_coordinator::{
    coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi},
    TypesToMonitor,
};
use bitvmx_transaction_monitor::errors::MonitorError;
use mockall::predicate::{self, eq};
use utils::{clear_output, get_mock_data, get_mocks};
mod utils;

#[test]
fn coordinator_is_ready_method_test() -> Result<(), anyhow::Error> {
    let (mut mock_monitor, store, account, mock_dispatcher) = get_mocks();

    mock_monitor
        .expect_is_ready()
        .times(1)
        .returning(|| Ok(false));

    mock_monitor
        .expect_is_ready()
        .times(1)
        .returning(|| Ok(true));

    let coordinator = BitcoinCoordinator::new(mock_monitor, store, mock_dispatcher, account);

    let is_ready = coordinator.is_ready()?;

    assert!(!is_ready);

    let is_ready = coordinator.is_ready()?;

    assert!(is_ready);

    clear_output();

    Ok(())
}

#[test]
fn tick_method_is_not_ready() -> Result<(), anyhow::Error> {
    let (mut mock_monitor, store, account, mock_dispatcher) = get_mocks();

    // Monitor is not ready then should call monitor tick
    mock_monitor
        .expect_is_ready()
        .times(1)
        .returning(|| Ok(false));

    mock_monitor.expect_tick().times(1).returning(|| Ok(()));

    let coordinator = BitcoinCoordinator::new(mock_monitor, store, mock_dispatcher, account);

    coordinator.tick()?;

    clear_output();

    Ok(())
}

#[test]
fn monitor_test() -> Result<(), anyhow::Error> {
    let (mut mock_monitor, store, account, mock_dispatcher) = get_mocks();

    let (tx_to_monitor, _, _, _, _) = get_mock_data();

    mock_monitor
        .expect_monitor()
        .with(eq(tx_to_monitor.clone()))
        .returning(|_| Ok(()));

    let coordinator = BitcoinCoordinator::new(mock_monitor, store, mock_dispatcher, account);

    coordinator.monitor(tx_to_monitor)?;

    clear_output();

    Ok(())
}

#[test]
fn dispatch_with_target_block_height() -> Result<(), anyhow::Error> {
    let (mut mock_monitor, store, account, mut mock_dispatcher) = get_mocks();
    let (_, tx, _, _, context) = get_mock_data();
    let tx_id = tx.compute_txid();
    let target_block_height = Some(1001);

    let tx_to_monitor = TypesToMonitor::Transactions(vec![tx_id], context.clone());

    // Always return true for is_ready and empty news for get_news
    mock_monitor.expect_is_ready().returning(|| Ok(true));
    mock_monitor.expect_get_news().returning(move || Ok(vec![]));

    // FIRST TICK (do not dispatch the transaction becasue is not ready) >>>>>>>>>>>>>>>>>
    mock_monitor
        .expect_monitor()
        .with(eq(tx_to_monitor))
        .returning(|_| Ok(()));

    // Mock get_monitor_height
    mock_monitor
        .expect_get_monitor_height()
        .times(1)
        .returning(move || Ok(1000));

    // SECOND TICK (dispatch the transaction, it is ready) >>>>>>>>>>>>>>>>>

    // Mock the dispatcher to send the transaction
    mock_dispatcher
        .expect_send()
        .times(1)
        .with(eq(tx.clone()))
        .returning(move |tx_ret| Ok(tx_ret.compute_txid()));

    // Mock get_monitor_height
    mock_monitor
        .expect_get_monitor_height()
        .returning(move || Ok(1001));

    // Mock get_tx_status
    mock_monitor
        .expect_get_tx_status()
        .times(1)
        .with(eq(tx_id))
        .returning(move |_| Err(MonitorError::TransactionNotFound(tx_id.clone().to_string())));

    let coordinator = BitcoinCoordinator::new(mock_monitor, store, mock_dispatcher, account);

    // Call dispatch with a specific target block height
    coordinator.dispatch(tx, context.clone(), target_block_height)?;

    coordinator.tick()?; // should not dispatch the transaction because height is not reached 1001
    coordinator.tick()?; // should dispatch the transaction because height is reached 1001

    clear_output();

    Ok(())
}

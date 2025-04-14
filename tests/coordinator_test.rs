use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use mockall::predicate::eq;
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

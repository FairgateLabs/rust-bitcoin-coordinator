use bitcoin::Amount;
use bitcoin_coordinator::{
    config::CoordinatorSettingsConfig,
    coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi},
};
use bitcoind::bitcoind::BitcoindFlags;
use bitvmx_bitcoin_rpc::bitcoin_client::BitcoinClientApi;
use protocol_builder::types::Utxo;
use std::rc::Rc;
use tracing::info;

use crate::utils::{config_trace_aux, coordinate_tx, create_test_setup, TestSetupConfig};
mod utils;

// This test verifies the behavior of the BitcoinCoordinator when an error occurs while sending a speedup transaction (either CPFP or RBF).
//
// The test procedure includes:
// - Dispatching a transaction that requires a speedup (e.g., CPFP).
// - Intentionally causing an error by using an invalid funding UTXO for the speedup transaction.
// - Asserting that the coordinator accurately reports the error.
#[test]
fn error_sending_speedup_test() -> Result<(), anyhow::Error> {
    config_trace_aux();

    let mut blocks_mined = 102;
    let setup = create_test_setup(TestSetupConfig {
        blocks_mined,
        bitcoind_flags: Some(BitcoindFlags {
            block_min_tx_fee: 0.00002,
            ..Default::default()
        }),
    })?;

    let amount = Amount::from_sat(23450000);

    // Increment the block count after mining 1 block to fund the address
    blocks_mined += 1;

    let (funding_speedup, _) = setup
        .bitcoin_client
        .fund_address(&setup.funding_wallet, amount)?;

    // Increment the block count after mining 1 block for the funding speedup transaction
    blocks_mined += 1;

    const RETRY_INTERVAL_SECONDS: u64 = 1;
    let mut settings = CoordinatorSettingsConfig::default();
    settings.retry_attempts_sending_tx = Some(4);
    settings.retry_interval_seconds = Some(RETRY_INTERVAL_SECONDS);

    let coordinator = Rc::new(BitcoinCoordinator::new_with_paths(
        &setup.config_bitcoin_client,
        setup.storage.clone(),
        setup.key_manager.clone(),
        Some(settings),
    )?);

    // Advance the coordinator by the number of blocks mined to synchronize with the current blockchain height
    for _ in 0..blocks_mined {
        coordinator.tick()?;
    }

    // Add funding for the speedup transaction using an invalid output index to trigger an error
    coordinator.add_funding(Utxo::new(
        funding_speedup.compute_txid(),
        10,
        amount.to_sat(),
        &setup.public_key,
    ))?;

    coordinator.tick()?;

    coordinate_tx(
        coordinator.clone(),
        amount,
        setup.network,
        setup.key_manager.clone(),
        setup.bitcoin_client.clone(),
        None,
    )?;

    // Mine a block to confirm the initial transactions (tx1 and speedup tx)
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();
    coordinator.tick()?;

    // First tick: Attempt to send the transaction for the first time, expecting an error
    info!("Should print error 1");
    std::thread::sleep(std::time::Duration::from_secs(RETRY_INTERVAL_SECONDS));
    coordinator.tick()?;

    // Second tick: Retry sending the transaction, expecting another error
    info!("Should print error 2");
    std::thread::sleep(std::time::Duration::from_secs(RETRY_INTERVAL_SECONDS));
    coordinator.tick()?;

    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();
    coordinator.tick()?;

    // Third tick: Retry sending the transaction again, expecting a third error
    info!("Should print error 3");
    std::thread::sleep(std::time::Duration::from_secs(RETRY_INTERVAL_SECONDS));
    coordinator.tick()?;

    // Before the final retry, update the funding with a valid UTXO to allow successful dispatch
    let (funding_speedup, funding_vout) = setup
        .bitcoin_client
        .fund_address(&setup.funding_wallet, amount)?;

    coordinator.add_funding(Utxo::new(
        funding_speedup.compute_txid(),
        funding_vout,
        amount.to_sat(),
        &setup.public_key,
    ))?;

    coordinator.tick()?;

    // Dispatch a new transaction (tx2) to be processed
    coordinate_tx(
        coordinator.clone(),
        amount,
        setup.network,
        setup.key_manager.clone(),
        setup.bitcoin_client.clone(),
        None,
    )?;

    // Mine 5 blocks to confirm transaction tx2 and its speedup transaction
    for _ in 0..5 {
        coordinator.tick()?;

        setup
            .bitcoin_client
            .mine_blocks_to_address(1, &setup.funding_wallet)
            .unwrap();

        coordinator.tick()?;
    }

    // Wait for the retry interval to pass before the final retry
    std::thread::sleep(std::time::Duration::from_secs(RETRY_INTERVAL_SECONDS));

    // Mine 5 more blocks to ensure transaction tx2 and its speedup transaction are confirmed
    for _ in 0..5 {
        coordinator.tick()?;

        setup
            .bitcoin_client
            .mine_blocks_to_address(1, &setup.funding_wallet)
            .unwrap();

        coordinator.tick()?;
    }

    let news = coordinator.get_news()?;
    // Verify error notifications and confirmed transactions.
    // The error "bad-txns-inputs-missingorspent" is now classified as "Other" (non-retryable),
    // so it's reported immediately as DispatchSpeedUpError.
    // There should be just one error notification for the failed CPFP attempt.
    assert_eq!(
        news.coordinator_news.len(),
        1,
        "Expected exactly one coordinator news (DispatchSpeedUpError), got {}: {:?}",
        news.coordinator_news.len(),
        news.coordinator_news
    );

    assert!(
        matches!(
            &news.coordinator_news[0],
            bitcoin_coordinator::types::CoordinatorNews::DispatchSpeedUpError(_, _, _, _)
        ),
        "Expected DispatchSpeedUpError, got: {:?}",
        news.coordinator_news[0]
    );

    // Verify monitor news: should be exactly 1 (tx2 confirmed, tx1's CPFP failed so it may not be confirmed)
    // Note: CPFP transactions are filtered out from monitor_news
    assert_eq!(
        news.monitor_news.len(),
        1,
        "Expected exactly one monitor news (tx2 confirmed), got {}: {:?}",
        news.monitor_news.len(),
        news.monitor_news
    );

    setup.bitcoind.stop()?;

    Ok(())
}

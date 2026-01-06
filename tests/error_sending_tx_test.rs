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

// This test verifies the behavior of the BitcoinCoordinator when an error occurs while sending a transaction.
//
// The test procedure includes:
// - Dispatching a transaction that requires funding.
// - Asserting that the coordinator performs exactly the configured number of retries.
// - Validating that the retry count matches the retry_attempts_sending_tx setting.
#[test]
fn error_sending_tx_test() -> Result<(), anyhow::Error> {
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

    let (funding_speedup, funding_speedup_vout) = setup
        .bitcoin_client
        .fund_address(&setup.funding_wallet, amount)?;

    // Increment the block count after mining 1 block to fund the address
    blocks_mined += 1;

    const RETRY_INTERVAL_SECONDS: u64 = 1;
    const EXPECTED_RETRIES: u32 = 3;
    let mut settings = CoordinatorSettingsConfig::default();
    settings.retry_attempts_sending_tx = Some(EXPECTED_RETRIES);
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

    coordinator.add_funding(Utxo::new(
        funding_speedup.compute_txid(),
        funding_speedup_vout,
        amount.to_sat(),
        &setup.public_key,
    ))?;

    coordinator.tick()?;

    // Dispatch the first transaction that will fail due to invalid UTXO
    coordinate_tx(
        coordinator.clone(),
        amount,
        setup.network,
        setup.key_manager.clone(),
        setup.bitcoin_client.clone(),
        Some(0),
    )?;

    // Mine a block to confirm the initial funding transaction
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();
    coordinator.tick()?;

    // Track the number of error notifications we receive
    let mut error_count = 0;

    // Process retries: we expect 1 initial attempt + EXPECTED_RETRIES retries
    // The coordinator will:
    // 1. Try to send the transaction initially (fails due to invalid UTXO)
    // 2. Queue it for retry and increment retry count on each failure
    // 3. Stop retrying after reaching the configured retry_attempts_sending_tx limit
    for attempt in 0..=EXPECTED_RETRIES {
        // Wait for retry interval (except for first attempt)
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_secs(RETRY_INTERVAL_SECONDS));
        }

        coordinator.tick()?;

        // Check for error notifications
        let news = coordinator.get_news()?;
        for news_item in &news.coordinator_news {
            if let bitcoin_coordinator::types::CoordinatorNews::DispatchTransactionError(_, _, _) =
                news_item
            {
                error_count += 1;
                info!("Error notification {} received", error_count);
            }
        }

        // Mine a block every few attempts to keep the chain moving
        if attempt % 2 == 0 {
            setup
                .bitcoin_client
                .mine_blocks_to_address(1, &setup.funding_wallet)
                .unwrap();
            coordinator.tick()?;
        }
    }

    // Verify that we received exactly the expected number of error notifications
    // We should get 1 error for the initial attempt + EXPECTED_RETRIES errors for retries
    let expected_errors = 1 + EXPECTED_RETRIES;
    assert_eq!(
        error_count, expected_errors as usize,
        "Expected {} error notifications (1 initial + {} retries), but got {}",
        expected_errors, EXPECTED_RETRIES, error_count
    );

    setup.bitcoind.stop()?;

    Ok(())
}

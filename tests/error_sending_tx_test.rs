use bitcoin::{Address, Amount, CompressedPublicKey, Network};
use bitcoin_coordinator::{
    config::CoordinatorSettingsConfig,
    coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi},
};
use bitcoind::bitcoind::{Bitcoind, BitcoindFlags};
use bitvmx_bitcoin_rpc::{
    bitcoin_client::{BitcoinClient, BitcoinClientApi},
    rpc_config::RpcConfig,
};
use console::style;
use key_manager::config::KeyManagerConfig;
use key_manager::create_key_manager_from_config;
use key_manager::key_store::KeyStore;
use protocol_builder::types::Utxo;
use std::rc::Rc;
use storage_backend::storage::Storage;
use storage_backend::storage_config::StorageConfig;
use tracing::info;
use utils::generate_random_string;

use crate::utils::{config_trace_aux, coordinate_tx};
mod utils;

// This test verifies the behavior of the BitcoinCoordinator when an error occurs while sending a transaction.
//
// The test procedure includes:
// - Dispatching a transaction that requires funding.
// - Asserting that the coordinator performs exactly the configured number of retries.
// - Validating that the retry count matches the retry_attempts_sending_tx setting.
#[test]
#[ignore = "This test works, but it runs in regtest with a bitcoind running"]
fn error_sending_tx_test() -> Result<(), anyhow::Error> {
    config_trace_aux();

    let mut blocks_mined = 102;
    let network = Network::Regtest;
    let path = format!("test_output/test/{}", generate_random_string());
    let config = StorageConfig::new(path, None);
    let storage = Rc::new(Storage::new(&config).unwrap());
    let config_bitcoin_client = RpcConfig::new(
        network,
        "http://127.0.0.1:18443".to_string(),
        "foo".to_string(),
        "rpcpassword".to_string(),
        "test_wallet".to_string(),
    );
    let config = KeyManagerConfig::new(network.to_string(), None, None, None);
    let key_store = KeyStore::new(storage.clone());
    let key_manager =
        Rc::new(create_key_manager_from_config(&config, key_store, storage.clone()).unwrap());
    let bitcoin_client = Rc::new(BitcoinClient::new_from_config(&config_bitcoin_client)?);

    let bitcoind = Bitcoind::new_with_flags(
        "bitcoin-regtest",
        "ruimarinho/bitcoin-core",
        config_bitcoin_client.clone(),
        BitcoindFlags {
            block_min_tx_fee: 0.00002,
            ..Default::default()
        },
    );

    info!("{} Starting bitcoind", style("Test").green());
    bitcoind.start()?;

    info!("{} Creating keypair in key manager", style("Test").green());
    let public_key = key_manager.derive_keypair(0).unwrap();
    let compressed = CompressedPublicKey::try_from(public_key).unwrap();
    let funding_wallet = Address::p2wpkh(&compressed, network);
    let regtest_wallet = bitcoin_client.init_wallet("test_wallet").unwrap();

    info!(
        "{} Mine {} blocks to address {:?}",
        style("Test").green(),
        blocks_mined,
        regtest_wallet
    );

    let amount = Amount::from_sat(23450000);

    bitcoin_client
        .mine_blocks_to_address(blocks_mined, &regtest_wallet)
        .unwrap();

    let (funding_speedup, funding_speedup_vout) =
        bitcoin_client.fund_address(&funding_wallet, amount)?;

    // Increment the block count after mining 1 block to fund the address
    blocks_mined += 1;

    const RETRY_INTERVAL_SECONDS: u64 = 1;
    const EXPECTED_RETRIES: u32 = 3;
    let mut settings = CoordinatorSettingsConfig::default();
    settings.retry_attempts_sending_tx = Some(EXPECTED_RETRIES);
    settings.retry_interval_seconds = Some(RETRY_INTERVAL_SECONDS);

    let coordinator = Rc::new(BitcoinCoordinator::new_with_paths(
        &config_bitcoin_client,
        storage.clone(),
        key_manager.clone(),
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
        &public_key,
    ))?;

    coordinator.tick()?;

    // Dispatch the first transaction that will fail due to invalid UTXO
    coordinate_tx(
        coordinator.clone(),
        amount,
        network,
        key_manager.clone(),
        bitcoin_client.clone(),
        Some(0),
    )?;

    // Mine a block to confirm the initial funding transaction
    bitcoin_client
        .mine_blocks_to_address(1, &funding_wallet)
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
            bitcoin_client
                .mine_blocks_to_address(1, &funding_wallet)
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

    bitcoind.stop()?;

    Ok(())
}

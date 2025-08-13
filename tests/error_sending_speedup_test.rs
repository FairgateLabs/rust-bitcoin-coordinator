use bitcoin::{Address, Amount, CompressedPublicKey, Network, OutPoint};
use bitcoin_coordinator::{
    config::CoordinatorSettingsConfig,
    coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi},
    TypesToMonitor,
};
use bitcoind::bitcoind::{Bitcoind, BitcoindFlags};
use bitvmx_bitcoin_rpc::{
    bitcoin_client::{BitcoinClient, BitcoinClientApi},
    rpc_config::RpcConfig,
};
use console::style;
use key_manager::create_key_manager_from_config;
use key_manager::key_store::KeyStore;
use key_manager::{config::KeyManagerConfig, key_manager::KeyManager};
use protocol_builder::types::{output::SpeedupData, Utxo};
use std::rc::Rc;
use storage_backend::storage::Storage;
use storage_backend::storage_config::StorageConfig;
use tracing::info;
use tracing_subscriber::EnvFilter;
use utils::{generate_random_string, generate_tx};
mod utils;

fn config_trace_aux() {
    let default_modules = [
        "info",
        "libp2p=off",
        "bitvmx_transaction_monitor=off",
        "bitcoin_indexer=off",
        "bitcoin_coordinator=debug",
        "bitcoin_client=info",
        "p2p_protocol=off",
        "p2p_handler=off",
        "tarpc=off",
        "key_manager=off",
        "memory=off",
    ];

    let filter = EnvFilter::builder()
        .parse(default_modules.join(","))
        .expect("Invalid filter");

    tracing_subscriber::fmt()
        //.without_time()
        //.with_ansi(false)
        .with_target(true)
        .with_env_filter(filter)
        .init();
}

// This test verifies the behavior of the BitcoinCoordinator when an error occurs while sending a speedup transaction (either CPFP or RBF).
//
// The test procedure includes:
// - Dispatching a transaction that requires a speedup (e.g., CPFP).
// - Intentionally causing an error by using an invalid funding UTXO for the speedup transaction.
// - Asserting that the coordinator accurately reports the error.
#[test]
#[ignore = "This test works, but it runs in regtest with a bitcoind running"]
fn error_sending_speedup_test() -> Result<(), anyhow::Error> {
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

    // Increment the block count after mining 1 block to fund the address
    blocks_mined += 1;

    let (funding_speedup, _) = bitcoin_client.fund_address(&funding_wallet, amount)?;

    // Increment the block count after mining 1 block for the funding speedup transaction
    blocks_mined += 1;

    const RETRY_INTERVAL_SECONDS: u64 = 1;
    let mut settings = CoordinatorSettingsConfig::default();
    settings.retry_attempts_sending_tx = Some(4);
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

    // Add funding for the speedup transaction using an invalid output index to trigger an error
    coordinator.add_funding(Utxo::new(
        funding_speedup.compute_txid(),
        10,
        amount.to_sat(),
        &public_key,
    ))?;

    coordinator.tick()?;

    coordinate_tx(
        coordinator.clone(),
        amount,
        network,
        key_manager.clone(),
        bitcoin_client.clone(),
    )?;

    // Mine a block to confirm the initial transactions (tx1 and speedup tx)
    bitcoin_client
        .mine_blocks_to_address(1, &funding_wallet)
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

    bitcoin_client
        .mine_blocks_to_address(1, &funding_wallet)
        .unwrap();
    coordinator.tick()?;

    // Third tick: Retry sending the transaction again, expecting a third error
    info!("Should print error 3");
    std::thread::sleep(std::time::Duration::from_secs(RETRY_INTERVAL_SECONDS));
    coordinator.tick()?;

    // Before the final retry, update the funding with a valid UTXO to allow successful dispatch
    let (funding_speedup, funding_vout) = bitcoin_client.fund_address(&funding_wallet, amount)?;

    coordinator.add_funding(Utxo::new(
        funding_speedup.compute_txid(),
        funding_vout,
        amount.to_sat(),
        &public_key,
    ))?;

    coordinator.tick()?;

    // Dispatch a new transaction (tx2) to be processed
    coordinate_tx(
        coordinator.clone(),
        amount,
        network,
        key_manager.clone(),
        bitcoin_client.clone(),
    )?;

    // Mine 5 blocks to confirm transaction tx2 and its speedup transaction
    for _ in 0..5 {
        coordinator.tick()?;

        bitcoin_client
            .mine_blocks_to_address(1, &funding_wallet)
            .unwrap();

        coordinator.tick()?;
    }

    // Wait for the retry interval to pass before the final retry
    std::thread::sleep(std::time::Duration::from_secs(RETRY_INTERVAL_SECONDS));

    // Mine 5 more blocks to ensure transaction tx2 and its speedup transaction are confirmed
    for _ in 0..5 {
        coordinator.tick()?;

        bitcoin_client
            .mine_blocks_to_address(1, &funding_wallet)
            .unwrap();

        coordinator.tick()?;
    }

    let news = coordinator.get_news()?;
    // Verify that there is one error notification due to retrying, and two confirmed transactions.
    // Note that although there were three retry attempts, only one error notification is present.
    assert_eq!(news.coordinator_news.len(), 1);
    assert_eq!(news.monitor_news.len(), 2);

    bitcoind.stop()?;

    Ok(())
}

fn coordinate_tx(
    coordinator: Rc<BitcoinCoordinator>,
    amount: Amount,
    network: Network,
    key_manager: Rc<KeyManager>,
    bitcoin_client: Rc<BitcoinClient>,
) -> Result<(), anyhow::Error> {
    // Create a funding wallet
    // Fund the funding wallet
    // Create a tx1 and a speedup utxo for tx1
    // Monitor tx1
    // Dispatch tx1
    // First tick dispatch the tx and create and dispatch a speedup tx
    let public_key = key_manager.derive_keypair(0).unwrap();
    let compressed = CompressedPublicKey::try_from(public_key).unwrap();
    let funding_wallet = Address::p2wpkh(&compressed, network);

    let (funding_tx, funding_vout) = bitcoin_client.fund_address(&funding_wallet, amount)?;

    let (tx1, tx1_speedup_utxo) = generate_tx(
        OutPoint::new(funding_tx.compute_txid(), funding_vout),
        amount.to_sat(),
        public_key,
        key_manager.clone(),
    )?;

    let speedup_data = SpeedupData::new(tx1_speedup_utxo);

    let tx_context = "My tx".to_string();
    let tx_to_monitor = TypesToMonitor::Transactions(vec![tx1.compute_txid()], tx_context.clone());
    coordinator.monitor(tx_to_monitor)?;

    // Dispatch the transaction through the bitcoin coordinator.
    coordinator.dispatch(tx1.clone(), Some(speedup_data), tx_context.clone(), None)?;

    Ok(())
}

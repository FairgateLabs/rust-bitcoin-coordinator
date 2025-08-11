use bitcoin::{Address, Amount, CompressedPublicKey, Network, OutPoint};
use bitcoin_coordinator::{
    config::CoordinatorSettings,
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

// This test is designed to verify the behavior of the BitcoinCoordinator
// when there is an error sending a speedup (CPFP or RBF) transaction.
//
// The test will:
// - Attempt to dispatch a transaction that requires a speedup (e.g., CPFP).
// - Trigger an error when sending the speedup transaction (because funding utxo is invalid).
// - Assert that the coordinator correctly reports the error.
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
            block_min_tx_fee: 0.00004,
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

    let (funding_tx, funding_vout) = bitcoin_client.fund_address(&funding_wallet, amount)?;

    // Fund address mines 1 block
    blocks_mined = blocks_mined + 1;

    let (funding_speedup, funding_speedup_vout) =
        bitcoin_client.fund_address(&funding_wallet, amount)?;

    // Funding speed up tx mines 1 block
    blocks_mined = blocks_mined + 1;

    let mut settings = CoordinatorSettings::default();
    settings.retry_attempts_sending_tx = 4;
    settings.retry_interval_seconds = 1;

    let coordinator = Rc::new(BitcoinCoordinator::new_with_paths(
        &config_bitcoin_client,
        storage.clone(),
        key_manager.clone(),
        Some(settings),
    )?);

    // Since we've already mined 102 blocks, we need to advance the coordinator by 102 ticks
    // so the indexer can catch up with the current blockchain height.
    for _ in 0..blocks_mined {
        coordinator.tick()?;
    }

    // Add funding for speed up transaction, in this case we will use the funding_speedup_vout in 10, to get an error after sending CPFP transaction.lleellesad
    coordinator.add_funding(Utxo::new(
        funding_speedup.compute_txid(),
        10,
        amount.to_sat(),
        &public_key,
    ))?;

    coordinate_tx(
        coordinator.clone(),
        amount,
        network,
        key_manager.clone(),
        bitcoin_client.clone(),
    )?;

    coordinator.tick()?;

    info!("Mine and Tick");
    // Mine a block to mined txs (tx1 and speedup tx)
    bitcoin_client
        .mine_blocks_to_address(1, &funding_wallet)
        .unwrap();

    coordinator.tick()?;
    coordinator.tick()?;
    coordinator.tick()?;

    let news = coordinator.get_news()?;
    // First send + 4 retries = 5
    assert_eq!(news.coordinator_news.len(), 5);

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

    coordinator.tick()?;

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

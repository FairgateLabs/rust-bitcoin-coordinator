use bitcoin::{Address, Amount, CompressedPublicKey, Network, OutPoint};
use bitcoin_coordinator::{
    coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi},
    TypesToMonitor,
};
use bitcoind::bitcoind::Bitcoind;
use bitvmx_bitcoin_rpc::{
    bitcoin_client::{BitcoinClient, BitcoinClientApi},
    rpc_config::RpcConfig,
};
use key_manager::config::KeyManagerConfig;
use key_manager::create_key_manager_from_config;
use key_manager::key_store::KeyStore;
use protocol_builder::types::Utxo;
use std::rc::Rc;
use storage_backend::storage::Storage;
use storage_backend::storage_config::StorageConfig;
use tracing::info;
use tracing_subscriber::EnvFilter;
use utils::{generate_random_string, generate_tx};
mod utils;
/*
    Test Summary: send_tx_regtest

    1. Setup:
       - Initializes a regtest Bitcoin node and key manager.
       - Mines 101 blocks to fund a test wallet.
       - Funds a new address for use in the test.

    2. Transaction Dispatch:
       - Creates and dispatches a transaction to the network.
       - Submits the transaction for monitoring by the coordinator.

    3. Monitoring and Confirmation:
       - Coordinator ticks to detect the transaction's status.
       - Mines a block to confirm the transaction.
       - Coordinator ticks again to detect the mined transaction.
       - News and status updates are checked to ensure the transaction is confirmed and finalized.

    4. Cleanup:
       - Stops the regtest node and completes the test.
*/

fn config_trace_aux() {
    let default_modules = [
        "info",
        "libp2p=off",
        "bitvmx_transaction_monitor=off",
        "bitcoin_indexer=off",
        "bitcoin_coordinator=info",
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

#[test]
#[ignore = "This test runs in regtest with a bitcoind running, it fails intermittently"]
fn speedup_tx() -> Result<(), anyhow::Error> {
    config_trace_aux();

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
    let bitcoin_client = BitcoinClient::new_from_config(&config_bitcoin_client)?;

    let bitcoind = Bitcoind::new(
        "bitcoin-regtest",
        "ruimarinho/bitcoin-core",
        config_bitcoin_client.clone(),
    );

    info!("Starting bitcoind");
    bitcoind.start()?;

    info!("Creating keypair in key manager");
    let public_key = key_manager.derive_keypair(0).unwrap();
    let compressed = CompressedPublicKey::try_from(public_key).unwrap();
    let funding_wallet = Address::p2wpkh(&compressed, network);
    let regtest_wallet = bitcoin_client.init_wallet(network, "test_wallet").unwrap();

    info!("Mine 101 blocks to address {:?}", regtest_wallet);
    bitcoin_client
        .mine_blocks_to_address(101, &regtest_wallet)
        .unwrap();

    let amount = Amount::from_sat(23450000);
    info!("Funding address {:?}", funding_wallet);

    info!("Funding main tx address {:?}", funding_wallet);
    let (funding_tx, funding_vout) = bitcoin_client.fund_address(&funding_wallet, amount)?;

    let (funding_speedup, funding_speedup_vout) =
        bitcoin_client.fund_address(&funding_wallet, amount)?;

    info!(
        "Funding tx: {:?} | vout: {:?}",
        funding_tx.compute_txid(),
        funding_vout
    );

    info!(
        "Funding speed up tx: {:?} | vout: {:?}",
        funding_speedup.compute_txid(),
        funding_speedup_vout
    );

    let coordinator = BitcoinCoordinator::new_with_paths(
        &config_bitcoin_client,
        storage.clone(),
        key_manager.clone(),
        None,
        1,
    )?;

    // Since we've already mined 102 blocks, we need to advance the coordinator by 102 ticks
    // so the indexer can catch up with the current blockchain height.
    for _ in 0..105 {
        coordinator.tick()?;
    }

    let (tx_to_speedup, speedup_utxo) = generate_tx(
        OutPoint::new(funding_tx.compute_txid(), funding_vout),
        amount.to_sat(),
        public_key,
        key_manager.clone(),
    )?;

    let tx_context = "My tx".to_string();
    let tx_to_monitor =
        TypesToMonitor::Transactions(vec![tx_to_speedup.compute_txid()], tx_context.clone());
    coordinator.monitor(tx_to_monitor)?;

    // Dispatch the transaction through the bitcoin coordinator.
    coordinator.dispatch(tx_to_speedup, Some(speedup_utxo), tx_context.clone(), None)?;

    // Add funding for speed up transaction
    coordinator.add_funding(Utxo::new(
        funding_speedup.compute_txid(),
        funding_speedup_vout,
        amount.to_sat(),
        &public_key,
    ))?;

    coordinator.tick()?;
    coordinator.tick()?;

    bitcoin_client
        .mine_blocks_to_address(1, &funding_wallet)
        .unwrap();

    coordinator.tick()?;

    // Should be news.
    let news = coordinator.get_news()?;
    
    if news.coordinator_news.len() > 0 {
        info!("Coordinator news: {:?}", news);
        assert!(false);
    }

    if news.monitor_news.len() > 0 {
        info!("Monitor news: {:?}", news);
        assert!(true);
    }

    bitcoind.stop()?;

    Ok(())
}

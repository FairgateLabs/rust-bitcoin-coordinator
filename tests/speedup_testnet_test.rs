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
use utils::{generate_random_string, generate_tx};
mod utils;

#[test]
#[ignore = "This test runs in regtest with a bitcoind running, it fails intermittently"]
fn speed_up_tx() -> Result<(), anyhow::Error> {
    let log_level = tracing::Level::INFO;
    tracing_subscriber::fmt().with_max_level(log_level).init();
    let network = Network::Testnet;
    let path = format!("test_output/test/{}", generate_random_string());
    let config = StorageConfig::new(path, None);
    let storage = Rc::new(Storage::new(&config).unwrap());
    let config_bitcoin_client = RpcConfig::new(
        network,
         "https://distinguished-intensive-frost.btc-testnet.quiknode.pro/38d0f064dc8e72fe44d8a9a762d448bc64c54619/".to_string(),
        "".to_string(),
        "".to_string(),
        "test_wallet".to_string(),
    );

    let config = KeyManagerConfig::new(network.to_string(), None, None, None);
    let key_store = KeyStore::new(storage.clone());
    let key_manager =
        Rc::new(create_key_manager_from_config(&config, key_store, storage.clone()).unwrap());
    let bitcoin_client = BitcoinClient::new_from_config(&config_bitcoin_client)?;

    info!("Deriving keypair");
    let public_key = key_manager.derive_keypair(0).unwrap();
    let compressed = CompressedPublicKey::try_from(public_key).unwrap();
    let funding_wallet = Address::p2wpkh(&compressed, network);
    let regtest_wallet = bitcoin_client.init_wallet(network, "test_wallet").unwrap();

    info!("Mine 101 blocks to address {:?}", regtest_wallet);
    bitcoin_client
        .mine_blocks_to_address(101, &regtest_wallet)
        .unwrap();

    let amount = Amount::from_sat(234500000);
    info!("Funding address {:?}", funding_wallet);
    let (funding_tx, vout) = bitcoin_client.fund_address(&funding_wallet, amount)?;

    info!(
        "Funding tx: {:?} | vout: {:?}",
        funding_tx.compute_txid(),
        vout
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

    let tx = generate_tx(
        OutPoint::new(funding_tx.compute_txid(), 0),
        amount.to_sat(),
        public_key,
        key_manager.clone(),
    )?;

    let tx_context = "My tx".to_string();
    let tx_to_monitor = TypesToMonitor::Transactions(vec![tx.compute_txid()], tx_context.clone());
    coordinator.monitor(tx_to_monitor)?;

    // Dispatch the transaction through the bitcoin coordinator.
    coordinator.dispatch(tx, None, tx_context.clone(), None)?;

    // Add funding for speed up transaction
    coordinator.add_funding(Utxo::new(
        funding_tx.compute_txid(),
        vout,
        10000,
        &public_key,
    ))?;

    info!("Dispatching transaction");
    coordinator.tick()?;

    info!("Mining transaction");
    bitcoin_client
        .mine_blocks_to_address(1, &funding_wallet)
        .unwrap();

    info!("Detecting transaction");
    coordinator.tick()?;

    // Should be news.
    let news = coordinator.get_news()?;
    if news.coordinator_news.len() > 0 {
        info!("Coordinator news: {:?}", news);
    }

    if news.monitor_news.len() > 0 {
        info!("Monitor news: {:?}", news);
    }

    Ok(())
}

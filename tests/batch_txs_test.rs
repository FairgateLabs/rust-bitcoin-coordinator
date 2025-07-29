use bitcoin::{Address, Amount, CompressedPublicKey, Network, OutPoint};
use bitcoin_coordinator::{
    config::CoordinatorSettings,
    coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi},
    TypesToMonitor,
};
use bitcoind::bitcoind::Bitcoind;
use bitvmx_bitcoin_rpc::{
    bitcoin_client::{BitcoinClient, BitcoinClientApi},
    rpc_config::RpcConfig,
};
use console::style;
use key_manager::create_key_manager_from_config;
use key_manager::key_store::KeyStore;
use key_manager::{config::KeyManagerConfig, key_manager::KeyManager};
use protocol_builder::types::{output::SpeedupData, Utxo};
use std::sync::Arc;
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
        "bitcoin_coordinator=info",
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

// The idea of this test is to dispatch a lot of txs and check if the coordinator can handle it.
// What we are testing is the batch dispatching of txs. So should be able to dispatch 200 txs in a single tick and create 3 CPFPs.
#[test]
#[ignore = "This test requires a running bitcoind in regtest mode"]
fn batch_txs_regtest_test() -> Result<(), anyhow::Error> {
    config_trace_aux();

    let mut blocks_mined = 102;
    let network = Network::Regtest;
    let path = format!("test_output/test/{}", generate_random_string());
    let config = StorageConfig::new(path, None);
    let storage = Arc::new(Storage::new(&config).unwrap());
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
        Arc::new(create_key_manager_from_config(&config, key_store, storage.clone()).unwrap());
    let bitcoin_client = Arc::new(BitcoinClient::new_from_config(&config_bitcoin_client)?);

    let bitcoind = Bitcoind::new(
        "bitcoin-regtest",
        "ruimarinho/bitcoin-core",
        config_bitcoin_client.clone(),
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

    info!(
        "{} Funding tx address {:?}",
        style("Test").green(),
        funding_wallet
    );

    info!(
        "{} Funding tx: {:?} | vout: {:?}",
        style("Test").green(),
        funding_tx.compute_txid(),
        funding_vout
    );

    let (funding_speedup, funding_speedup_vout) =
        bitcoin_client.fund_address(&funding_wallet, amount)?;

    // Funding speed up tx mines 1 block
    blocks_mined = blocks_mined + 1;

    info!(
        "{} Funding speed up tx: {:?} | vout: {:?}",
        style("Test").green(),
        funding_speedup.compute_txid(),
        funding_speedup_vout
    );

    let coordinator = Rc::new(BitcoinCoordinator::new_with_paths(
        &config_bitcoin_client,
        storage.clone(),
        key_manager.clone(),
        Some(CoordinatorSettings::default()),
    )?);

    // Since we've already mined 102 blocks, we need to advance the coordinator by 102 ticks
    // so the indexer can catch up with the current blockchain height.
    for _ in 0..blocks_mined {
        coordinator.tick()?;
    }

    // Add funding for speed up transaction
    coordinator.add_funding(Utxo::new(
        funding_speedup.compute_txid(),
        funding_speedup_vout,
        amount.to_sat(),
        &public_key,
    ))?;

    // Create 60 txs with funding and dispatch them using the coordinator.
    for _ in 0..60 {
        coordinate_tx(
            coordinator.clone(),
            amount,
            network,
            key_manager.clone(),
            bitcoin_client.clone(),
        )?;
    }

    // Up to here we have 60 txs dispatched and they should be batched.
    for _ in 0..60 {
        coordinator.tick()?;
    }

    bitcoin_client.mine_blocks_to_address(1, &funding_wallet)?;

    coordinator.tick()?;

    // Only 24 transactions can remain unconfirmed at this point because the coordinator enforces a maximum limit of 24 unconfirmed parent transactions (MAX_LIMIT_UNCONFIRMED_PARENTS).
    // The first batch of transactions is successfully dispatched, but when the coordinator attempts to dispatch the next batch, it hits the unconfirmed parent limit and does not dispatch further transactions.
    // This test asserts that the coordinator correctly enforces this policy.
    let news = coordinator.get_news()?;
    assert_eq!(news.monitor_news.len(), 24);

    for _ in 0..24 {
        coordinator.tick()?;
    }

    bitcoin_client.mine_blocks_to_address(1, &funding_wallet)?;

    coordinator.tick()?;

    let news = coordinator.get_news()?;
    assert_eq!(news.monitor_news.len(), 48);

    for _ in 0..12 {
        coordinator.tick()?;
    }

    bitcoin_client.mine_blocks_to_address(1, &funding_wallet)?;

    coordinator.tick()?;

    let news = coordinator.get_news()?;
    assert_eq!(news.monitor_news.len(), 60);

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

    let tx_context = "My tx".to_string();
    let tx_to_monitor = TypesToMonitor::Transactions(vec![tx1.compute_txid()], tx_context.clone());
    coordinator.monitor(tx_to_monitor)?;

    // Dispatch the transaction through the bitcoin coordinator.
    coordinator.dispatch(
        tx1.clone(),
        Some(SpeedupData::new(tx1_speedup_utxo)),
        tx_context.clone(),
        None,
    )?;

    Ok(())
}

use bitcoin::{Address, Amount, CompressedPublicKey, Network, OutPoint};
use bitcoin_coordinator::{
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

// Almost every transaction sent in the protocol uses a CPFP (Child Pays For Parent) transaction for broadcasting.
// The purpose of this test is to pay for tx1.
// Tx1 will not be mined initially because its fee is too low. Therefore, we need to replace the CPFP transaction with a
// new one that has a higher fee, repeating this process two more times (for a total of 3 RBF transactions).
// When RBF pays the sufficient fee, the tx1 will be mined. And the last RBF also will be mined.
#[test]
#[ignore = "This test works, but it runs in regtest with a bitcoind running"]
fn replace_speedup_regtest_test() -> Result<(), anyhow::Error> {
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
        None,
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

    // Create 10 txs and dispatch them
    for _ in 0..10 {
        coordinate_tx(
            coordinator.clone(),
            amount,
            network,
            key_manager.clone(),
            bitcoin_client.clone(),
        )?;
    }

    // Up to here we have 10 txs dispatched and 10 CPFP dispatched.
    // In this tick coordinator should RBF the last CPFP.
    coordinator.tick()?;

    for _ in 0..19 {
        info!("Mine and Tick");
        // Mine a block to mined txs (tx1 and speedup tx)
        bitcoin_client
            .mine_blocks_to_address(1, &funding_wallet)
            .unwrap();

        coordinator.tick()?;
    }

    let news = coordinator.get_news()?;
    assert_eq!(news.monitor_news.len(), 10);

    let news = coordinator.get_news()?;

    assert_eq!(news.monitor_news.len(), 10);

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

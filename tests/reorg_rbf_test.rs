use bitcoin::{Address, Amount, CompressedPublicKey, Network};
use bitcoin_coordinator::{
    config::CoordinatorSettingsConfig,
    coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi},
    MonitorNews,
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

#[test]
#[ignore = "This test works, but it runs in regtest with a bitcoind running"]
fn replace_speedup_regtest_test() -> Result<(), anyhow::Error> {
    config_trace_aux();
    // This test simulates a blockchain reorganization scenario. It begins by setting up a Bitcoin
    // regtest environment and dispatching a transaction with a Child-Pays-For-Parent (CPFP) speedup.
    // The test continues to apply speedups until a Replace-By-Fee (RBF) is necessary. After executing
    // the RBF, the blockchain is reorganized. The test then verifies that all transactions are correctly
    // handled and ensures that dispatching can continue smoothly.

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
        "bitcoin/bitcoin:29.1",
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

    // Fund address mines 1 block
    blocks_mined += 1;

    let (funding_speedup, funding_speedup_vout) =
        bitcoin_client.fund_address(&funding_wallet, amount)?;

    // Funding speed up tx mines 1 block
    blocks_mined += 1;

    info!(
        "{} Funding speed up tx: {:?} | vout: {:?}",
        style("Test").green(),
        funding_speedup.compute_txid(),
        funding_speedup_vout
    );

    // We reduce the max unconfirmed speedups to 2 to test the RBF behavior
    let mut settings = CoordinatorSettingsConfig::default();
    settings.max_unconfirmed_speedups = Some(2);

    let coordinator = Rc::new(BitcoinCoordinator::new_with_paths(
        &config_bitcoin_client,
        storage.clone(),
        key_manager.clone(),
        Some(settings),
    )?);

    // Advance the coordinator by the number of blocks mined to sync with the blockchain height.
    for _ in 0..blocks_mined {
        coordinator.tick()?;
    }

    // Add funding for the speedup transaction
    coordinator.add_funding(Utxo::new(
        funding_speedup.compute_txid(),
        funding_speedup_vout,
        amount.to_sat(),
        &public_key,
    ))?;

    coordinate_tx(
        coordinator.clone(),
        amount,
        network,
        key_manager.clone(),
        bitcoin_client.clone(),
        None,
    )?;

    coordinator.tick()?;

    for _ in 0..4 {
        info!("{} Mine and Tick", style("Test").green());
        // Mine a block to confirm tx1 and its speedup transaction
        bitcoin_client
            .mine_blocks_to_address(1, &funding_wallet)
            .unwrap();
        coordinator.tick()?;
    }

    let news = coordinator.get_news()?;
    assert_eq!(news.monitor_news.len(), 1);

    let best_block = bitcoin_client.get_best_block()?;
    let block_hash = bitcoin_client.get_block_id_by_height(&best_block).unwrap();
    bitcoin_client.invalidate_block(&block_hash).unwrap();
    info!("{} Invalidated block", style("Test").green());

    coordinator.tick()?;

    let news = coordinator.get_news()?;

    assert!(
        news.monitor_news.iter().all(|n| match n {
            MonitorNews::Transaction(_, tx_status, _) => tx_status.is_orphan(),
            _ => false,
        }),
        "Not all news are in Orphan status"
    );

    coordinator.tick()?;

    // Dispatch two more transactions to observe the reorganization effects
    coordinate_tx(
        coordinator.clone(),
        amount,
        network,
        key_manager.clone(),
        bitcoin_client.clone(),
        None,
    )?;

    coordinate_tx(
        coordinator.clone(),
        amount,
        network,
        key_manager.clone(),
        bitcoin_client.clone(),
        None,
    )?;

    let public_key = key_manager.derive_keypair(1).unwrap();
    let compressed = CompressedPublicKey::try_from(public_key).unwrap();
    let funding_wallet = Address::p2wpkh(&compressed, network);

    for _ in 0..10 {
        coordinator.tick()?;

        bitcoin_client
            .mine_blocks_to_address(1, &funding_wallet)
            .unwrap();

        coordinator.tick()?;
    }

    coordinator.tick()?;

    let news = coordinator.get_news()?;

    assert!(
        news.monitor_news.iter().all(|n| match n {
            MonitorNews::Transaction(_, tx_status, _) => tx_status.is_confirmed(),
            _ => false,
        }),
        "Not all news are in Confirmed status"
    );

    assert_eq!(news.monitor_news.len(), 3);

    bitcoind.stop()?;

    Ok(())
}

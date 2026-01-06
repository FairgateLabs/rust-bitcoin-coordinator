use bitcoin::{Address, Amount, CompressedPublicKey};
use bitcoin_coordinator::{
    config::CoordinatorSettingsConfig,
    coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi},
    MonitorNews,
};
use bitcoind::bitcoind::BitcoindFlags;
use bitvmx_bitcoin_rpc::bitcoin_client::BitcoinClientApi;
use console::style;
use protocol_builder::types::Utxo;
use std::rc::Rc;
use tracing::info;

use crate::utils::{config_trace_aux, coordinate_tx, create_test_setup, TestSetupConfig};
mod utils;

#[test]
fn replace_speedup_regtest_test() -> Result<(), anyhow::Error> {
    config_trace_aux();
    // This test simulates a blockchain reorganization scenario. It begins by setting up a Bitcoin
    // regtest environment and dispatching a transaction with a Child-Pays-For-Parent (CPFP) speedup.
    // The test continues to apply speedups until a Replace-By-Fee (RBF) is necessary. After executing
    // the RBF, the blockchain is reorganized. The test then verifies that all transactions are correctly
    // handled and ensures that dispatching can continue smoothly.

    let mut blocks_mined = 102;
    let setup = create_test_setup(TestSetupConfig {
        blocks_mined,
        bitcoind_flags: Some(BitcoindFlags {
            block_min_tx_fee: 0.00004,
            ..Default::default()
        }),
    })?;

    let amount = Amount::from_sat(23450000);

    // Fund address mines 1 block
    blocks_mined += 1;

    let (funding_speedup, funding_speedup_vout) = setup
        .bitcoin_client
        .fund_address(&setup.funding_wallet, amount)?;

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
        &setup.config_bitcoin_client,
        setup.storage.clone(),
        setup.key_manager.clone(),
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
        &setup.public_key,
    ))?;

    coordinate_tx(
        coordinator.clone(),
        amount,
        setup.network,
        setup.key_manager.clone(),
        setup.bitcoin_client.clone(),
        None,
    )?;

    coordinator.tick()?;

    for _ in 0..4 {
        info!("{} Mine and Tick", style("Test").green());
        // Mine a block to confirm tx1 and its speedup transaction
        setup
            .bitcoin_client
            .mine_blocks_to_address(1, &setup.funding_wallet)
            .unwrap();
        coordinator.tick()?;
    }

    let news = coordinator.get_news()?;
    assert_eq!(news.monitor_news.len(), 1);

    let best_block = setup.bitcoin_client.get_best_block()?;
    let block_hash = setup
        .bitcoin_client
        .get_block_id_by_height(&best_block)
        .unwrap();
    setup.bitcoin_client.invalidate_block(&block_hash).unwrap();
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
        setup.network,
        setup.key_manager.clone(),
        setup.bitcoin_client.clone(),
        None,
    )?;

    coordinate_tx(
        coordinator.clone(),
        amount,
        setup.network,
        setup.key_manager.clone(),
        setup.bitcoin_client.clone(),
        None,
    )?;

    let public_key = setup
        .key_manager
        .derive_keypair(key_manager::key_type::BitcoinKeyType::P2tr, 1)
        .unwrap();
    let compressed = CompressedPublicKey::try_from(public_key).unwrap();
    let funding_wallet = Address::p2wpkh(&compressed, setup.network);

    for _ in 0..10 {
        coordinator.tick()?;

        setup
            .bitcoin_client
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

    setup.bitcoind.stop()?;

    Ok(())
}

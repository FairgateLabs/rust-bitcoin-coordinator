use crate::utils::{config_trace_aux, coordinate_tx, create_test_setup, TestSetupConfig};
use bitcoin::Amount;
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitcoind::bitcoind::BitcoindFlags;
use bitvmx_bitcoin_rpc::bitcoin_client::BitcoinClientApi;
use console::style;
use protocol_builder::types::Utxo;
use std::rc::Rc;
use tracing::info;
mod utils;

#[test]
fn replace_speedup_regtest_test() -> Result<(), anyhow::Error> {
    config_trace_aux();

    let mut blocks_mined = 102;
    let setup = create_test_setup(TestSetupConfig {
        blocks_mined,
        bitcoind_flags: Some(BitcoindFlags {
            block_min_tx_fee: 0.00004,
            ..Default::default()
        }),
    })?;

    let amount = Amount::from_sat(23450000);

    let (funding_tx, funding_vout) = setup
        .bitcoin_client
        .fund_address(&setup.funding_wallet, amount)?;

    // Fund address mines 1 block
    blocks_mined = blocks_mined + 1;

    info!(
        "{} Funding tx address {:?}",
        style("Test").green(),
        setup.funding_wallet
    );

    info!(
        "{} Funding tx: {:?} | vout: {:?}",
        style("Test").green(),
        funding_tx.compute_txid(),
        funding_vout
    );

    let (funding_speedup, funding_speedup_vout) = setup
        .bitcoin_client
        .fund_address(&setup.funding_wallet, amount)?;

    // Funding speed up tx mines 1 block
    blocks_mined = blocks_mined + 1;

    info!(
        "{} Funding speed up tx: {:?} | vout: {:?}",
        style("Test").green(),
        funding_speedup.compute_txid(),
        funding_speedup_vout
    );

    let coordinator = Rc::new(BitcoinCoordinator::new_with_paths(
        &setup.config_bitcoin_client,
        setup.storage.clone(),
        setup.key_manager.clone(),
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

    info!("Mine and Tick");
    // Mine a block to mined txs (tx1 and speedup tx)
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    coordinator.tick()?;

    let news = coordinator.get_news()?;
    assert_eq!(news.monitor_news.len(), 0);

    coordinator.tick()?;

    info!("Mine and Tick");
    // Mine a block to mined txs (tx1 and speedup tx)
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    coordinator.tick()?;

    info!("Mine and Tick");
    // Mine a block to mined txs (tx1 and speedup tx)
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    coordinator.tick()?;

    info!("Mine and Tick");
    // Mine a block to mined txs (tx1 and speedup tx)
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    coordinator.tick()?;

    info!("Mine and Tick");
    // Mine a block to mined txs (tx1 and speedup tx)
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    coordinator.tick()?;

    info!("Mine and Tick");
    // Mine a block to mined txs (tx1 and speedup tx)
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    coordinator.tick()?;

    info!("Mine and Tick");
    // Mine a block to mined txs (tx1 and speedup tx)
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    coordinator.tick()?;
    info!("Mine and Tick");
    // Mine a block to mined txs (tx1 and speedup tx)
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    coordinator.tick()?;
    info!("Mine and Tick");
    // Mine a block to mined txs (tx1 and speedup tx)
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    coordinator.tick()?;
    info!("Mine and Tick");
    // Mine a block to mined txs (tx1 and speedup tx)
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    coordinator.tick()?;
    info!("Mine and Tick");
    // Mine a block to mined txs (tx1 and speedup tx)
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    coordinator.tick()?;

    let news = coordinator.get_news()?;
    assert_eq!(news.monitor_news.len(), 1);

    setup.bitcoind.stop()?;

    Ok(())
}

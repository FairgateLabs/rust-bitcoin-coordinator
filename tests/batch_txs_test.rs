use bitcoin::Amount;
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitvmx_bitcoin_rpc::bitcoin_client::BitcoinClientApi;
use console::style;
use protocol_builder::types::Utxo;
use std::rc::Rc;
use tracing::info;

use crate::utils::{config_trace_aux, coordinate_tx, create_test_setup, TestSetupConfig};
mod utils;

// This is a integration test.
// The idea of this test is to dispatch a lot of txs and check if the coordinator can handle it.
// What we are testing is the batch dispatching of txs. So should be able to dispatch 200 txs in a single tick and create 3 CPFPs.
#[test]
fn batch_txs_regtest_test() -> Result<(), anyhow::Error> {
    config_trace_aux();

    let mut blocks_mined = 102;
    let setup = create_test_setup(TestSetupConfig {
        blocks_mined,
        bitcoind_flags: None,
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

    // Create 60 txs with funding and dispatch them using the coordinator.
    for _ in 0..60 {
        coordinate_tx(
            coordinator.clone(),
            amount,
            setup.network,
            setup.key_manager.clone(),
            setup.bitcoin_client.clone(),
            None,
        )?;
    }

    // Up to here we have 60 txs dispatched and they should be batched.
    for _ in 0..60 {
        coordinator.tick()?;
    }

    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)?;

    coordinator.tick()?;

    // Only 24 transactions can remain unconfirmed at this point because the coordinator enforces a maximum limit of 24 unconfirmed parent transactions (MAX_LIMIT_UNCONFIRMED_PARENTS).
    // The first batch of transactions is successfully dispatched, but when the coordinator attempts to dispatch the next batch, it hits the unconfirmed parent limit and does not dispatch further transactions.
    // This test asserts that the coordinator correctly enforces this policy.
    let news = coordinator.get_news()?;
    assert_eq!(news.monitor_news.len(), 24);

    for _ in 0..24 {
        coordinator.tick()?;
    }

    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)?;

    coordinator.tick()?;

    let news = coordinator.get_news()?;
    assert_eq!(news.monitor_news.len(), 48);

    for _ in 0..12 {
        coordinator.tick()?;
    }

    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)?;

    coordinator.tick()?;

    let news = coordinator.get_news()?;
    assert_eq!(news.monitor_news.len(), 60);

    setup.bitcoind.stop()?;

    Ok(())
}

use bitcoin::Amount;
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitcoind::bitcoind::BitcoindFlags;
use bitvmx_bitcoin_rpc::bitcoin_client::BitcoinClientApi;
use console::style;
use protocol_builder::types::Utxo;
use std::rc::Rc;
use tracing::info;

use crate::utils::{config_trace_aux, coordinate_tx, create_test_setup, TestSetupConfig};
mod utils;

// Almost every transaction sent in the protocol uses a CPFP (Child Pays For Parent) transaction for broadcasting.
// The purpose of this test is to pay for tx1.
// Tx1 will not be mined initially because its fee is too low. Therefore, we need to replace the CPFP transaction with a
// new one that has a higher fee, repeating this process two more times (for a total of 3 RBF transactions).
// When RBF pays the sufficient fee, the tx1 will be mined. And the last RBF also will be mined.
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

    // Create 10 txs and dispatch them
    for _ in 0..10 {
        coordinate_tx(
            coordinator.clone(),
            amount,
            setup.network,
            setup.key_manager.clone(),
            setup.bitcoin_client.clone(),
            None,
        )?;

        coordinator.tick()?;
    }

    // Up to here we have 10 txs dispatched and 10 CPFP dispatched.
    // In this tick coordinator should RBF the last CPFP.
    coordinator.tick()?;

    for _ in 0..19 {
        info!("Mine and Tick");
        // Mine a block to mined txs (tx1 and speedup tx)
        setup
            .bitcoin_client
            .mine_blocks_to_address(1, &setup.funding_wallet)
            .unwrap();

        coordinator.tick()?;
    }

    let news = coordinator.get_news()?;
    assert_eq!(news.monitor_news.len(), 10);

    let news = coordinator.get_news()?;

    assert_eq!(news.monitor_news.len(), 10);

    setup.bitcoind.stop()?;

    Ok(())
}

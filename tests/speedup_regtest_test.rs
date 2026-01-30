use bitcoin::{Amount, OutPoint};
use bitcoin_coordinator::{
    config::CoordinatorConfig,
    coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi},
    types::AckNews,
    AckMonitorNews, MonitorNews, TypesToMonitor,
};
use bitvmx_bitcoin_rpc::bitcoin_client::BitcoinClientApi;
use bitvmx_settings::settings::load_config_file;
use console::style;
use protocol_builder::types::{output::SpeedupData, Utxo};
use tracing::info;
use utils::generate_tx;

use crate::utils::{config_trace_aux, create_test_setup, TestSetupConfig};
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

// This test creates and dispatches two transactions in sequence, where each transaction is accelerated using a speedup (CPFP) mechanism.
// The funding for the second transaction comes from the change output of the first transaction.
// The test verifies that both transactions are successfully mined and confirmed, and asserts that the coordinator reports the expected news events.
#[test]
fn speedup_tx() -> Result<(), anyhow::Error> {
    config_trace_aux();

    let setup = create_test_setup(TestSetupConfig {
        blocks_mined: 101,
        bitcoind_flags: None,
    })?;

    let amount = Amount::from_sat(23450000);

    info!(
        "{} Funding address {:?}",
        style("Test").green(),
        setup.funding_wallet
    );

    let (funding_tx, funding_vout) = setup
        .bitcoin_client
        .fund_address(&setup.funding_wallet, amount)?;

    info!(
        "{} Funding tx address {:?}",
        style("Test").green(),
        setup.funding_wallet
    );

    let (funding_speedup, funding_speedup_vout) = setup
        .bitcoin_client
        .fund_address(&setup.funding_wallet, amount)?;

    info!(
        "{} Funding tx: {:?} | vout: {:?}",
        style("Test").green(),
        funding_tx.compute_txid(),
        funding_vout
    );

    info!(
        "{} Funding speed up tx: {:?} | vout: {:?}",
        style("Test").green(),
        funding_speedup.compute_txid(),
        funding_speedup_vout
    );

    let coordinator = BitcoinCoordinator::new_with_paths(
        &setup.config_bitcoin_client,
        setup.storage.clone(),
        setup.key_manager.clone(),
        None,
    )?;

    // Since we've already mined 102 blocks, we need to advance the coordinator by 102 ticks
    // so the indexer can catch up with the current blockchain height.
    for _ in 0..105 {
        coordinator.tick()?;
    }

    let (tx1, tx1_speedup_utxo) = generate_tx(
        OutPoint::new(funding_tx.compute_txid(), funding_vout),
        amount.to_sat(),
        setup.public_key,
        setup.key_manager.clone(),
        172,
    )?;

    let speedup_data = SpeedupData::new(tx1_speedup_utxo);

    let tx_context = "My tx".to_string();
    let tx_to_monitor =
        TypesToMonitor::Transactions(vec![tx1.compute_txid()], tx_context.clone(), None);
    coordinator.monitor(tx_to_monitor)?;

    // Dispatch the transaction through the bitcoin coordinator.
    coordinator.dispatch(tx1, Some(speedup_data), tx_context.clone(), None, None)?;

    // Add funding for speed up transaction
    coordinator.add_funding(Utxo::new(
        funding_speedup.compute_txid(),
        funding_speedup_vout,
        amount.to_sat(),
        &setup.public_key,
    ))?;

    // First tick dispatch the tx and CPFP speedup tx.
    coordinator.tick()?;

    // Mine a block to mine txs (tx1 and speedup tx)
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    // Detect txs (tx1 and speedup tx)
    coordinator.tick()?;

    let news = coordinator.get_news()?;

    if news.monitor_news.len() > 0 {
        info!(
            "{} News(#{:?})",
            style("Test").green(),
            news.monitor_news.len()
        );
        // Ack the news
        match news.monitor_news[0] {
            MonitorNews::Transaction(txid, _, _) => {
                let ack_news = AckMonitorNews::Transaction(txid, "My tx".to_string());
                coordinator.ack_news(AckNews::Monitor(ack_news))?;
            }
            _ => {
                assert!(false);
            }
        }
        assert!(true);
    } else {
        assert!(false);
    }

    let (funding_speedup_2, funding_speedup_vout_2) = setup
        .bitcoin_client
        .fund_address(&setup.funding_wallet, amount)?;

    let (tx2, tx2_speedup_utxo) = generate_tx(
        OutPoint::new(funding_speedup_2.compute_txid(), funding_speedup_vout_2),
        amount.to_sat(),
        setup.public_key,
        setup.key_manager.clone(),
        172,
    )?;

    let speedup_data = SpeedupData::new(tx2_speedup_utxo);

    let tx_to_monitor_2 =
        TypesToMonitor::Transactions(vec![tx2.compute_txid()], tx_context.clone(), None);
    coordinator.monitor(tx_to_monitor_2)?;

    coordinator.dispatch(tx2, Some(speedup_data), tx_context.clone(), None, None)?;

    // First tick dispatch the tx2 and create a speedup tx to be send
    coordinator.tick()?;

    // Second tick dispatch the speedup tx
    coordinator.tick()?;

    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    // Third tick detect the speedup tx2 + tx2 mined
    coordinator.tick()?;

    // Should be news.
    let news = coordinator.get_news()?;

    if news.monitor_news.len() > 0 {
        info!(
            "{} News(#{:?})",
            style("Test").green(),
            news.monitor_news.len()
        );
        assert!(true);
    } else {
        assert!(false);
    }

    setup.bitcoind.stop()?;

    Ok(())
}

#[test]
fn test_load_config_file() -> Result<(), anyhow::Error> {
    // Load the configuration file to verify that the keys in regtest.yaml are present and ensure the structure remains valid
    let settings =
        load_config_file::<CoordinatorConfig>(Some("config/coordinator_config.yaml".to_string()));
    assert!(settings.is_ok());

    Ok(())
}

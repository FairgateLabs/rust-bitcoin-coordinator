use bitcoin::Amount;
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitcoin_coordinator::types::{AckCoordinatorNews, CoordinatorNews};
use bitcoin_coordinator::TypesToMonitor;
use bitcoind::bitcoind::BitcoindFlags;
use bitvmx_bitcoin_rpc::bitcoin_client::BitcoinClientApi;
use std::rc::Rc;

use crate::utils::{config_trace_aux, create_test_setup, TestSetupConfig};
mod utils;

#[test]
fn stuck_in_mempool_test() -> Result<(), anyhow::Error> {
    config_trace_aux();

    let mut blocks_mined = 102;
    let setup = create_test_setup(TestSetupConfig {
        blocks_mined,
        bitcoind_flags: Some(BitcoindFlags {
            block_min_tx_fee: 0.0002, // High fee requirement to prevent mine the transaction (keep in mempool)
            ..Default::default()
        }),
    })?;

    let coordinator = Rc::new(BitcoinCoordinator::new_with_paths(
        &setup.config_bitcoin_client,
        setup.storage.clone(),
        setup.key_manager.clone(),
        None,
    )?);

    while !coordinator.is_ready()? {
        coordinator.tick()?;
    }

    let amount = Amount::from_sat(1000000);
    let (funding_tx, funding_vout) = setup
        .bitcoin_client
        .fund_address(&setup.funding_wallet, amount)?;

    // Create a transaction with very low fee so it stays in mempool
    let (tx, _) = crate::utils::generate_tx(
        bitcoin::OutPoint::new(funding_tx.compute_txid(), funding_vout),
        amount.to_sat(),
        setup.public_key,
        setup.key_manager.clone(),
        1500, // Low fee so it stays in mempool but is not mined with block_min_tx_fee
    )?;

    let tx_context = "Stuck test tx".to_string();
    let tx_to_monitor =
        TypesToMonitor::Transactions(vec![tx.compute_txid()], tx_context.clone(), None);
    coordinator.monitor(tx_to_monitor)?;

    // Dispatch transaction without speedup, with stuck threshold of 3 blocks
    let stuck_threshold = 3;
    coordinator.dispatch_without_speedup(
        tx.clone(),
        tx_context.clone(),
        None,
        None,
        stuck_threshold,
    )?;

    // Process the dispatch
    coordinator.tick()?;

    // Mine and tick blocks to trigger the stuck notification.
    // We need to mine at least `stuck_threshold` blocks after dispatch.
    for _ in 0..(stuck_threshold + 1) {
        setup
            .bitcoin_client
            .mine_blocks_to_address(1, &setup.funding_wallet)
            .unwrap();
        blocks_mined = blocks_mined + 1;

        coordinator.tick()?;
    }

    // After mining and ticking enough blocks, the ONLY news should be TransactionStuckInMempool.
    let news = coordinator.get_news()?;
    assert!(
        news.monitor_news.is_empty(),
        "Expected no monitor news, got {:?}",
        news.monitor_news
    );
    assert_eq!(
        news.coordinator_news.len(),
        1,
        "Expected exactly 1 coordinator news item, got {:?}",
        news.coordinator_news
    );

    match &news.coordinator_news[0] {
        CoordinatorNews::TransactionStuckInMempool(txid, context) => {
            assert_eq!(*txid, tx.compute_txid(), "Unexpected txid in stuck news");
            assert_eq!(context, &tx_context, "Unexpected context in stuck news");
        }
        other => {
            return Err(anyhow::anyhow!(
                "Expected TransactionStuckInMempool as the only news item, got {:?}",
                other
            ));
        }
    };

    // Ack the stuck news
    coordinator.ack_news(bitcoin_coordinator::types::AckNews::Coordinator(
        AckCoordinatorNews::TransactionStuckInMempool(tx.compute_txid()),
    ))?;

    // Mine a new block, tick, and verify there is no news.
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)
        .unwrap();

    coordinator.tick()?;

    let news_after_ack = coordinator.get_news()?;
    assert!(
        news_after_ack.monitor_news.is_empty(),
        "Expected no monitor news after ack, got {:?}",
        news_after_ack.monitor_news
    );
    assert!(
        news_after_ack.coordinator_news.is_empty(),
        "Expected no coordinator news after ack, got {:?}",
        news_after_ack.coordinator_news
    );

    setup.bitcoind.stop()?;

    Ok(())
}

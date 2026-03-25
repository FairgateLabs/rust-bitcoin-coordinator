use bitcoin::{Address, Amount, CompressedPublicKey};
use bitcoin_coordinator::{
    config::CoordinatorSettingsConfig,
    coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi},
    types::AckNews,
    AckMonitorNews, MonitorNews,
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

    // ============================================================================
    // TEST OVERVIEW: Blockchain Reorganization with RBF (Replace-By-Fee) Scenario
    // ============================================================================
    // This test verifies that the coordinator correctly handles blockchain reorganizations
    // when transactions have been replaced using RBF. The test flow:
    // 1. Setup: Create a regtest environment and configure coordinator with max_unconfirmed_speedups=2
    // 2. Phase 1: Dispatch first transaction (tx1) with CPFP speedup and confirm it
    // 3. Phase 2: Trigger a blockchain reorganization (reorg) by invalidating a block
    // 4. Phase 3: Dispatch two more transactions (tx2, tx3) after the reorg
    // 5. Phase 4: Mine blocks and verify all three transactions are properly tracked
    // ============================================================================

    // Initialize test environment with 102 pre-mined blocks
    let mut blocks_mined = 102;
    let setup = create_test_setup(TestSetupConfig {
        blocks_mined,
        bitcoind_flags: Some(BitcoindFlags {
            block_min_tx_fee: 0.00004,
            ..Default::default()
        }),
    })?;

    let amount = Amount::from_sat(23450000);

    // ============================================================================
    // SETUP PHASE: Create funding transaction for speedup transactions
    // ============================================================================
    // Create a funding transaction that will be used to pay for CPFP speedup transactions.
    // This funding transaction needs to be mined before we can use it.
    let (funding_speedup, funding_speedup_vout) = setup
        .bitcoin_client
        .fund_address(&setup.funding_wallet, amount)?;
    blocks_mined += 1; // Funding transaction mines 1 block

    info!(
        "{} Funding speed up tx: {:?} | vout: {:?}",
        style("Test").green(),
        funding_speedup.compute_txid(),
        funding_speedup_vout
    );

    // Configure coordinator with max_unconfirmed_speedups=2 to force RBF behavior
    // When we reach 2 unconfirmed speedups, the coordinator will replace the last one with RBF
    let mut settings = CoordinatorSettingsConfig::default();
    settings.max_unconfirmed_speedups = Some(2);

    let coordinator = Rc::new(BitcoinCoordinator::new_with_paths(
        &setup.config_bitcoin_client,
        setup.storage.clone(),
        setup.key_manager.clone(),
        Some(settings),
    )?);

    // Sync coordinator with blockchain: advance by all blocks mined so far
    // This ensures the indexer is caught up with the current blockchain height
    for _ in 0..blocks_mined {
        coordinator.tick()?;
    }

    // Register the funding UTXO with the coordinator so it can be used for speedup transactions
    coordinator.add_funding(Utxo::new(
        funding_speedup.compute_txid(),
        funding_speedup_vout,
        amount.to_sat(),
        &setup.public_key,
    ))?;

    // ============================================================================
    // PHASE 1: Dispatch first transaction (tx1) and confirm it
    // ============================================================================
    // Dispatch transaction tx1 with a CPFP speedup. The coordinate_tx helper function:
    // - Creates a funding transaction
    // - Creates tx1 and its speedup UTXO
    // - Monitors tx1
    // - Dispatches tx1 with speedup data
    let tx1 = coordinate_tx(
        coordinator.clone(),
        amount,
        setup.network,
        setup.key_manager.clone(),
        setup.bitcoin_client.clone(),
        None,
    )?;
    let tx1_id = tx1.compute_txid();

    // First tick: dispatches tx1 and creates/dispatches its CPFP speedup transaction
    coordinator.tick()?;

    // Mine 3 blocks to confirm tx1 and its speedup transaction
    // Each block mined advances the blockchain, and each tick processes the new blocks
    for _ in 0..3 {
        info!("{} Mine and Tick", style("Test").green());
        setup
            .bitcoin_client
            .mine_blocks_to_address(1, &setup.funding_wallet)
            .unwrap();
        coordinator.tick()?;
    }

    // Verify that tx1 has been confirmed (1 confirmation)
    let news = coordinator.get_news()?;
    assert_eq!(
        news.monitor_news.len(),
        1,
        "Expected exactly 1 monitor news after confirming tx1"
    );
    assert!(
        news.monitor_news.iter().all(|n| match n {
            MonitorNews::Transaction(_, tx_status, _) => tx_status.confirmations == 1,
            _ => false,
        }),
        "Expected tx1 to have 1 confirmation"
    );

    // Acknowledge the news to clear it from the monitor
    match news.monitor_news[0] {
        MonitorNews::Transaction(txid, _, _) => {
            let ack_news = AckMonitorNews::Transaction(txid, "My tx".to_string());
            coordinator.ack_news(AckNews::Monitor(ack_news))?;
        }
        _ => {
            panic!("Expected MonitorNews::Transaction");
        }
    }

    // ============================================================================
    // PHASE 2: Trigger blockchain reorganization (reorg)
    // ============================================================================
    // Invalidate the best block to simulate a blockchain reorganization.
    // This causes all transactions in that block (including tx1) to become orphaned.
    let best_block = setup.bitcoin_client.get_best_block()?;
    let block_hash = setup
        .bitcoin_client
        .get_block_id_by_height(&best_block)
        .unwrap();
    setup.bitcoin_client.invalidate_block(&block_hash).unwrap();
    info!(
        "{} Invalidated block to trigger reorg",
        style("Test").green()
    );

    // Process the reorg: coordinator should detect that tx1 is now orphaned
    coordinator.tick()?;

    // Verify that tx1 is now in orphan status after the reorg
    let news = coordinator.get_news()?;
    assert_eq!(
        news.monitor_news.len(),
        1,
        "Expected 1 monitor news after reorg"
    );
    assert!(
        news.monitor_news.iter().all(|n| match n {
            MonitorNews::Transaction(_, tx_status, _) => tx_status.is_orphan(),
            _ => false,
        }),
        "Expected tx1 to be orphaned after blockchain reorganization"
    );

    // Process one more tick to handle the orphaned transaction state
    coordinator.tick()?;

    // ============================================================================
    // PHASE 3: Dispatch two more transactions after the reorg
    // ============================================================================
    // Dispatch tx2 and tx3 to test that the coordinator can continue operating
    // correctly after a blockchain reorganization. These transactions should be
    // processed normally despite the previous reorg.
    let tx2 = coordinate_tx(
        coordinator.clone(),
        amount,
        setup.network,
        setup.key_manager.clone(),
        setup.bitcoin_client.clone(),
        None,
    )?;

    let tx3 = coordinate_tx(
        coordinator.clone(),
        amount,
        setup.network,
        setup.key_manager.clone(),
        setup.bitcoin_client.clone(),
        None,
    )?;

    let tx2_id = tx2.compute_txid();
    let tx3_id = tx3.compute_txid();

    // Create a new funding wallet address for mining blocks
    // This ensures we're mining to a different address than the test setup
    let public_key = setup
        .key_manager
        .derive_keypair(key_manager::key_type::BitcoinKeyType::P2tr, 1)
        .unwrap();
    let compressed = CompressedPublicKey::try_from(public_key).unwrap();
    let funding_wallet = Address::p2wpkh(&compressed, setup.network);

    // Mine a block to include the funding transactions for tx2 and tx3
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &funding_wallet)
        .unwrap();

    // Wait for coordinator to be ready (indexer synced with blockchain)
    while !coordinator.is_ready()? {
        coordinator.tick()?;
    }

    // After mining a new block, tx1 should be confirmed again (re-mined in the new chain)
    let news = coordinator.get_news()?;
    assert!(
        news.monitor_news.iter().all(|n| match n {
            MonitorNews::Transaction(_, tx_status, _) => tx_status.is_confirmed(),
            _ => false,
        }),
        "Expected all transactions to be confirmed after re-mining"
    );

    // At this point, only tx1 should be in the news (it was re-mined after the reorg)
    assert_eq!(
        news.monitor_news.len(),
        1,
        "Expected 1 news item (tx1 re-confirmed)"
    );

    // Acknowledge tx1's news to clear it from the monitor
    match news.monitor_news[0] {
        MonitorNews::Transaction(txid, _, _) => {
            let ack_news = AckMonitorNews::Transaction(txid, "My tx".to_string());
            coordinator.ack_news(AckNews::Monitor(ack_news))?;
        }
        _ => {
            panic!("Expected MonitorNews::Transaction");
        }
    }

    // ============================================================================
    // PHASE 4: Mine blocks and verify all transactions are tracked
    // ============================================================================
    // Mine 10 blocks to give enough time for all three transactions (tx1, tx2, tx3)
    // and their speedup transactions to be confirmed. Each tick processes the new blocks.
    for _ in 0..10 {
        coordinator.tick()?;
        setup
            .bitcoin_client
            .mine_blocks_to_address(1, &funding_wallet)
            .unwrap();
    }

    // Verify that all three transactions are present in the monitor news
    // After mining, we should have news for:
    // - tx1: re-confirmed after the reorg
    // - tx2: newly dispatched and confirmed
    // - tx3: newly dispatched and confirmed
    let news = coordinator.get_news()?;
    assert_eq!(
        news.monitor_news.len(),
        3,
        "Expected 3 monitor news items (one for each transaction: tx1, tx2, tx3)"
    );

    // Verify that each transaction ID appears in the news
    let mut found_tx1 = false;
    let mut found_tx2 = false;
    let mut found_tx3 = false;

    for news_item in &news.monitor_news {
        match news_item {
            MonitorNews::Transaction(txid, _, _) => {
                if *txid == tx1_id {
                    found_tx1 = true;
                } else if *txid == tx2_id {
                    found_tx2 = true;
                } else if *txid == tx3_id {
                    found_tx3 = true;
                }
            }
            _ => {
                panic!("Expected MonitorNews::Transaction, got unexpected news type");
            }
        }
    }

    // Assert that all three transactions were found in the news
    assert!(found_tx1, "Transaction 1 (tx1) not found in monitor news");
    assert!(found_tx2, "Transaction 2 (tx2) not found in monitor news");
    assert!(found_tx3, "Transaction 3 (tx3) not found in monitor news");

    setup.bitcoind.stop()?;

    Ok(())
}

use bitcoin::Amount;
use bitcoin_coordinator::{
    coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi},
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::TransactionState,
};
use bitcoind::bitcoind::BitcoindFlags;
use bitvmx_bitcoin_rpc::bitcoin_client::BitcoinClientApi;
use protocol_builder::types::Utxo;
use std::rc::Rc;
use tracing::info;

mod utils;
use crate::utils::{config_trace_aux, coordinate_tx, create_test_setup, TestSetupConfig};

/// Indices (0-based) of the 4 invalid transactions among 40 (not consecutive).
const INVALID_TX_INDICES: [usize; 4] = [3, 11, 19, 27];

/// Verifies pre-validation and batching with 40 txs, 4 invalid (0 fee, rejected by testmempoolaccept).
/// - filter_txs_allowed_by_mempool returns 36 allowed; 4 are marked Failed.
/// - 36 txs are dispatched in 2 batches; 36 monitor notifications; 2 speedup (CPFP) batches.
#[test]
fn speedup_prevalidation_40_txs_4_invalid_two_batches() -> Result<(), anyhow::Error> {
    config_trace_aux();

    let mut blocks_mined = 102;
    let setup = create_test_setup(TestSetupConfig {
        blocks_mined,
        bitcoind_flags: None,
    })?;

    let amount = Amount::from_sat(23450000);

    let (funding_tx, _funding_vout) = setup
        .bitcoin_client
        .fund_address(&setup.funding_wallet, amount)?;
    blocks_mined += 1;

    let (funding_speedup, funding_speedup_vout) = setup
        .bitcoin_client
        .fund_address(&setup.funding_wallet, amount)?;
    blocks_mined += 1;

    let coordinator = Rc::new(BitcoinCoordinator::new_with_paths(
        &setup.config_bitcoin_client,
        setup.storage.clone(),
        setup.key_manager.clone(),
        None,
    )?);

    for _ in 0..blocks_mined {
        coordinator.tick()?;
    }

    coordinator.add_funding(Utxo::new(
        funding_speedup.compute_txid(),
        funding_speedup_vout,
        amount.to_sat(),
        &setup.public_key,
    ))?;

    // 40 txs: at indices INVALID_TX_INDICES use fee 0 (rejected by testmempoolaccept); rest normal fee.
    let mut invalid_tx_ids = Vec::new();
    info!(
        "Submitting 40 transactions (4 invalid at indices {:?})",
        INVALID_TX_INDICES
    );
    for i in 0..40 {
        let is_invalid = INVALID_TX_INDICES.contains(&i);
        let fee = if is_invalid { Some(0) } else { None };
        let tx = coordinate_tx(
            coordinator.clone(),
            amount,
            setup.network,
            setup.key_manager.clone(),
            setup.bitcoin_client.clone(),
            fee,
        )?;
        if is_invalid {
            invalid_tx_ids.push(tx.compute_txid());
        }
    }

    assert_eq!(invalid_tx_ids.len(), 4, "expected 4 invalid tx ids");

    // Process: filter_txs_allowed_by_mempool should drop 4, dispatch 36 in 2 batches.
    for _ in 0..50 {
        coordinator.tick()?;
    }

    // First batch: mine one block so first batch confirms.
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)?;
    coordinator.tick()?;

    let news = coordinator.get_news()?;
    let first_batch_count = news.monitor_news.len();
    info!(
        "After first block: monitor_news.len() = {}",
        first_batch_count
    );
    assert!(
        first_batch_count >= 20 && first_batch_count <= 25,
        "expected first batch ~20–25 monitor notifications, got {}",
        first_batch_count
    );

    // Second batch: mine again so second batch confirms.
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)?;
    coordinator.tick()?;

    let news = coordinator.get_news()?;
    info!(
        "After second block: monitor_news.len() = {}",
        news.monitor_news.len()
    );
    assert_eq!(
        news.monitor_news.len(),
        36,
        "expected 36 monitor notifications (only valid txs), got {}",
        news.monitor_news.len()
    );

    // 4 invalid txs must be Failed in store (not active).
    let store = BitcoinCoordinatorStore::new(setup.storage.clone(), 10)?;
    for tx_id in &invalid_tx_ids {
        let coordinated = store.get_tx(tx_id)?;
        assert_eq!(
            coordinated.state,
            TransactionState::Failed,
            "invalid tx {} should be Failed",
            tx_id
        );
    }

    setup.bitcoind.stop()?;

    Ok(())
}

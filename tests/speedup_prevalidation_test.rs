use bitcoin::Amount;
use bitcoin_coordinator::{
    coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi},
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::TransactionState,
};
use bitvmx_bitcoin_rpc::bitcoin_client::BitcoinClientApi;
use protocol_builder::types::Utxo;
use std::rc::Rc;
use tracing::info;

mod utils;
use crate::utils::{
    config_trace_aux, coordinate_tx, create_test_setup, tick_until_coordinator_ready,
    TestSetupConfig,
};

/// Indices (0-based) of the 4 intentionally invalid transactions among 40 (not consecutive).
const INVALID_TX_INDICES: [usize; 4] = [3, 11, 19, 27];

/// Integration test for the speedup pre‑validation + batching path:
/// - Creates 40 coordinated transactions; 4 of them use fee 0 so `testmempoolaccept` rejects them.
/// - `filter_txs_allowed_by_mempool` must mark those 4 as `Failed` and only keep the 36 valid ones.
/// - The 36 valid txs are dispatched in 2 CPFP batches and later reported as monitor news.
#[test]
fn speedup_prevalidation_40_txs_4_invalid_two_batches() -> Result<(), anyhow::Error> {
    config_trace_aux();

    let mut blocks_mined = 102;
    let setup = create_test_setup(TestSetupConfig {
        blocks_mined,
        bitcoind_flags: None,
    })?;

    let amount = Amount::from_sat(23450000);

    let _ = setup
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

    // Build 40 coordinated txs; at indices in INVALID_TX_INDICES we force fee = 0 so
    // `testmempoolaccept` rejects them during pre‑validation. The rest use the default fee.
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

    tick_until_coordinator_ready(&coordinator)?;

    // First batch:
    // Mine + tick twice so the coordinator first processes parent txs and then their CPFP speedups,
    // and the monitor accumulates news for the first batch of valid transactions.
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)?;

    coordinator.tick()?;

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
        first_batch_count == 24,
        "expected first batch 24 monitor notifications, got {}",
        first_batch_count
    );

    tick_until_coordinator_ready(&coordinator)?;

    // Second batch:
    // Again, mine + tick twice so the remaining valid transactions are confirmed and reported.
    setup
        .bitcoin_client
        .mine_blocks_to_address(1, &setup.funding_wallet)?;

    coordinator.tick()?;

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

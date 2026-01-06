use bitcoin::{Address, Amount, CompressedPublicKey, Network, OutPoint};
use bitcoind::bitcoind::BitcoindFlags;
use bitvmx_bitcoin_rpc::bitcoin_client::BitcoinClientApi;
use console::style;
use protocol_builder::{
    builder::ProtocolBuilder,
    types::{output::SpeedupData, Utxo},
};
use std::{rc::Rc, vec};
use tracing::info;
use utils::generate_tx;

use crate::utils::{config_trace_aux, create_test_setup, TestSetupConfig};
mod utils;

#[test]
fn replacement_cycling_test() -> Result<(), anyhow::Error> {
    config_trace_aux();

    let setup = create_test_setup(TestSetupConfig {
        blocks_mined: 101,
        bitcoind_flags: Some(BitcoindFlags {
            block_min_tx_fee: 0.00002,
            ..Default::default()
        }),
    })?;

    // This test uses BitcoinClient directly (not Rc), so we need to extract it
    let bitcoin_client = Rc::try_unwrap(setup.bitcoin_client)
        .unwrap_or_else(|_| panic!("Expected single owner of BitcoinClient"));
    let public_key = setup.public_key;
    let funding_wallet = setup.funding_wallet;
    let key_manager = setup.key_manager;

    let amount = Amount::from_sat(23450000);
    info!(
        "{} Funding address {:?}",
        style("Test").green(),
        funding_wallet
    );

    info!(
        "{} Funding tx address {:?}",
        style("Test").green(),
        funding_wallet
    );
    let (funding_tx, funding_vout) = bitcoin_client.fund_address(&funding_wallet, amount)?;

    let (bob_funding, bob_funding_vout) = bitcoin_client.fund_address(&funding_wallet, amount)?;
    let (mallory_funding, mallory_funding_vout) =
        bitcoin_client.fund_address(&funding_wallet, amount)?;

    info!(
        "{} Funding tx: {:?} | vout: {:?}",
        style("Test").green(),
        funding_tx.compute_txid(),
        funding_vout
    );

    info!(
        "{} Funding speed up tx: {:?} | vout: {:?}",
        style("Test").green(),
        bob_funding.compute_txid(),
        bob_funding_vout
    );

    let (tx1, tx1_speedup_utxo) = generate_tx(
        OutPoint::new(funding_tx.compute_txid(), funding_vout),
        amount.to_sat(),
        public_key,
        key_manager.clone(),
        172,
    )?;

    let speedup_data = SpeedupData::new(tx1_speedup_utxo);
    let utxos = vec![speedup_data];

    let bob_funding_utxo = Utxo::new(
        bob_funding.compute_txid(),
        bob_funding_vout,
        amount.to_sat(),
        &public_key,
    );

    let mallory_funding_utxo = Utxo::new(
        mallory_funding.compute_txid(),
        mallory_funding_vout,
        amount.to_sat(),
        &public_key,
    );

    let bob_cpfp_tx = (ProtocolBuilder {}).speedup_transactions(
        &utxos,
        bob_funding_utxo,
        &public_key,
        10000,
        &key_manager,
    )?;

    let mallory_cpfp_tx = (ProtocolBuilder {}).speedup_transactions(
        &utxos,
        mallory_funding_utxo.clone(),
        &public_key,
        12000,
        &key_manager,
    )?;

    let mallory_tx = (ProtocolBuilder {}).speedup_transactions(
        &vec![],
        mallory_funding_utxo,
        &public_key,
        13000,
        &key_manager,
    )?;

    bitcoin_client.send_transaction(&tx1)?;
    bitcoin_client.send_transaction(&bob_cpfp_tx)?;
    bitcoin_client.send_transaction(&mallory_cpfp_tx)?;
    bitcoin_client.send_transaction(&mallory_tx)?;

    bitcoin_client.mine_blocks_to_address(10, &funding_wallet)?;

    info!("Tx1: {:?}", tx1.compute_txid());
    let tx1_status = bitcoin_client.get_transaction(&tx1.compute_txid())?;
    let info = bitcoin_client.get_raw_transaction_info(&tx1.compute_txid())?;
    assert_eq!(tx1_status.is_some(), true);
    assert_eq!(info.confirmations, None);

    info!("Bob CPFP: {:?}", bob_cpfp_tx.compute_txid());
    let bob_cpfp_status = bitcoin_client.get_transaction(&bob_cpfp_tx.compute_txid())?;
    assert_eq!(bob_cpfp_status.is_some(), false);

    info!("Mallory CPFP: {:?}", mallory_cpfp_tx.compute_txid());
    let mallory_cpfp_status = bitcoin_client.get_transaction(&mallory_cpfp_tx.compute_txid())?;
    assert_eq!(mallory_cpfp_status.is_some(), false);

    info!(
        "Mallory tx to take away funds: {:?}",
        mallory_tx.compute_txid()
    );
    let mallory_tx_status = bitcoin_client.get_transaction(&mallory_tx.compute_txid())?;
    let info = bitcoin_client.get_raw_transaction_info(&mallory_tx.compute_txid())?;
    assert_eq!(mallory_tx_status.is_some(), true);
    assert_eq!(info.confirmations, Some(10));

    setup.bitcoind.stop()?;

    Ok(())
}

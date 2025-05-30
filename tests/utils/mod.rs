use bitcoin::secp256k1::{All, SecretKey};
use bitcoin::{
    absolute, key::Secp256k1, secp256k1::Message, sighash::SighashCache, transaction, Amount,
    EcdsaSighashType, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
};
use bitcoin::{Address, CompressedPublicKey, Network, PublicKey, Txid, WPubkeyHash};
use bitcoin_coordinator::errors::TxBuilderHelperError;
use bitcoin_coordinator::storage::BitcoinCoordinatorStore;
use bitcoin_coordinator::TypesToMonitor;
use bitcoincore_rpc::{json::GetTransactionResult, Auth, Client, RpcApi};
use bitvmx_bitcoin_rpc::bitcoin_client::MockBitcoinClient;
use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use bitvmx_transaction_monitor::monitor::MockMonitorApi;
use key_manager::config::KeyManagerConfig;
use key_manager::create_key_manager_from_config;
use key_manager::key_manager::KeyManager;
use key_manager::key_store::KeyStore;
use protocol_builder::types::Utxo;
use std::rc::Rc;
use std::str::FromStr;
use storage_backend::storage::Storage;
use storage_backend::storage_config::StorageConfig;

pub fn clear_output() {
    let _ = std::fs::remove_dir_all("test_output");
}

pub fn clear_db(path: &str) {
    let _ = std::fs::remove_dir_all(path);
}

pub fn generate_random_string() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..10).map(|_| rng.gen_range('a'..='z')).collect()
}

pub fn get_mocks() -> (
    MockMonitorApi,
    BitcoinCoordinatorStore,
    MockBitcoinClient,
    Rc<KeyManager>,
) {
    let mock_monitor = MockMonitorApi::new();
    let path = format!("test_output/test/{}", generate_random_string());
    let config = StorageConfig::new(path, None);
    let storage = Rc::new(Storage::new(&config).unwrap());
    let store = BitcoinCoordinatorStore::new(storage.clone()).unwrap();
    let bitcoin_client = MockBitcoinClient::new();
    let config = KeyManagerConfig::new(Network::Regtest.to_string(), None, None, None);
    let key_store = KeyStore::new(storage.clone());
    let key_manager =
        Rc::new(create_key_manager_from_config(&config, key_store, storage.clone()).unwrap());

    (mock_monitor, store, bitcoin_client, key_manager)
}

pub fn get_mock_data(
    key_manager: Rc<KeyManager>,
) -> (TypesToMonitor, Transaction, Utxo, Txid, String, Utxo) {
    let public_key = key_manager.derive_keypair(0).unwrap();

    let new_funding_tx_id =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200a").unwrap();

    let funding_utxo = Utxo::new(new_funding_tx_id, 0, 10000000, &public_key);

    let tx = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![],
        output: vec![],
    };

    let tx_id = tx.compute_txid();
    let context_data = "My context monitor".to_string();
    let to_monitor = TypesToMonitor::Transactions(vec![tx_id], context_data.clone());

    let speedup_utxo = Utxo::new(tx_id, 0, 10000000, &public_key);

    (
        to_monitor,
        tx,
        funding_utxo,
        tx_id,
        context_data,
        speedup_utxo,
    )
}

pub fn generate_tx(
    rpc_config: &RpcConfig,
    network: Network,
) -> Result<Transaction, TxBuilderHelperError> {
    let secp: Secp256k1<All> = Secp256k1::new();
    let sk = SecretKey::new(&mut rand::thread_rng());
    let pk = bitcoin::PublicKey::new(sk.public_key(&secp));
    let wpkh = pk.wpubkey_hash().expect("key is compressed");
    let compressed = CompressedPublicKey::try_from(pk).unwrap();
    let address = Address::p2wpkh(&compressed, network).as_unchecked().clone();
    let address_checked = address.require_network(network).unwrap();

    // build and send a mock transaction that we can spend in our drp transaction
    let tx_info = make_mock_output(rpc_config, &address_checked)?;
    let spent_amount = tx_info.amount.unsigned_abs();

    let cpfp_fee = Amount::from_sat(100);
    let cpfp_amount = Amount::from_sat(100);

    // reciduo.
    let drp_amount = spent_amount - cpfp_fee - cpfp_amount;

    // The input for the transaction we are constructing.
    let input = TxIn {
        previous_output: OutPoint {
            txid: tx_info.info.txid,
            vout: tx_info
                .details
                .first()
                .expect("No details found for transaction")
                .vout,
        },
        script_sig: ScriptBuf::default(), // For a p2wpkh script_sig is empty.
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: Witness::default(), // Filled in after signing.
    };

    // The drp output. For this example, we just pay back to the user.
    let drp = TxOut {
        value: drp_amount,
        script_pubkey: address_checked.script_pubkey(),
    };

    // The cpfp output is locked to a key controlled by the user.
    let cpfp = TxOut {
        value: cpfp_amount,
        script_pubkey: ScriptBuf::new_p2wpkh(&wpkh),
    };

    let tx = build_transaction(vec![input], vec![drp, cpfp], spent_amount, &wpkh, &sk)?;

    Ok(tx)
}

pub fn make_mock_output(
    rpc_config: &RpcConfig,
    address: &Address,
) -> Result<GetTransactionResult, TxBuilderHelperError> {
    let client = Client::new(
        rpc_config.url.as_str(),
        Auth::UserPass(
            rpc_config.username.as_str().to_string(),
            rpc_config.password.as_str().to_string(),
        ),
    )?;

    // fund the user address
    let txid = client.send_to_address(
        address,
        Amount::from_sat(100_000_000), // 1 BTC
        None,
        None,
        None,
        None,
        None,
        None,
    )?;

    // get transaction details
    Ok(client.get_transaction(&txid, Some(true))?)
}

/// Builds a transaction with a single input and multiple outputs.
pub fn build_transaction(
    inputs: Vec<TxIn>,
    outputs: Vec<TxOut>,
    spent_amount: Amount,
    wpkh: &WPubkeyHash,
    sk: &SecretKey,
) -> Result<Transaction, TxBuilderHelperError> {
    // TODO support multiple inputs and accounts (we only support one input, for now)
    // The transaction we want to sign and broadcast.
    let mut unsigned_tx = Transaction {
        version: transaction::Version::TWO,  // Post BIP-68.
        lock_time: absolute::LockTime::ZERO, // Ignore the locktime.
        input: inputs,                       // Input goes into index 0.
        output: outputs,                     // cpfp output is always index 0.
    };
    let input_index = 0;

    // Get the sighash to sign.
    let sighash_type = EcdsaSighashType::All;
    let mut sighasher = SighashCache::new(&mut unsigned_tx);
    let sighash = sighasher
        .p2wpkh_signature_hash(
            input_index,
            &ScriptBuf::new_p2wpkh(&wpkh),
            spent_amount,
            sighash_type,
        )
        .expect("failed to create sighash");

    // Sign the sighash using the secp256k1 library (exported by rust-bitcoin).
    let msg = Message::from(sighash);
    let secp = Secp256k1::new();
    let signature = secp.sign_ecdsa(&msg, sk);

    // Update the witness stack.
    let signature = bitcoin::ecdsa::Signature {
        signature,
        sighash_type,
    };
    let pk = sk.public_key(&secp);
    *sighasher.witness_mut(input_index).unwrap() = Witness::p2wpkh(&signature, &pk);

    // Get the signed transaction.
    Ok(sighasher.into_transaction().to_owned())
}

use bitcoin::secp256k1::SecretKey;
use bitcoin::{
    absolute, key::Secp256k1, secp256k1::Message, sighash::SighashCache, transaction, Amount,
    EcdsaSighashType, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
};
use bitcoin::{Address, Network, PrivateKey, PublicKey, Txid};
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
use protocol_builder::builder::{Protocol, ProtocolBuilder};
use protocol_builder::types::connection::InputSpec;
use protocol_builder::types::input::{SighashType, SpendMode};
use protocol_builder::types::{InputArgs, OutputType, Utxo};
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
    funding_outpoint: OutPoint,
    origin_amount: u64,
    origin_pubkey: PublicKey,
    key_manager: Rc<KeyManager>,
) -> Result<Transaction, TxBuilderHelperError> {
    let amount = 10000;
    let fee = 1000;

    let tx = create_transfer_transaction(
        funding_outpoint,
        origin_amount,
        origin_pubkey,
        origin_pubkey,
        amount,
        fee,
        key_manager,
    );

    Ok(tx)
}

fn create_transfer_transaction(
    outpoint: OutPoint,
    origin_amount: u64,
    origin_pubkey: PublicKey,
    to_pubkey: PublicKey,
    amount: u64,
    fee: u64,
    key_manager: Rc<KeyManager>,
) -> Transaction {
    // Create the  for funding
    let external_output = OutputType::segwit_key(origin_amount, &origin_pubkey).unwrap();

    let mut protocol = Protocol::new("transfer_tx");
    protocol.add_external_transaction("origin").unwrap();
    protocol
        .add_unkwnoun_outputs("origin", outpoint.vout)
        .unwrap();
    protocol
        .add_connection(
            "origin_tx_transfer",
            "origin",
            external_output.clone().into(),
            "transfer",
            InputSpec::Auto(SighashType::ecdsa_all(), SpendMode::Segwit),
            None,
            Some(outpoint.txid),
        )
        .unwrap();

    let transfer_output = OutputType::segwit_key(amount, &to_pubkey).unwrap();

    protocol
        .add_transaction_output("transfer", &transfer_output)
        .unwrap();

    let change = origin_amount - amount - fee;

    if change > 0 {
        let change_output = OutputType::segwit_key(change, &origin_pubkey).unwrap();
        protocol
            .add_transaction_output("transfer", &change_output)
            .unwrap();
    }

    protocol.build_and_sign(&key_manager, "id").unwrap();

    let signature = protocol
        .input_ecdsa_signature("transfer", 0)
        .unwrap()
        .unwrap();

    let mut spending_args = InputArgs::new_segwit_args();
    spending_args.push_ecdsa_signature(signature).unwrap();

    let result = protocol
        .transaction_to_send("transfer", &[spending_args])
        .unwrap();

    result
}

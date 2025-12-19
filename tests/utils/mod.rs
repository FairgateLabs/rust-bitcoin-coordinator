use bitcoin::{absolute, transaction, Address, Amount, CompressedPublicKey, OutPoint, Transaction};
use bitcoin::{Network, PublicKey, Txid};
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitcoin_coordinator::errors::TxBuilderHelperError;
use bitcoin_coordinator::storage::BitcoinCoordinatorStore;
use bitcoin_coordinator::TypesToMonitor;
use bitvmx_bitcoin_rpc::bitcoin_client::{BitcoinClient, BitcoinClientApi, MockBitcoinClient};
use bitvmx_transaction_monitor::monitor::MockMonitorApi;
use key_manager::config::KeyManagerConfig;
use key_manager::create_key_manager_from_config;
use key_manager::key_manager::KeyManager;
use key_manager::key_store::KeyStore;
use key_manager::key_type::BitcoinKeyType;
use protocol_builder::builder::Protocol;
use protocol_builder::types::connection::InputSpec;
use protocol_builder::types::input::{SighashType, SpendMode};
use protocol_builder::types::output::SpeedupData;
use protocol_builder::types::{InputArgs, OutputType, Utxo};
use std::rc::Rc;
use std::str::FromStr;
use storage_backend::storage::Storage;
use storage_backend::storage_config::StorageConfig;
use tracing_subscriber::EnvFilter;

pub fn clear_output() {
    let _ = std::fs::remove_dir("test_output/");
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
    const MAX_RETRIES: u32 = 3;
    const RETRY_INTERVAL: u64 = 2;
    let mock_monitor = MockMonitorApi::new();
    let path = format!("test_output/test/{}", generate_random_string());
    let storage_config = StorageConfig::new(path, None);
    let storage = Rc::new(Storage::new(&storage_config).unwrap());
    let store =
        BitcoinCoordinatorStore::new(storage.clone(), 1, MAX_RETRIES, RETRY_INTERVAL).unwrap();
    let bitcoin_client = MockBitcoinClient::new();
    let key_manager_config = KeyManagerConfig::new(Network::Regtest.to_string(), None, None);
    let key_manager =
        Rc::new(create_key_manager_from_config(&key_manager_config, &storage_config).unwrap());

    (mock_monitor, store, bitcoin_client, key_manager)
}

pub fn get_mock_data(
    key_manager: Rc<KeyManager>,
) -> (TypesToMonitor, Transaction, Utxo, Txid, String, Utxo) {
    let public_key = key_manager.derive_keypair(BitcoinKeyType::P2tr, 0).unwrap();

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
    fee: u64,
) -> Result<(Transaction, Utxo), TxBuilderHelperError> {
    Ok(create_tx_to_speedup(
        funding_outpoint,
        origin_amount,
        origin_pubkey,
        origin_pubkey,
        10000,
        fee,
        key_manager,
    ))
}

fn create_tx_to_speedup(
    outpoint: OutPoint,
    origin_amount: u64,
    origin_pubkey: PublicKey,
    to_pubkey: PublicKey,
    amount: u64,
    fee: u64,
    key_manager: Rc<KeyManager>,
) -> (Transaction, Utxo) {
    // Create the  for funding
    let external_output = OutputType::segwit_key(origin_amount, &origin_pubkey).unwrap();

    let mut protocol = Protocol::new("transfer_tx");
    protocol.add_external_transaction("origin").unwrap();
    protocol
        .add_unknown_outputs("origin", outpoint.vout)
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

    // Add the output for the transfer transaction
    let transfer_output = OutputType::segwit_key(amount, &to_pubkey).unwrap();
    protocol
        .add_transaction_output("transfer", &transfer_output)
        .unwrap();

    // Add the output for the speed up transaction
    let speedup_amount = 294; // This is the minimal non-dust output.
    let speedup_output = OutputType::segwit_key(speedup_amount, &to_pubkey).unwrap();

    protocol
        .add_transaction_output("transfer", &speedup_output)
        .unwrap();

    // Add the output for the change
    let change = origin_amount - amount - fee - speedup_amount;
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

    let speedup_utxo = Utxo::new(result.compute_txid(), 1, speedup_amount, &to_pubkey);

    (result, speedup_utxo)
}

pub fn create_store() -> BitcoinCoordinatorStore {
    const MAX_RETRIES: u32 = 3;
    const RETRY_INTERVAL: u64 = 2;
    let path = format!("test_output/speedup/{}", generate_random_string());
    let storage_config = StorageConfig::new(path, None);
    let storage = Rc::new(Storage::new(&storage_config).unwrap());
    BitcoinCoordinatorStore::new(storage, 10, MAX_RETRIES, RETRY_INTERVAL).unwrap()
}

pub fn config_trace_aux() {
    let default_modules = [
        "info",
        "libp2p=off",
        "bitvmx_transaction_monitor=off",
        "bitcoin_indexer=off",
        "bitcoin_coordinator=info",
        "bitcoin_client=off",
        "p2p_protocol=off",
        "p2p_handler=off",
        "tarpc=off",
        "key_manager=off",
        "memory=off",
    ];

    let filter = EnvFilter::builder()
        .parse(default_modules.join(","))
        .expect("Invalid filter");

    tracing_subscriber::fmt()
        //.without_time()
        //.with_ansi(false)
        .with_target(true)
        .with_env_filter(filter)
        .init();
}

pub fn coordinate_tx(
    coordinator: Rc<BitcoinCoordinator>,
    amount: Amount,
    network: Network,
    key_manager: Rc<KeyManager>,
    bitcoin_client: Rc<BitcoinClient>,
    fee: Option<u64>,
) -> Result<(), anyhow::Error> {
    let fee = fee.unwrap_or(172);

    // Create a funding wallet
    // Fund the funding wallet
    // Create a tx1 and a speedup utxo for tx1
    // Monitor tx1
    // Dispatch tx1
    // First tick dispatch the tx and create and dispatch a speedup tx
    let public_key = key_manager.derive_keypair(BitcoinKeyType::P2tr, 0).unwrap();
    let compressed = CompressedPublicKey::try_from(public_key).unwrap();
    let funding_wallet = Address::p2wpkh(&compressed, network);

    let (funding_tx, funding_vout) = bitcoin_client.fund_address(&funding_wallet, amount)?;

    let (tx1, tx1_speedup_utxo) = generate_tx(
        OutPoint::new(funding_tx.compute_txid(), funding_vout),
        amount.to_sat(),
        public_key,
        key_manager.clone(),
        fee,
    )?;

    let speedup_data = SpeedupData::new(tx1_speedup_utxo);

    let tx_context = "My tx".to_string();
    let tx_to_monitor = TypesToMonitor::Transactions(vec![tx1.compute_txid()], tx_context.clone());
    coordinator.monitor(tx_to_monitor)?;

    // Dispatch the transaction through the bitcoin coordinator.
    coordinator.dispatch(tx1.clone(), Some(speedup_data), tx_context.clone(), None)?;

    Ok(())
}

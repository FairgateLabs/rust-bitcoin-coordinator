use bitcoin::{absolute, transaction, Address, Amount, CompressedPublicKey, OutPoint, Transaction};
use bitcoin::{Network, PublicKey, Txid};
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitcoin_coordinator::errors::TxBuilderHelperError;
use bitcoin_coordinator::storage::BitcoinCoordinatorStore;
use bitcoin_coordinator::TypesToMonitor;
use bitcoind::bitcoind::{Bitcoind, BitcoindFlags};
use bitcoind::config::BitcoindConfig;
use bitvmx_bitcoin_rpc::bitcoin_client::{BitcoinClient, BitcoinClientApi, MockBitcoinClient};
use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use bitvmx_transaction_monitor::monitor::MockMonitorApi;
use console::style;
use key_manager::config::KeyManagerConfig;
use key_manager::create_key_manager_from_config;
use key_manager::key_manager::KeyManager;
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
use tracing::info;
use tracing_subscriber::EnvFilter;

pub fn clear_output() {
    let _ = std::fs::remove_dir("test_output/");
}

pub fn clear_db(path: &str) {
    let _ = std::fs::remove_dir_all(path);
}

pub fn generate_random_string() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..10).map(|_| rng.random_range('a'..='z')).collect()
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
    let path_key_manager = format!("test_output/test/key_manager/{}", generate_random_string());
    let key_manager_storage_config = StorageConfig::new(path_key_manager, None);
    let key_manager_config = KeyManagerConfig::new(Network::Regtest.to_string(), None, None);
    let key_manager = Rc::new(
        create_key_manager_from_config(&key_manager_config, &key_manager_storage_config).unwrap(),
    );
    let path_storage = format!("test_output/test/storage/{}", generate_random_string());
    let storage_config = StorageConfig::new(path_storage, None);
    let storage = Rc::new(Storage::new(&storage_config).unwrap());
    let store =
        BitcoinCoordinatorStore::new(storage.clone(), 1, MAX_RETRIES, RETRY_INTERVAL).unwrap();
    let bitcoin_client = MockBitcoinClient::new();

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
    let to_monitor = TypesToMonitor::Transactions(vec![tx_id], context_data.clone(), None);

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
    let external_output = OutputType::segwit_key(origin_amount.into(), &origin_pubkey).unwrap();

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
    let transfer_output = OutputType::segwit_key(amount.into(), &to_pubkey).unwrap();
    protocol
        .add_transaction_output("transfer", &transfer_output)
        .unwrap();

    // Add the output for the speed up transaction
    let speedup_amount = 540; // This is the minimal non-dust output.
    let speedup_output = OutputType::segwit_key(speedup_amount.into(), &to_pubkey).unwrap();

    protocol
        .add_transaction_output("transfer", &speedup_output)
        .unwrap();

    // Add the output for the change
    let change = origin_amount - amount - fee - speedup_amount;
    if change > 0 {
        let change_output = OutputType::segwit_key(change.into(), &origin_pubkey).unwrap();

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

    // Try to set the global default, but ignore if it's already set
    // This allows multiple tests to call this function without panicking
    let _ = tracing_subscriber::fmt()
        .with_target(true)
        .with_env_filter(filter)
        .try_init();
}

pub fn coordinate_tx(
    coordinator: Rc<BitcoinCoordinator>,
    amount: Amount,
    network: Network,
    key_manager: Rc<KeyManager>,
    bitcoin_client: Rc<BitcoinClient>,
    fee: Option<u64>,
) -> Result<Transaction, anyhow::Error> {
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
    let tx_to_monitor =
        TypesToMonitor::Transactions(vec![tx1.compute_txid()], tx_context.clone(), None);
    coordinator.monitor(tx_to_monitor)?;

    // Dispatch the transaction through the bitcoin coordinator.
    coordinator.dispatch(
        tx1.clone(),
        Some(speedup_data),
        tx_context.clone(),
        None,
        None,
    )?;

    Ok(tx1)
}

/// Test setup components that are commonly used across tests
pub struct TestSetup {
    pub network: Network,
    pub config_bitcoin_client: RpcConfig,
    pub key_manager: Rc<KeyManager>,
    pub storage: Rc<Storage>,
    pub bitcoin_client: Rc<BitcoinClient>,
    pub bitcoind: Bitcoind,
    pub public_key: PublicKey,
    pub funding_wallet: Address,
    pub regtest_wallet: Address,
}

/// Configuration for creating a test setup
pub struct TestSetupConfig {
    pub blocks_mined: u32,
    pub bitcoind_flags: Option<BitcoindFlags>,
}

impl Default for TestSetupConfig {
    fn default() -> Self {
        Self {
            blocks_mined: 102,
            bitcoind_flags: None,
        }
    }
}

/// Creates the basic test infrastructure (network, key manager, storage, bitcoin client config)
pub fn create_test_infrastructure(
    network: Network,
) -> Result<(RpcConfig, Rc<KeyManager>, Rc<Storage>, Rc<BitcoinClient>), anyhow::Error> {
    let path_key_manager = format!("test_output/test/key_manager/{}", generate_random_string());
    let key_manager_storage_config = StorageConfig::new(path_key_manager, None);
    let config_bitcoin_client = RpcConfig::new(
        network,
        "http://127.0.0.1:18443".to_string(),
        "foo".to_string(),
        "rpcpassword".to_string(),
        "test_wallet".to_string(),
    );
    let key_manager_config = KeyManagerConfig::new(network.to_string(), None, None);
    let key_manager = Rc::new(
        create_key_manager_from_config(&key_manager_config, &key_manager_storage_config)
            .map_err(|e| anyhow::anyhow!("Failed to create key manager: {:?}", e))?,
    );
    let path_storage = format!("test_output/test/storage/{}", generate_random_string());
    let storage_config = StorageConfig::new(path_storage, None);
    let storage = Rc::new(
        Storage::new(&storage_config)
            .map_err(|e| anyhow::anyhow!("Failed to create storage: {:?}", e))?,
    );
    let bitcoin_client = Rc::new(BitcoinClient::new_from_config(&config_bitcoin_client)?);

    Ok((config_bitcoin_client, key_manager, storage, bitcoin_client))
}

/// Creates and starts bitcoind with optional flags
pub fn create_and_start_bitcoind(
    config_bitcoin_client: &RpcConfig,
    flags: Option<BitcoindFlags>,
) -> Result<Bitcoind, anyhow::Error> {
    let bitcoind_config = BitcoindConfig::default();
    let bitcoind = Bitcoind::new(bitcoind_config, config_bitcoin_client.clone(), flags);

    info!("{} Starting bitcoind", style("Test").green());
    bitcoind.start().map_err(|e| {
        anyhow::anyhow!(
            "Failed to start bitcoind: {:?}. Make sure Docker is running.",
            e
        )
    })?;

    Ok(bitcoind)
}

/// Sets up wallet and mines initial blocks
pub fn setup_wallet_and_mine_blocks(
    key_manager: &Rc<KeyManager>,
    bitcoin_client: &Rc<BitcoinClient>,
    network: Network,
    blocks_mined: u32,
) -> Result<(PublicKey, Address, Address), anyhow::Error> {
    info!("{} Creating keypair in key manager", style("Test").green());
    let public_key = key_manager
        .derive_keypair(BitcoinKeyType::P2tr, 0)
        .map_err(|e| anyhow::anyhow!("Failed to derive keypair: {:?}", e))?;
    let compressed = CompressedPublicKey::try_from(public_key)
        .map_err(|e| anyhow::anyhow!("Failed to compress public key: {:?}", e))?;
    let funding_wallet = Address::p2wpkh(&compressed, network);
    let regtest_wallet = bitcoin_client
        .init_wallet("test_wallet")
        .map_err(|e| anyhow::anyhow!("Failed to init wallet: {:?}", e))?;

    info!(
        "{} Mine {} blocks to address {:?}",
        style("Test").green(),
        blocks_mined,
        regtest_wallet
    );

    bitcoin_client
        .mine_blocks_to_address(blocks_mined as u64, &regtest_wallet)
        .map_err(|e| anyhow::anyhow!("Failed to mine blocks: {:?}", e))?;

    Ok((public_key, funding_wallet, regtest_wallet))
}

/// Creates a complete test setup with all common components
pub fn create_test_setup(config: TestSetupConfig) -> Result<TestSetup, anyhow::Error> {
    let network = Network::Regtest;
    let (config_bitcoin_client, key_manager, storage, bitcoin_client) =
        create_test_infrastructure(network)?;

    let bitcoind = create_and_start_bitcoind(&config_bitcoin_client, config.bitcoind_flags)?;

    let (public_key, funding_wallet, regtest_wallet) =
        setup_wallet_and_mine_blocks(&key_manager, &bitcoin_client, network, config.blocks_mined)?;

    Ok(TestSetup {
        network,
        config_bitcoin_client,
        key_manager,
        storage,
        bitcoin_client,
        bitcoind,
        public_key,
        funding_wallet,
        regtest_wallet,
    })
}

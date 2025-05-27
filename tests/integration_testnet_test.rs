use bitcoin::{Amount, OutPoint, PublicKey, ScriptBuf, Transaction, TxOut, Txid};
use bitcoin_coordinator::storage::BitcoinCoordinatorStore;
use bitcoin_coordinator::{AckMonitorNews, MonitorNews, TypesToMonitor};
use bitvmx_wallet::config::WalletConfig;
use bitvmx_wallet::errors::WalletError;
use bitvmx_wallet::wallet::Wallet;
use key_manager::create_key_manager_from_config;
use key_manager::key_manager::KeyManager;
use key_manager::key_store::KeyStore;
use protocol_builder::builder::{Protocol, ProtocolBuilder};
use protocol_builder::scripts::{self, ProtocolScript, SignMode};
use protocol_builder::types::input::SighashType;
use protocol_builder::types::output::SpendMode;
use protocol_builder::types::{InputArgs, OutputType};
use std::rc::Rc;
use std::str::FromStr;
use storage_backend::storage::Storage;
use utils::{clear_db, generate_tx};
mod utils;
use anyhow::{Ok, Result};
use bitcoin::Network;
use bitcoin_coordinator::config::Config;
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitcoin_coordinator::types::FundingTransaction;
use bitvmx_bitcoin_rpc::bitcoin_client::BitcoinClient;
use bitvmx_transaction_monitor::monitor::Monitor;
use console::style;
use transaction_dispatcher::dispatcher::TransactionDispatcher;
use transaction_dispatcher::signer::Account;

// 1) We crete a wallet with funds
// 2) We create a transaction protocol fund with wallet
// 3) With the wallet we create a OutPoint for speed up transaction
// 4) Create the coordinator an monitor the transaction protocol
// 5) Create the coordinator and start it
#[test]
#[ignore]
fn integration_test() -> Result<(), anyhow::Error> {
    let config = Config::load()?;
    println!("Config: {:?}", config);

    let log_level = match config.log_level {
        Some(ref level) => level.parse().unwrap_or(tracing::Level::INFO),
        None => tracing::Level::INFO,
    };

    tracing_subscriber::fmt().with_max_level(log_level).init();

    println!(
        "\n{} I'm here to showcase the interaction between the different BitVMX modules.\n",
        style("Hi!").cyan()
    );

    clear_db(&config.storage.path);
    clear_db(&config.key_storage.path);

    let bitcoin_client = BitcoinClient::new_from_config(&config.rpc)?;
    let account = Account::new(config.rpc.network);
    let store = Rc::new(Storage::new(&config.storage)?);

    println!("Storage Created");

    let storage = Rc::new(Storage::new(&config.key_storage)?);
    let keystore = KeyStore::new(storage.clone());
    let funding_key_manager = Rc::new(create_key_manager_from_config(
        &config.key_manager,
        keystore,
        store.clone(),
    )?);

    let dispatcher = TransactionDispatcher::new(bitcoin_client, funding_key_manager.clone());

    let monitor = Monitor::new_with_paths(
        &config.rpc,
        store.clone(),
        config.monitor.checkpoint_height,
        config.monitor.confirmation_threshold,
    )?;

    // This is the storage for the protocol, for this porpouse will be a different storage
    let store = BitcoinCoordinatorStore::new(store.clone())?;

    let wallet = create_wallet(&config)?;
    let pubkey = wallet.create_wallet("fund_speedup")?;
    let (_, pk) = wallet.export_wallet("fund_speedup")?;
    funding_key_manager.import_secret_key(&pk.to_string(), Network::Testnet)?;

    // Create funds for tx to speed up
    let funding_txid = wallet.fund_address(
        "funds",
        "1",
        pubkey,
        &vec![500, 2000],
        500,
        false,
        true,
        None,
    )?;

    let tx = create_tx_for_speedup(
        OutPoint {
            txid: funding_txid,
            vout: 0,
        },
        pubkey,
        funding_key_manager,
        pubkey,
    )?;

    let context = "My Transaction to speed up".to_string();
    let tx_to_monitor = TypesToMonitor::Transactions(vec![tx.compute_txid()], context.clone());

    let coordinator = BitcoinCoordinator::new(monitor, store, dispatcher, account.clone());

    coordinator.monitor(tx_to_monitor)?;

    let tx_out = TxOut {
        value: Amount::from_sat(500),
        script_pubkey: ScriptBuf::new_p2wpkh(&pubkey.wpubkey_hash().unwrap()),
    };

    let funding = FundingTransaction::new(tx.compute_txid(), 1, tx_out);
    coordinator.fund_for_speedup(
        vec![tx.compute_txid()],
        funding,
        "Funds for speed up tx".to_string(),
    )?;

    coordinator.dispatch(tx, context.clone(), None)?;

    loop {
        coordinator.tick()?;

        let news = coordinator.get_news()?;

        if news.monitor_news.len() > 0 {
            println!("News: {:?}", news);
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    Ok(())
}

fn create_wallet(config: &Config) -> Result<Wallet, anyhow::Error> {
    let wallet_config = WalletConfig::new(
        config.rpc.clone(),
        config.key_manager.clone(),
        config.key_storage.clone(),
        config.storage.clone(),
    )?;

    let wallet = Wallet::new(wallet_config, true)?;

    Ok(wallet)
}

fn create_tx_for_speedup(
    tx_funding: OutPoint,
    tx_funding_pubkey: PublicKey,
    key_manager: Rc<KeyManager>,
    speedup_pubkey: PublicKey,
) -> Result<Transaction, anyhow::Error> {
    let tx_amount = 300; // dust
    let speedup_amount = 245000;

    let external_output = OutputType::segwit_key(tx_amount, &tx_funding_pubkey)?;

    let mut protocol = Protocol::new("tx_to_speed_up");

    protocol.add_external_connection(
        tx_funding.txid,
        tx_funding.vout,
        external_output,
        "transfer",
        &SpendMode::Segwit,
        &SighashType::ecdsa_all(),
    )?;

    let output_type = OutputType::segwit_key(speedup_amount, &speedup_pubkey)?;
    protocol.add_transaction_output("transfer", &output_type)?;

    protocol.build_and_sign(&key_manager, "id")?;

    let signature = protocol.input_ecdsa_signature("transfer", 0)?.unwrap();

    let mut spending_args = InputArgs::new_segwit_args();
    spending_args.push_ecdsa_signature(signature)?;

    let result = protocol.transaction_to_send("transfer", &[spending_args])?;

    Ok(result)
}

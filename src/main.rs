use anyhow::{Context, Ok, Result};
use bitcoin::{Amount, ScriptBuf, Transaction, TxOut, Txid};
use bitcoin_coordinator::config::Config;
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitcoin_coordinator::storage::BitcoinCoordinatorStore;
use bitcoin_coordinator::tx_builder_helper::{
    create_key_manager, create_txs, generate_tx, send_transaction,
};
use bitcoin_coordinator::types::FundingTransaction;
use bitvmx_bitcoin_rpc::bitcoin_client::BitcoinClient;
use bitvmx_transaction_monitor::monitor::Monitor;
use bitvmx_transaction_monitor::types::{ExtraData, TransactionMonitor};
use console::style;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::mpsc::{channel, Receiver};
use storage_backend::storage::Storage;
use tracing::info;
use transaction_dispatcher::{dispatcher::TransactionDispatcher, signer::Account};
use uuid::Uuid;

fn main() -> Result<()> {
    let config = Config::load()?;

    let log_level = match config.log_level {
        Some(ref level) => level.parse().unwrap_or(tracing::Level::INFO),
        None => tracing::Level::INFO,
    };

    tracing_subscriber::fmt().with_max_level(log_level).init();

    info!(
        "\n{} I'm here to showcase the interaction between the different BitVMX modules.\n",
        style("Hi!").cyan()
    );

    let config = Config::load()?;
    let client = BitcoinClient::new_from_config(&config.rpc)?;
    let account = Account::new(config.rpc.network);
    let key_manager = create_key_manager(&config)?;
    let dispatcher = TransactionDispatcher::new(client, Rc::new(key_manager));
    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(
        &config.database.path,
    ))?);
    let monitor = Monitor::new_with_paths(
        &config.rpc,
        storage,
        config.monitor.checkpoint_height,
        config.monitor.confirmation_threshold,
    )?;

    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(
        &config.database.path,
    ))?);
    // This is the storage for the protocol, for this porpouse will be a different storage
    let store = BitcoinCoordinatorStore::new(storage)?;

    // Step 1: Create an instance with 2 transactions for different operators
    println!(
        "\n{} Step 1: Creating an instance with 2 transactions for different operators...\n",
        style("Step 1").blue()
    );

    let group_id = Uuid::from_u128(1);

    //hardcoded transaction.
    let funding_tx_id =
        Txid::from_str("3a3f8d147abf0b9b9d25b07de7a16a4db96bda3e474ceab4c4f9e8e107d5b02f").unwrap();

    let funding_tx = FundingTransaction {
        tx_id: funding_tx_id,
        utxo_index: 0,
        utxo_output: TxOut {
            value: Amount::default(),
            script_pubkey: ScriptBuf::default(),
        },
    };

    let tx_1: Transaction = generate_tx(
        &account,
        &config.rpc,
        config.rpc.network,
        &config.dispatcher,
    )?;
    let tx_2: Transaction = generate_tx(
        &account,
        &config.rpc,
        config.rpc.network,
        &config.dispatcher,
    )?;

    let txs = vec![tx_1, tx_2];

    println!("{} Create Group tx: 1", style("→").cyan());

    println!(
        "{} Create transaction: {:#?} for operator: 1",
        style("→").cyan(),
        style(tx_1.compute_txid()).red()
    );

    println!(
        "{} Create transaction: {:#?}  for operator: 2",
        style("→").cyan(),
        style(tx_2.compute_txid()).blue(),
    );

    let extra_data = ExtraData::GroupId(group_id);

    let txs_to_monitor = TransactionMonitor::Transactions(
        txs.iter().map(|tx| tx.compute_txid()).collect(),
        extra_data,
    );

    // Step 2: Send the first transaction for operator one
    println!(
        "\n{} Step 3: Sending tx_id: {}.\n",
        style("Step 3").cyan(),
        style(txs[0].compute_txid()).red(),
    );

    send_transaction(txs[0].clone(), &Config::load()?)?;

    let mut tx_to_answer: (Uuid, Txid, Option<Transaction>) =
        (group_id, txs[0].compute_txid(), Some(txs[1].clone()));

    // Step 2: Make the Bitcoin Coordinator monitor the instance
    println!(
        "\n{} Step 2: Bitcoin Coordinator monitor the instance...\n",
        style("Bitcoin Coordinator").cyan()
    );

    let coordinator = BitcoinCoordinator::new(monitor, store, dispatcher, account.clone());

    coordinator
        .monitor(txs_to_monitor)
        .context("Error monitoring instance")?;

    let rx = handle_contro_c();

    loop {
        if rx.try_recv().is_ok() {
            info!("Stopping Bitvmx Runner");
            break;
        }

        info!("New tick for for Bitcoin Coordinator");

        coordinator
            .tick()
            .context("Failed tick Bitcoin Coordinator")?;

        let news_list = coordinator.get_news()?;

        for tx_news in news_list.txs {
            info!(
                "{} Transaction ID {} for Instance ID {} CONFIRMED!!! \n",
                style("Bitcoin Coordinator").green(),
                style(tx_news.tx_id).blue(),
                style(tx_news.instance_id).green()
            );

            let tx_id = tx_new.tx.compute_txid();
            let tx: Option<Transaction> = tx_to_answer.2;
            tx_to_answer.2 = None;

            if tx.is_none() {
                info!(
                    "{} Transaction ID {} for Instance ID {} NO ANSWER FOUND \n",
                    style("Info").green(),
                    style(tx_id).blue(),
                    style(instance_id).green()
                );
                return Ok(());
            }

            let tx: Transaction = tx.unwrap();
            coordinator.dispatch(instance_id, &tx)?;

            coordinator.ack_news(ProcessedNews {
                txs: vec![(instance_id, vec![tx.compute_txid()])],
                single_txs: vec![],
                funds_requests: vec![],
            })?;
        }

        wait();
    }

    Ok(())
}

fn handle_contro_c() -> Receiver<()> {
    let (tx, rx) = channel();
    ctrlc::set_handler(move || tx.send(()).expect("Could not send signal on channel."))
        .expect("Error setting Ctrl-C handler");

    rx
}

fn wait() {
    std::thread::sleep(std::time::Duration::from_secs(5));
}

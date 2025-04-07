use anyhow::{Context, Ok, Result};
use bitcoin::Transaction;
use bitcoin_coordinator::config::Config;
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitcoin_coordinator::storage::BitcoinCoordinatorStore;
use bitcoin_coordinator::tx_builder_helper::{
    create_instance, create_key_manager, send_transaction,
};
use bitcoin_coordinator::types::{Id, ProcessedNews};
use bitvmx_bitcoin_rpc::bitcoin_client::BitcoinClient;
use bitvmx_transaction_monitor::monitor::Monitor;
use console::style;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver};
use storage_backend::storage::Storage;
use tracing::info;
use transaction_dispatcher::{dispatcher::TransactionDispatcher, signer::Account};

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

    let instance = create_instance(
        &account,
        &config.rpc,
        config.rpc.network,
        &config.dispatcher,
    )?;

    // Step 2: Send the first transaction for operator one
    println!(
        "\n{} Step 3: Sending tx_id: {}.\n",
        style("Step 3").cyan(),
        style(instance.txs[0].tx.compute_txid()).red(),
    );
    send_transaction(instance.txs[0].tx.clone(), &Config::load()?)?;

    let mut tx_to_answer: (Id, bitcoin::Txid, Option<Transaction>) = (
        instance.id,
        instance.txs[0].tx.compute_txid(),
        Some(instance.txs[1].tx.clone()),
    );

    // Step 2: Make the Bitcoin Coordinator monitor the instance
    println!(
        "\n{} Step 2: Bitcoin Coordinator monitor the instance...\n",
        style("Bitcoin Coordinator").cyan()
    );

    println!("{:?}", instance.map_partial_info());
    let coordinator = BitcoinCoordinator::new(monitor, store, dispatcher, account.clone());

    coordinator
        .monitor(&instance.map_partial_info())
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

        let news = coordinator.get_news()?;

        for (instance_id, tx_news) in news.instance_txs {
            for tx_new in tx_news {
                info!(
                    "{} Transaction ID {} for Instance ID {} CONFIRMED!!! \n",
                    style("Bitcoin Coordinator").green(),
                    style(tx_new.tx.compute_txid()).blue(),
                    style(instance_id).green()
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

                coordinator.acknowledge_news(ProcessedNews {
                    txs: vec![(instance_id, vec![tx.compute_txid()])],
                    single_txs: vec![],
                    funds_requests: vec![],
                })?;
            }
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

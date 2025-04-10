use anyhow::{Context, Ok, Result};
use bitcoin::{Network, Transaction, Txid};
use bitcoin_coordinator::config::Config;
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitcoin_coordinator::storage::BitcoinCoordinatorStore;
use bitcoin_coordinator::tx_builder_helper::{create_key_manager, generate_tx};
use bitcoin_coordinator::types::AckNews;
use bitcoind::bitcoind::Bitcoind;
use bitvmx_bitcoin_rpc::bitcoin_client::{BitcoinClient, BitcoinClientApi};
use bitvmx_transaction_monitor::monitor::Monitor;
use bitvmx_transaction_monitor::types::{
    AckTransactionNews, ExtraData, TransactionMonitor, TransactionNews,
};
use console::style;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver};
use storage_backend::storage::Storage;
use tracing::info;
use transaction_dispatcher::dispatcher::TransactionDispatcherApi;
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

    let bitcoind = Bitcoind::new(
        "bitcoin-regtest",
        "ruimarinho/bitcoin-core",
        config.rpc.clone(),
    );
    info!("Starting bitcoind");
    bitcoind.start()?;

    let bitcoin_client = BitcoinClient::new_from_config(&config.rpc)?;
    let wallet = bitcoin_client
        .init_wallet(Network::Regtest, "test_wallet")
        .unwrap();

    info!("Mine 101 blocks to address {:?}", wallet);
    bitcoin_client.mine_blocks_to_address(202, &wallet).unwrap();

    let account = Account::new(config.rpc.network);
    let key_manager = create_key_manager(&config)?;
    let dispatcher = TransactionDispatcher::new(bitcoin_client, Rc::new(key_manager));
    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(
        &config.database.path,
    ))?);
    let monitor = Monitor::new_with_paths(
        &config.rpc,
        storage.clone(),
        config.monitor.checkpoint_height,
        config.monitor.confirmation_threshold,
    )?;

    // This is the storage for the protocol, for this porpouse will be a different storage
    let store = BitcoinCoordinatorStore::new(storage.clone())?;

    // Step 1: Create an instance with 2 transactions for different operators
    println!(
        "\n{} Step 1: Creating an instance with 2 transactions for different operators...\n",
        style("Step 1").blue()
    );

    let group_id = Uuid::from_u128(1);

    let tx_1: Transaction = generate_tx(
        &account,
        &config.rpc,
        config.rpc.network,
        &config.dispatcher,
    )?;
    let tx_1_id = tx_1.compute_txid();
    let tx_2: Transaction = generate_tx(
        &account,
        &config.rpc,
        config.rpc.network,
        &config.dispatcher,
    )?;

    let tx_2_id = tx_2.compute_txid();

    let txs = [tx_1, tx_2];

    println!("{} Create Group tx: 1", style("→").cyan());

    println!(
        "{} Create transaction: {:#?} for operator: 1",
        style("→").cyan(),
        style(tx_1_id).red()
    );

    println!(
        "{} Create transaction: {:#?}  for operator: 2",
        style("→").cyan(),
        style(tx_2_id).blue(),
    );

    let extra_data = ExtraData::Context("MY context".to_string());

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

    dispatcher.send(txs[0].clone()).unwrap();

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

    let bitcoin_client = BitcoinClient::new_from_config(&config.rpc)?;

    let rx = handle_contro_c();

    for i in 0..1000 {
        if i % 20 == 0 {
            bitcoin_client.mine_blocks_to_address(1, &wallet).unwrap();
        }

        if rx.try_recv().is_ok() {
            info!("Stopping Bitvmx Runner");
            break;
        }

        info!("New tick for for Bitcoin Coordinator");

        coordinator
            .tick()
            .context("Failed tick Bitcoin Coordinator")?;

        let news_list = coordinator.get_news()?;

        for news in news_list.txs {
            match news {
                TransactionNews::Transaction(tx_id, _, extra_data) => {
                    println!("Extra data: {:?}", extra_data);
                    let context = match extra_data {
                        ExtraData::Context(context) => context,
                        _ => "".to_string(),
                    };

                    info!(
                        "{} Transaction ID {} for Instance ID {} CONFIRMED!!! \n",
                        style("Bitcoin Coordinator").green(),
                        style(tx_id).blue(),
                        style(context.clone()).green()
                    );

                    let tx: Option<Transaction> = tx_to_answer.2;

                    tx_to_answer.2 = None;

                    if tx.is_none() {
                        info!(
                            "{} Transaction ID {} for Instance ID {} NO ANSWER FOUND \n",
                            style("Info").green(),
                            style(tx_1_id).blue(),
                            style(context).green()
                        );
                        return Ok(());
                    }

                    let tx: Transaction = tx.unwrap();
                    coordinator.dispatch(tx, "my_context".to_string())?;

                    let ack_news = AckNews::Transaction(AckTransactionNews::Transaction(tx_1_id));
                    coordinator.ack_news(ack_news)?;
                }
                TransactionNews::RskPeginTransaction(tx_id, _) => {
                    info!(
                        "{} RSK Pegin transaction with ID: {} detected",
                        style("Bitcoin Coordinator").green(),
                        style(tx_id).yellow(),
                    );
                }
                TransactionNews::SpendingUTXOTransaction(tx_id, utxo_txid, _, _) => {
                    info!(
                        "{} Insufficient funds for transaction: {} - UTXO {} was spent",
                        style("Bitcoin Coordinator").red(),
                        style(tx_id).red(),
                        style(utxo_txid).yellow()
                    );
                }
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
    std::thread::sleep(std::time::Duration::from_millis(50));
}

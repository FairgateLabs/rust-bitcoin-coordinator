use bitcoin::{Transaction, Txid};
use bitcoin_coordinator::storage::BitcoinCoordinatorStore;
use bitcoin_coordinator::{AckMonitorNews, MonitorNews, TypesToMonitor};
use key_manager::create_key_manager_from_config;
use key_manager::key_store::KeyStore;
use std::rc::Rc;
use storage_backend::storage::Storage;
use utils::{clear_db, generate_tx};
mod utils;
use anyhow::{Ok, Result};
use bitcoin::Network;
use bitcoin_coordinator::config::Config;
use bitcoin_coordinator::coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi};
use bitcoin_coordinator::types::AckNews;
use bitcoind::bitcoind::Bitcoind;
use bitvmx_bitcoin_rpc::bitcoin_client::{BitcoinClient, BitcoinClientApi};
use bitvmx_transaction_monitor::monitor::Monitor;
use console::style;
use std::sync::mpsc::{channel, Receiver};
use uuid::Uuid;

#[test]
#[ignore = "This test is not working"]
fn integration_test() -> Result<(), anyhow::Error> {
    let config = Config::load()?;

    let log_level = match config.log_level {
        Some(ref level) => level.parse().unwrap_or(tracing::Level::INFO),
        None => tracing::Level::INFO,
    };

    tracing_subscriber::fmt().with_max_level(log_level).init();

    println!(
        "\n{} I'm here to showcase the interaction between the different BitVMX modules.\n",
        style("Hi!").cyan()
    );

    let config = Config::load()?;

    clear_db(&config.storage.path);
    clear_db(&config.key_storage.path);

    let bitcoind = Bitcoind::new(
        "bitcoin-regtest",
        "ruimarinho/bitcoin-core",
        config.rpc.clone(),
    );
    println!("Starting bitcoind");
    bitcoind.start()?;

    let bitcoin_client = BitcoinClient::new_from_config(&config.rpc)?;
    let wallet = bitcoin_client
        .init_wallet(Network::Regtest, "test_wallet")
        .unwrap();

    println!("Mine 101 blocks to address {:?}", wallet);
    bitcoin_client.mine_blocks_to_address(202, &wallet).unwrap();

    let store = Rc::new(Storage::new(&config.storage)?);

    println!("Storage Created");

    let storage = Rc::new(Storage::new(&config.key_storage)?);
    let keystore = KeyStore::new(storage.clone());
    let key_manager = create_key_manager_from_config(&config.key_manager, keystore, store.clone())?;

    let monitor = Monitor::new_with_paths(
        &config.rpc,
        store.clone(),
        config.monitor.checkpoint_height,
        config.monitor.confirmation_threshold,
    )?;

    // This is the storage for the protocol, for this porpouse will be a different storage
    let store = BitcoinCoordinatorStore::new(store.clone())?;

    // Step 1: Create an  array with 2 transactions for different operators
    println!(
        "\n{} Step 1: Creating an array with 2 transactions for different operators...\n",
        style("Step 1").blue()
    );

    // let group_id = Uuid::from_u128(1);

    // let tx_1: Transaction = generate_tx(&config.rpc, Network::Regtest)?;
    // let tx_1_id = tx_1.compute_txid();
    // let tx_2: Transaction = generate_tx(&config.rpc, Network::Regtest)?;

    // let tx_2_id = tx_2.compute_txid();

    // let txs = [tx_1, tx_2];

    // println!("{} Create Group tx: 1", style("→").cyan());

    // println!(
    //     "{} Create transaction: {:#?} for operator: 1",
    //     style("→").cyan(),
    //     style(tx_1_id).red()
    // );

    // println!(
    //     "{} Create transaction: {:#?}  for operator: 2",
    //     style("→").cyan(),
    //     style(tx_2_id).blue(),
    // );

    // let context_data = "MY context".to_string();

    // let txs_to_monitor = TypesToMonitor::Transactions(
    //     txs.iter().map(|tx| tx.compute_txid()).collect(),
    //     context_data,
    // );

    // // Step 2: Send the first transaction for operator one
    // println!(
    //     "\n{} Step 3: Sending tx_id: {}.\n",
    //     style("Step 3").cyan(),
    //     style(txs[0].compute_txid()).red(),
    // );

    // // dispatcher.send(txs[0].clone()).unwrap();

    // let mut tx_to_answer: (Uuid, Txid, Option<Transaction>) =
    //     (group_id, txs[0].compute_txid(), Some(txs[1].clone()));

    // // Step 2: Make the Bitcoin Coordinator monitor the txs
    // println!(
    //     "\n{} Step 2: Bitcoin Coordinator monitor the txs...\n",
    //     style("Bitcoin Coordinator").cyan()
    // );

    // let coordinator = BitcoinCoordinator::new(
    //     monitor,
    //     store,
    //     Rc::new(key_manager),
    //     bitcoin_client,
    //     Network::Regtest,
    // );

    // coordinator.monitor(txs_to_monitor)?;

    // coordinator.monitor(TypesToMonitor::NewBlock)?;

    // let bitcoin_client = BitcoinClient::new_from_config(&config.rpc)?;

    // let rx = handle_contro_c();

    // for i in 0..1000 {
    //     if i % 20 == 0 {
    //         println!("Mine new block");
    //         bitcoin_client.mine_blocks_to_address(1, &wallet).unwrap();
    //     }

    //     if rx.try_recv().is_ok() {
    //         println!("Stopping Bitvmx Runner");
    //         break;
    //     }

    //     println!("New tick for for Bitcoin Coordinator");

    //     coordinator.tick()?;

    //     let news_list = coordinator.get_news()?;

    //     for news in news_list.monitor_news {
    //         match news {
    //             MonitorNews::Transaction(tx_id, _, data) => {
    //                 println!("Context Data: {:?}", data);

    //                 println!(
    //                     "{} Transaction ID {} for group ID {} CONFIRMED!!! \n",
    //                     style("Bitcoin Coordinator").green(),
    //                     style(tx_id).blue(),
    //                     style(data.clone()).green()
    //                 );

    //                 let tx: Option<Transaction> = tx_to_answer.2;

    //                 tx_to_answer.2 = None;

    //                 if tx.is_none() {
    //                     println!(
    //                         "{} Transaction ID {} for group ID {} NO ANSWER FOUND \n",
    //                         style("Info").green(),
    //                         style(tx_1_id).blue(),
    //                         style(data).green()
    //                     );

    //                     continue;
    //                 }

    //                 let tx: Transaction = tx.unwrap();
    //                 coordinator.dispatch(tx, None, "my_context".to_string(), None)?;

    //                 let ack_news = AckNews::Monitor(AckMonitorNews::Transaction(tx_1_id));
    //                 coordinator.ack_news(ack_news)?;
    //             }
    //             MonitorNews::RskPeginTransaction(tx_id, _) => {
    //                 println!(
    //                     "{} RSK Pegin transaction with ID: {} detected",
    //                     style("Bitcoin Coordinator").green(),
    //                     style(tx_id).yellow(),
    //                 );
    //             }
    //             MonitorNews::SpendingUTXOTransaction(tx_id, utxo_txid, _, _) => {
    //                 println!(
    //                     "{} Insufficient funds for transaction: {} - UTXO {} was spent",
    //                     style("Bitcoin Coordinator").red(),
    //                     style(tx_id).red(),
    //                     style(utxo_txid).yellow()
    //                 );
    //             }
    //             MonitorNews::NewBlock(block_height, block_hash) => {
    //                 println!(
    //                     "{} New block detected: {} - {}",
    //                     style("Bitcoin Coordinator").green(),
    //                     style(block_height).yellow(),
    //                     style(block_hash).yellow()
    //                 );
    //             }
    //         }
    //     }

    //     wait();
    // }

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

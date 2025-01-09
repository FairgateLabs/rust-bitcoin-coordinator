use anyhow::{Context, Ok, Result};
use bitcoin::{Network, Transaction};
use bitcoincore_rpc::{Auth, Client};
use bitvmx_orchestrator::orchestrator::OrchestratorApi;
use bitvmx_orchestrator::storage::OrchestratorStore;
use bitvmx_orchestrator::tx_builder_helper::{
    create_instance, create_key_manager, send_transaction,
};
use bitvmx_orchestrator::types::ProcessedNews;
use bitvmx_orchestrator::{config::Config, orchestrator::Orchestrator};
use bitvmx_transaction_monitor::monitor::Monitor;
use console::style;
use log::info;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::mpsc::{channel, Receiver};
use storage_backend::storage::Storage;
use transaction_dispatcher::{dispatcher::TransactionDispatcher, signer::Account};

fn main() -> Result<()> {
    env_logger::init();

    info!(
        "\n{} I'm here to showcase the interaction between the different BitVMX modules.\n",
        style("Hi!").cyan()
    );

    let config = Config::load()?;
    let network = Network::from_str(config.rpc.network.as_str())?;
    let client = Client::new(
        config.rpc.url.as_str(),
        Auth::UserPass(
            config.rpc.username.as_str().to_string(),
            config.rpc.password.as_str().to_string(),
        ),
    )?;

    // let list = client.list_wallets()?;
    // info!("{} {:?}", style("Wallet list").green(), list);

    let account = Account::new(network);
    let key_manager = create_key_manager(&config.key_manager, network)?;
    let dispatcher = TransactionDispatcher::new(client, key_manager);
    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(
        &config.database.path,
    ))?);
    let monitor = Monitor::new_with_paths(
        &config.rpc.url,
        storage,
        config.monitor.checkpoint_height,
        config.monitor.confirmation_threshold,
    )?;

    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(
        &config.database.path,
    ))?);
    // This is the storage for the protocol, for this porpouse will be a different storage
    let store = OrchestratorStore::new(storage)?;

    // Step 1: Create an instance with 2 transactions for different operators
    println!(
        "\n{} Step 1: Creating an instance with 2 transactions for different operators...\n",
        style("Step 1").blue()
    );

    let instance = create_instance(&account, &config.rpc, network, &config.dispatcher)?;

    // Step 2: Send the first transaction for operator one
    println!(
        "\n{} Step 3: Sending tx_id: {}.\n",
        style("Step 3").cyan(),
        style(instance.txs[0].tx.compute_txid()).red(),
    );
    send_transaction(instance.txs[0].tx.clone(), &Config::load()?, network)?;

    let mut tx_to_answer: (u32, bitcoin::Txid, Option<Transaction>) = (
        instance.instance_id,
        instance.txs[0].tx.compute_txid(),
        Some(instance.txs[1].tx.clone()),
    );

    // Step 2: Make the orchestrator monitor the instance
    println!(
        "\n{} Step 2: Orchestrator monitor the instance...\n",
        style("Orchestrator").cyan()
    );

    println!("{:?}", instance.map_partial_info());
    let mut orchestrator = Orchestrator::new(monitor, store, dispatcher, account.clone());

    orchestrator
        .monitor_instance(&instance.map_partial_info())
        .context("Error monitoring instance")?;

    let rx = handle_contro_c();

    loop {
        if rx.try_recv().is_ok() {
            info!("Stopping Bitvmx Runner");
            break;
        }

        info!("New tick for for Orchestrator");

        orchestrator.tick().context("Failed tick orchestrator")?;

        let news = orchestrator.get_news()?;

        for (instance_id, tx_news) in news.txs_by_id {
            for tx_new in tx_news {
                info!(
                    "{} Transaction ID {} for Instance ID {} CONFIRMED!!! \n",
                    style("Orchestrator").green(),
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
                orchestrator.send_tx_instance(instance_id, &tx)?;

                orchestrator.acknowledge_news(ProcessedNews {
                    txs_by_id: vec![(instance_id, vec![tx.compute_txid()])],
                    txs_by_address: vec![],
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

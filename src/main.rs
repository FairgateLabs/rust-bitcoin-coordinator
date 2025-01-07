use anyhow::{Context, Ok, Result};
use bitcoin::{Network, Transaction};
use bitcoincore_rpc::{Auth, Client};
use bitvmx_transaction_monitor::monitor::Monitor;
use bitvmx_unstable::orchestrator::OrchestratorApi;
use bitvmx_unstable::storage::{OrchestratorStore, StepHandlerApi};
use bitvmx_unstable::tx_builder_helper::{create_instance, create_key_manager, send_transaction};
use bitvmx_unstable::types::TransactionState;
use bitvmx_unstable::{config::Config, orchestrator::Orchestrator};
use console::style;
use log::info;
use std::str::FromStr;
use std::sync::mpsc::{channel, Receiver};
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
    let monitor = Monitor::new_with_paths(
        &config.rpc.url,
        &config.database.path,
        config.monitor.checkpoint_height,
        config.monitor.confirmation_threshold,
    )?;

    let store = OrchestratorStore::new_with_path(&config.database.path)?;
    let mut orchestrator = Orchestrator::new(monitor, &store, dispatcher, account.clone());

    // Step 1: Create an instance with 2 transactions for different operators
    println!(
        "\n{} Step 1: Creating an instance with 2 transactions for different operators...\n",
        style("Step 1").blue()
    );

    let instance = create_instance(&account, &config.rpc, network, &config.dispatcher)?;

    // Step 2: Make the orchestrator monitor the instance
    println!(
        "\n{} Step 2: Orchestrator monitor the instance...\n",
        style("Orchestrator").cyan()
    );

    println!("{:?}", instance.map_partial_info());

    orchestrator
        .monitor_instance(&instance.map_partial_info())
        .context("Error monitoring instance")?;

    // Step 3: Send the first transaction for operator one
    println!(
        "\n{} Step 3: Sending tx_id: {}.\n",
        style("Step 3").cyan(),
        style(instance.txs[0].tx.compute_txid()).red(),
    );
    send_transaction(instance.txs[0].tx.clone(), &Config::load()?, network)?;

    store.set_tx_to_answer(
        instance.instance_id,
        instance.txs[0].tx.compute_txid(),
        instance.txs[1].tx.clone(),
    )?;

    let rx = handle_contro_c();

    loop {
        if rx.try_recv().is_ok() {
            info!("Stopping Bitvmx Runner");
            break;
        }

        info!("New tick for for Step Handler");

        orchestrator.tick().context("Failed tick orchestrator")?;

        let confirmed_txs = store.get_txs_info(TransactionState::Finalized)?;

        for (instance_id, txs) in confirmed_txs {
            for tx in txs {
                info!(
                    "{} Transaction ID {} for Instance ID {} CONFIRMED!!! \n",
                    style("StepHandler").green(),
                    style(tx.tx_id).blue(),
                    style(instance_id).green()
                );

                let tx_id = tx.tx_id;
                let tx: Option<Transaction> = store.get_tx_to_answer(instance_id, tx_id)?;

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

                store.update_instance_tx_status(
                    instance_id,
                    &tx.compute_txid(),
                    TransactionState::Acknowledged,
                )?;
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

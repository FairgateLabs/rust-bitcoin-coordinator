use anyhow::{Context, Ok, Result};
use bitcoin::Network;
use bitcoincore_rpc::{Auth, Client};
use bitvmx_unstable::storage::{BitvmxStore, StepHandlerApi};
use bitvmx_unstable::tx_builder_helper::{create_instance, create_key_manager, send_transaction};
use bitvmx_unstable::{
    config::Config,
    orchestrator::{Orchestrator, OrchestratorApi},
    step_handler::{StepHandler, StepHandlerTrait},
};
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
    let orchestrator = Orchestrator::new(
        &config.rpc.url,
        &config.database.path,
        config.monitor.checkpoint_height,
        dispatcher,
        account.clone(),
    )
    .context("Failed to create Orchestrator instance")?;

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
        "\n{} Step 3: Sending tx_id: {} for operator {}.\n",
        style("Step 3").cyan(),
        style(instance.txs[0].tx.compute_txid()).red(),
        style(instance.txs[0].owner_operator_id).green()
    );
    send_transaction(instance.txs[0].tx.clone(), &Config::load()?, network)?;

    //Step 4. Given that stepHandler should know what to do in each step we are gonna save that step for intstance is the following
    let storage = BitvmxStore::new_with_path((config.database.path + "/step_handler").as_str())?;
    storage.set_tx_to_answer(
        instance.instance_id,
        instance.txs[0].tx.compute_txid(),
        instance.txs[1].tx.clone(),
    )?;

    let mut step_handler = StepHandler::new(orchestrator, &storage)?;

    let rx = handle_contro_c();

    loop {
        if rx.try_recv().is_ok() {
            info!("Stopping Bitvmx Runner");
            break;
        }

        info!("New tick for for Step Handler");

        step_handler.tick()?;

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

//Create transaction: 438f8dd5549a765cf47038ea0dede37ef1905c36437d7c2c31a6a3f5f0fbcb3f for operator: 1
//Create transaction: 0baf114c4066ed836c62c470ec7d387bfdc012e05204068324b2db8ed255fa3c  for operator: 2

use anyhow::{Context, Ok, Result};
use bitcoin::Network;
use bitvmx_unstable::{
    config::Config,
    orchestrator::{Orchestrator, OrchestratorApi},
};
use std::str::FromStr;
use std::sync::mpsc::channel;
use tracing::{info, Level};

fn main() -> Result<()> {
    let (tx, rx) = channel();

    ctrlc::set_handler(move || tx.send(()).expect("Could not send signal on channel."))
        .expect("Error setting Ctrl-C handler");

    tracing_subscriber::fmt()
        .without_time()
        // .with_target(false)
        .with_max_level(Level::ERROR)
        .init();

    let config = Config::load()?;
    let network = Network::from_str(config.rpc.network.as_str())?;
    let node_rpc_url = config.rpc.url.clone();
    let db_file_path = config.database.path;
    let checkpoint_height = config.monitor.checkpoint_height;
    let username = config.rpc.username.clone();
    let password = config.rpc.password.clone();

    let mut orchestrator = Orchestrator::new(
        &node_rpc_url,
        &db_file_path,
        checkpoint_height,
        &username,
        &password,
        network,
    )
    .context("Failed to create Orchestrator instance")?;

    loop {
        if rx.try_recv().is_ok() {
            info!("Stop Bitvmx");
            break;
        }

        if orchestrator.is_ready()? {
            // Since the orchestrator is ready, indicating it's caught up with the blockchain, we can afford to wait for a minute
            //TODO: this may change for sure.
            std::thread::sleep(std::time::Duration::from_secs(60));
        }

        // If the orchestrator is not ready, it may require multiple ticks to become ready. No need to wait.
        orchestrator.tick().context("Failed tick orchestrator")?;
    }

    Ok(())
}

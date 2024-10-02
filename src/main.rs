use std::{str::FromStr, thread::sleep};

use anyhow::{Context, Ok, Result};

use bitcoin::Network;
use bitvmx_unstable::{
    config::Config,
    orchestrator::{Orchestrator, OrchestratorApi},
};
use tracing::Level;

fn main() -> Result<()> {
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
        orchestrator.tick().context("Failed tick orchestrator")?;

        sleep(std::time::Duration::from_secs(1000));
    }

    Ok(())
}

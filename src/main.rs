use std::str::FromStr;

use anyhow::{Context, Result};

use bitcoin::Network;
use bitvmx_unstable::{
    config::Config,
    orchestrator::{Orchestrator, OrchestratorApi},
};
use tracing::Level;

fn main() -> Result<()> {
    // TODO : should we keep this?
    tracing_subscriber::fmt()
        .without_time()
        // .with_target(false)
        .with_max_level(Level::ERROR)
        .init();

    let config = Config::load()?;
    let network = Network::from_str(config.rpc.network.as_str())?;
    println!("Network: {:?}", network);
    let node_rpc_url = config.rpc.url.clone();
    println!("Node RPC URL: {:?}", node_rpc_url);
    let db_file_path = config.database.path;
    println!("Database File Path: {:?}", db_file_path);
    let checkpoint_height = config.monitor.checkpoint_height;
    println!("Checkpoint Height: {:?}", checkpoint_height);
    let username = config.rpc.username.clone();
    println!("Username: {:?}", username);
    let password = config.rpc.password.clone();
    println!("Password: {:?}", password);

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
        // TODO: We need to figure out how often we should call this function
        orchestrator.tick().context("Failed to tick orchestrator")?;
    }
}

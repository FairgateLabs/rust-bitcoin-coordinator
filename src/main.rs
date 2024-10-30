use anyhow::{Context, Ok, Result};
use bitcoin::Network;
use bitcoin::{
    absolute, key::Secp256k1, secp256k1::Message, sighash::SighashCache, transaction, Amount,
    EcdsaSighashType, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
};
use bitcoincore_rpc::{json::GetTransactionResult, Auth, Client, RpcApi};
use bitvmx_unstable::{
    config::{Config, DispatcherConfig, KeyManagerConfig},
    orchestrator::{Orchestrator, OrchestratorApi},
    step_handler::{StepHandler, StepHandlerApi},
    types::{BitvmxInstance, FundingTx, TransactionFullInfo},
};
use console::style;
use log::info;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::mpsc::channel;
use storage_backend::storage::{KeyValueStore, Storage};

use bitvmx_unstable::errors::BitVMXError;
use key_manager::{key_manager::KeyManager, keystorage::file::FileKeyStore};
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

    let list = client.list_wallets()?;
    info!("{} {:?}", style("Wallet list").green(), list);

    let account = Account::new(network);
    let checkpoint_height = config.monitor.checkpoint_height;
    let key_manager = create_key_manager(&config.key_manager, network)?;
    let dispatcher = TransactionDispatcher::new(client, key_manager);
    let orchestrator = Orchestrator::new(
        &config.rpc.url,
        &config.database.path,
        checkpoint_height,
        dispatcher,
        account.clone(),
    )
    .context("Failed to create Orchestrator instance")?;

    let client = Client::new(
        config.rpc.url.as_str(),
        Auth::UserPass(
            config.rpc.username.as_str().to_string(),
            config.rpc.password.as_str().to_string(),
        ),
    )?;

    // Step 1: Create an instance with 2 transactions for different operators
    println!(
        "\n{} Step 1: Creating an instance with 2 transactions for different operators...\n",
        style("Step 1").blue()
    );

    let instance = create_instance(&account, &client, network, &config.dispatcher)?;

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
    let storage = Storage::new_with_path(&PathBuf::from(config.database.path + "/step_handler"))?;

    let key = format!(
        "instance/{}/tx/{}/sent",
        instance.instance_id,
        instance.txs[0].tx.compute_txid()
    );

    storage.set(key, false)?;

    let key = format!(
        "instance/{}/tx/{}",
        instance.instance_id,
        instance.txs[0].tx.compute_txid()
    );

    storage.set(key, instance.txs[1].tx.clone())?;

    let mut step_handler = StepHandler::new(orchestrator, storage)?;

    let (tx, rx) = channel();
    ctrlc::set_handler(move || tx.send(()).expect("Could not send signal on channel."))
        .expect("Error setting Ctrl-C handler");

    loop {
        if rx.try_recv().is_ok() {
            info!("{}", style("Stopping Bitvmx Runner").red());
            break;
        }

        info!("\n New tick for for Step Handler");

        step_handler.tick()?;

        wait();
    }

    Ok(())
}

fn wait() {
    std::thread::sleep(std::time::Duration::from_secs(6));
}

/// Creates a new transaction.
fn create_instance(
    user: &Account,
    rpc: &Client,
    network: Network,
    dispatcher: &DispatcherConfig,
) -> Result<BitvmxInstance<TransactionFullInfo>> {
    let instance_id = 1; // Example instance ID

    //hardcoded transaction.
    let funding_tx_id =
        Txid::from_str("3a3f8d147abf0b9b9d25b07de7a16a4db96bda3e474ceab4c4f9e8e107d5b02f").unwrap();

    let funding_tx = FundingTx {
        tx_id: funding_tx_id,
        utxo_index: 0,
        utxo_output: TxOut {
            value: Amount::default(),
            script_pubkey: ScriptBuf::default(),
        },
    };

    let tx_1: Transaction = generate_tx(user, rpc, network, dispatcher)?;
    let tx_2: Transaction = generate_tx(user, rpc, network, dispatcher)?;

    let txs = vec![
        TransactionFullInfo {
            tx: tx_1.clone(),
            owner_operator_id: 1,
        },
        TransactionFullInfo {
            tx: tx_2.clone(),
            owner_operator_id: 2,
        },
    ];

    println!("{} Create Instance id: 1", style("→").cyan());

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

    let instance = BitvmxInstance {
        instance_id,
        txs,
        funding_tx,
    };

    Ok(instance)
}

fn generate_tx(
    user: &Account,
    rpc: &Client,
    network: Network,
    dispatcher: &DispatcherConfig,
) -> Result<Transaction> {
    // build and send a mock transaction that we can spend in our drp transaction
    let tx_info = make_mock_output(rpc, user, network)?;

    let spent_amount = tx_info.amount.unsigned_abs();
    let fee = Amount::from_sat(dispatcher.cpfp_fee);
    let cpfp_amount = Amount::from_sat(dispatcher.cpfp_amount);
    let drp_amount = spent_amount - fee - cpfp_amount;

    // The input for the transaction we are constructing.
    let input = TxIn {
        previous_output: OutPoint {
            txid: tx_info.info.txid,
            vout: tx_info
                .details
                .first()
                .expect("No details found for transaction")
                .vout,
        },
        script_sig: ScriptBuf::default(), // For a p2wpkh script_sig is empty.
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: Witness::default(), // Filled in after signing.
    };

    // The drp output. For this example, we just pay back to the user.
    let drp = TxOut {
        value: drp_amount,
        script_pubkey: user.address_checked(network)?.script_pubkey(),
    };

    // The cpfp output is locked to a key controlled by the user.
    let cpfp = TxOut {
        value: cpfp_amount,
        script_pubkey: ScriptBuf::new_p2wpkh(&user.wpkh),
    };

    let tx = build_transaction(vec![input], vec![drp, cpfp], user.clone(), spent_amount)?;

    Ok(tx)
}

fn make_mock_output(
    rpc: &Client,
    user: &Account,
    network: Network,
) -> Result<GetTransactionResult> {
    // fund the user address
    let txid = rpc.send_to_address(
        &user.address_checked(network)?,
        Amount::from_sat(100_000_000), // 1 BTC
        None,
        None,
        None,
        None,
        None,
        None,
    )?;

    // get transaction details
    Ok(rpc.get_transaction(&txid, Some(true))?)
}

fn send_transaction(tx: Transaction, config: &Config, network: Network) -> Result<()> {
    let rpc = Client::new(
        config.rpc.url.as_str(),
        Auth::UserPass(
            config.rpc.username.as_str().to_string(),
            config.rpc.password.as_str().to_string(),
        ),
    )
    .unwrap();

    let key_manager = create_key_manager(&config.key_manager, network)?;
    let dispatcher = TransactionDispatcher::new(rpc, key_manager);

    dispatcher.send(tx)?;

    Ok(())
}

fn create_key_manager(
    key_manager: &KeyManagerConfig,
    network: Network,
) -> Result<KeyManager<FileKeyStore>> {
    let key_derivation_seed = get_key_derivation_seed(key_manager.key_derivation_seed.clone())?;
    let key_derivation_path = &key_manager.key_derivation_path;
    let winternitz_seed = get_winternitz_seed(key_manager.winternitz_seed.clone())?;
    let path = key_manager.storage.path.as_str();
    let password = key_manager.storage.password.as_bytes().to_vec();
    let key_store = FileKeyStore::new(path, password, network)?;
    let key_manager = KeyManager::new(
        network,
        key_derivation_path,
        key_derivation_seed,
        winternitz_seed,
        key_store,
    )?;
    Ok(key_manager)
}

fn get_winternitz_seed(wintenitz_seed: String) -> Result<[u8; 32]> {
    let winternitz_seed = hex::decode(wintenitz_seed.clone())?;

    if winternitz_seed.len() > 32 {
        return Err(BitVMXError::Unexpected(
            "Winternitz secret length must be 32 bytes".to_string(),
        )
        .into());
    }

    Ok(winternitz_seed.as_slice().try_into()?)
}

fn get_key_derivation_seed(key_derivation_seed: String) -> Result<[u8; 32]> {
    let key_derivation_seed = hex::decode(key_derivation_seed.clone())?;

    if key_derivation_seed.len() > 32 {
        return Err(BitVMXError::Unexpected(
            "Key derivation seed length must be 32 bytes".to_string(),
        )
        .into());
    }

    Ok(key_derivation_seed.as_slice().try_into()?)
}

/// Builds a transaction with a single input and multiple outputs.
fn build_transaction(
    inputs: Vec<TxIn>,
    outputs: Vec<TxOut>,
    account: Account,
    spent_amount: Amount,
) -> Result<Transaction> {
    // TODO support multiple inputs and accounts (we only support one input, for now)
    // The transaction we want to sign and broadcast.
    let mut unsigned_tx = Transaction {
        version: transaction::Version::TWO,  // Post BIP-68.
        lock_time: absolute::LockTime::ZERO, // Ignore the locktime.
        input: inputs,                       // Input goes into index 0.
        output: outputs,                     // cpfp output is always index 0.
    };
    let input_index = 0;

    // Get the sighash to sign.
    let sighash_type = EcdsaSighashType::All;
    let mut sighasher = SighashCache::new(&mut unsigned_tx);
    let sighash = sighasher
        .p2wpkh_signature_hash(
            input_index,
            &ScriptBuf::new_p2wpkh(&account.wpkh),
            spent_amount,
            sighash_type,
        )
        .expect("failed to create sighash");

    // Sign the sighash using the secp256k1 library (exported by rust-bitcoin).
    let msg = Message::from(sighash);
    let secp = Secp256k1::new();
    let signature = secp.sign_ecdsa(&msg, &account.sk);

    // Update the witness stack.
    let signature = bitcoin::ecdsa::Signature {
        signature,
        sighash_type,
    };
    let pk = account.sk.public_key(&secp);
    *sighasher.witness_mut(input_index).unwrap() = Witness::p2wpkh(&signature, &pk);

    // Get the signed transaction.
    Ok(sighasher.into_transaction().to_owned())
}

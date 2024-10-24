use anyhow::{Context, Ok, Result};
use bitcoin::Network;
use bitvmx_unstable::{
    config::{Config, DispatcherConfig, KeyManagerConfig},
    orchestrator::{Orchestrator, OrchestratorApi},
};
use std::str::FromStr;
use std::sync::mpsc::channel;
use std::{collections::HashMap, path::PathBuf};
use tracing::info;

use bitcoin::{
    absolute, consensus, key::Secp256k1, secp256k1::Message, sighash::SighashCache, transaction,
    Address, Amount, EcdsaSighashType, OutPoint, PrivateKey, ScriptBuf, Sequence, Transaction,
    TxIn, TxOut, Txid, Witness,
};
use bitcoincore_rpc::{json::GetTransactionResult, Auth, Client, RpcApi};
use console::style;
use serde_json::json;
use storage_backend::storage::{KeyValueStore, Storage};

use bitvmx_unstable::{
    errors::BitVMXError,
    model::{DispatcherTask, DispatcherTaskKind, DispatcherTaskStatus},
};
use key_manager::{key_manager::KeyManager, keystorage::file::FileKeyStore};
use transaction_dispatcher::{dispatcher::TransactionDispatcher, signer::Account};

fn main() -> Result<()> {
    println!(
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

    let account = Account::new(network);

    let node_rpc_url = config.rpc.url.clone();
    let db_file_path = config.database.path;
    let checkpoint_height = config.monitor.checkpoint_height;

    let key_manager = create_key_manager(&config.key_manager, network)?;
    let dispatcher = TransactionDispatcher::new(client, key_manager);

    let mut orchestrator = Orchestrator::new(
        &node_rpc_url,
        &db_file_path,
        checkpoint_height,
        dispatcher,
        account.clone(),
    )
    .context("Failed to create Orchestrator instance")?;

    let (tx, rx) = channel();
    ctrlc::set_handler(move || tx.send(()).expect("Could not send signal on channel."))
        .expect("Error setting Ctrl-C handler");
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

fn old_main() -> Result<()> {
    println!(
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

    // Create a storage or open one if present
    let storage = Storage::new_with_path(&PathBuf::from(&config.database.path2))?;

    // Create a node wallet
    let _ = client.create_wallet("test_wallet", None, None, None, None);

    // Generate an address for our miner in the rpc wallet
    let miner = client
        .get_new_address(None, None)?
        .require_network(network)?;

    let mut key_manager = create_key_manager(&config.key_manager, network)?;

    // create a user account whose keys we control and persist it to db
    let account = Account::new(network);

    storage.write(
        &account.address_checked(network)?.to_string(),
        &serde_json::to_string(&account)?,
    )?;
    let private_key = PrivateKey::new(account.sk, network);
    let _ = key_manager
        .import_private_key(&private_key.to_wif())
        .unwrap();

    println!(
        "{} User address: {:#?}",
        style("→").cyan(),
        account.address_checked(network)?.to_string()
    );

    let node_rpc_url = config.rpc.url.clone();
    let db_file_path = config.database.path;
    let checkpoint_height = config.monitor.checkpoint_height;
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

    let mut orchestrator = Orchestrator::new(
        &node_rpc_url,
        &db_file_path,
        checkpoint_height,
        dispatcher,
        account.clone(),
    )
    .context("Failed to create Orchestrator instance")?;

    // Mine blocks to collect block rewards
    client.generate_to_address(105, &miner)?;

    // build transactions mocks and save them to db
    let drp_transaction = get_drp_transaction_mock(
        &config.dispatcher,
        &account.clone(),
        network,
        &client,
        &miner,
    )?;
    storage.set(drp_transaction.compute_txid().to_string(), &drp_transaction)?;

    println!(
        "{} DRP transaction: {:#?}",
        style("→").cyan(),
        drp_transaction.compute_txid()
    );

    let funding_transaction = get_funding_transaction_mock(&client, &miner, network, &account)?;
    send_funding_tx(&funding_transaction, &client, &miner)?;
    storage.set(
        funding_transaction.compute_txid().to_string(),
        &funding_transaction,
    )?;
    println!(
        "{} Funding transaction: {:#?}",
        style("→").cyan(),
        funding_transaction.compute_txid()
    );

    let task_id = test_send_drp_transaction(
        &drp_transaction.compute_txid(),
        &storage,
        &Config::load()?,
        network,
    )?;
    test_retrieve_task(task_id, &storage)?;

    let (task_id, txid) = test_speedup_drp_transaction(
        drp_transaction.compute_txid(),
        funding_transaction.compute_txid(),
        account,
        &storage,
        network,
        &Config::load()?,
    )?;
    test_retrieve_task(task_id, &storage)?;
    test_speedup_confirmation(txid, &client, &miner)?;

    let (tx, rx) = channel();
    ctrlc::set_handler(move || tx.send(()).expect("Could not send signal on channel."))
        .expect("Error setting Ctrl-C handler");
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

/// Returns a Transaction mocking one of BitVMX DRP transactions.
fn get_drp_transaction_mock(
    dispatcher: &DispatcherConfig,
    user: &Account,
    network: Network,
    rpc: &Client,
    miner: &Address,
) -> Result<Transaction> {
    // build and send a mock transaction that we can spend in our drp transaction
    let tx_info = make_mock_output(rpc, user, network, miner)?;

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

    build_transaction(vec![input], vec![drp, cpfp], user.clone(), spent_amount)
}

/// Returns a Transaction to be used as funding for speeding up a DRP transaction.
fn get_funding_transaction_mock(
    rpc: &Client,
    miner: &Address,
    network: Network,
    user: &Account,
) -> Result<Transaction> {
    // build and send a mock transaction that we can spend in our funding transaction
    let tx_info = make_mock_output(rpc, user, network, miner)?;

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

    // The spend output is locked to a key controlled by the user.
    let spent_amount = tx_info.amount.unsigned_abs();

    static DEFAULT_FEE: Amount = Amount::from_sat(1_000_000); // 0.01 BTC

    let output = TxOut {
        value: spent_amount - DEFAULT_FEE,
        script_pubkey: user.address_checked(network)?.script_pubkey(),
    };

    build_transaction(
        vec![input],
        vec![output],
        user.clone(),
        tx_info.amount.unsigned_abs(),
    )
}

fn make_mock_output(
    rpc: &Client,
    user: &Account,
    network: Network,
    miner: &Address,
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

    // mine a block to confirm transaction
    rpc.generate_to_address(1, &miner)?;

    // get transaction details
    Ok(rpc.get_transaction(&txid, Some(true))?)
}

fn test_send_drp_transaction(
    transaction_id: &Txid,
    store: &Storage,
    config: &Config,
    network: Network,
) -> Result<String> {
    println!("\nSending DRP transaction...");

    // retrieve the transaction from the database
    let saved_tx: Option<Transaction> = store.get(transaction_id.to_string())?;

    let tx = saved_tx.ok_or_else(|| {
        BitVMXError::Unexpected(format!(
            "Transaction {} not found in database",
            transaction_id
        ))
    })?;

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

    // create a new `Send` task for the dispatcher
    let task = DispatcherTask {
        transaction_id: tx.compute_txid(),
        child_tx: None,
        kind: DispatcherTaskKind::Send,
        status: DispatcherTaskStatus::None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let task_id = store.save(task)?;

    // dispatch tx!
    dispatcher.send(tx)?;

    // update task status
    let task_updates = HashMap::from([
        ("status", json!(DispatcherTaskStatus::Sent)),
        ("updated_at", json!(chrono::Utc::now())),
    ]);

    store
        .update::<DispatcherTask>(&task_id, task_updates)
        .context("While updating dispatcher task")?;

    Ok(task_id)
}

fn test_retrieve_task(task_id: String, store: &Storage) -> Result<()> {
    let task: Option<DispatcherTask> = store.get(task_id)?;
    assert!(task.is_some(), "Task not found in database");

    let task = task.unwrap();
    println!("{} Task: {:#?}", style("→").magenta(), task);
    assert_eq!(task.status, DispatcherTaskStatus::Sent);
    Ok(())
}

fn test_speedup_drp_transaction(
    drp_txid: Txid,
    funding_txid: Txid,
    user: Account,
    store: &Storage,
    network: Network,
    config: &Config,
) -> Result<(String, Txid)> {
    println!("Speeding up DRP transaction...");

    let public_key_drptx = user.pk;
    let public_key_fundingtx = user.pk;

    // get transactions from the database
    let drp_tx: Transaction = store.get(drp_txid.to_string())?.ok_or_else(|| {
        BitVMXError::Unexpected(format!("Transaction {} not found in database", drp_txid))
    })?;

    let funding_tx: Transaction = store.get(funding_txid.to_string())?.ok_or_else(|| {
        BitVMXError::Unexpected(format!(
            "Transaction {} not found in database",
            funding_txid
        ))
    })?;

    // create a new `Speedup` task for the dispatcher
    let task = DispatcherTask {
        transaction_id: drp_tx.compute_txid(),
        child_tx: None,
        kind: DispatcherTaskKind::Speedup,
        status: DispatcherTaskStatus::None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let task_id = store.save(task)?;

    // create a new dispatcher and send transaction
    let rpc = Client::new(
        config.rpc.url.as_str(),
        Auth::UserPass(
            config.rpc.username.as_str().to_string(),
            config.rpc.password.as_str().to_string(),
        ),
    )
    .unwrap();

    let key_manager = create_key_manager(&config.key_manager, network)?;
    let mut dispatcher = TransactionDispatcher::new(rpc, key_manager);

    let funding_utxo = get_utxo(&funding_tx, user.address_checked(network)?)?;
    let funding_utxo = (funding_utxo.0, funding_utxo.1, public_key_fundingtx);
    let (txid, _) = dispatcher.speed_up(
        &drp_tx,
        public_key_drptx,
        funding_tx.compute_txid(),
        funding_utxo,
    )?;

    // update task child tx
    let task_updates = HashMap::from([
        ("child_tx", json!(txid)),
        ("status", json!(DispatcherTaskStatus::Sent)),
        ("updated_at", json!(chrono::Utc::now())),
    ]);

    store
        .update::<DispatcherTask>(&task_id, task_updates)
        .context("While updating dispatcher task")?;

    // save child tx to database
    let rpc = Client::new(
        config.rpc.url.as_str(),
        Auth::UserPass(
            config.rpc.username.as_str().to_string(),
            config.rpc.password.as_str().to_string(),
        ),
    )
    .unwrap();

    let child_tx = rpc.get_raw_transaction(&txid, None)?;
    store.set(txid.to_string(), child_tx)?;

    Ok((task_id, txid))
}

fn test_speedup_confirmation(txid: Txid, rpc: &Client, miner: &Address) -> Result<()> {
    print!("Checking speedup confirmation...");

    rpc.generate_to_address(1, &miner)?;
    let tx_result = rpc.get_raw_transaction_info(&txid, None)?;

    assert_eq!(tx_result.confirmations, Some(1));

    println!(" {}", style("✔").cyan());
    Ok(())
}

fn send_funding_tx(funding_tx: &Transaction, rpc: &Client, miner: &Address) -> Result<()> {
    let serialized_tx = consensus::encode::serialize_hex(&funding_tx);
    let txid = rpc.send_raw_transaction(serialized_tx)?;

    rpc.generate_to_address(1, &miner)?;

    let tx_result = rpc.get_raw_transaction_info(&txid, None)?;
    assert_eq!(tx_result.confirmations, Some(1));

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

/// Get the UTXO paying to the given address. If there's more than one,
/// return the first one.
fn get_utxo(tx: &Transaction, address: Address) -> Result<(u32, TxOut)> {
    for (index, output) in tx.output.iter().enumerate() {
        if address.matches_script_pubkey(&output.script_pubkey) {
            return Ok((index as u32, output.clone()));
        }
    }

    Err(BitVMXError::Unexpected(format!(
        "No UTXO paying to {} found in transaction {}",
        address,
        tx.compute_txid()
    ))
    .into())
}

use std::{collections::HashMap, path::PathBuf, str::FromStr};

use anyhow::{Context, Result};
use bitcoin::{absolute, consensus, key::Secp256k1, secp256k1::Message, sighash::SighashCache, transaction, Address, Amount, EcdsaSighashType, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness};
use bitcoincore_rpc::{json::GetTransactionResult, Auth, Client, RpcApi};
use console::style;
use rust_bitvmx_storage_backend::storage::{KeyValueStore, Storage};
use serde_json::json;
use tracing::{error, Level};

use bitvmx_unstable::{config::Config, errors::BitVMXError, model::{DispatcherTask, DispatcherTaskKind, DispatcherTaskStatus}};
use transaction_dispatcher::{dispatcher::TransactionDispatcher, signer::{Account, Signer}};


static DEFAULT_FEE: Amount = Amount::from_sat(1_000_000); // 0.01 BTC


fn main() -> Result<()> {
    println!(
        "\n{} I'm here to showcase the interaction between the different BitVMX modules.\n",
        style("Hi!").cyan()
    );

    tracing_subscriber::fmt()
        .without_time()
        // .with_target(false)
        .with_max_level(Level::ERROR)
        .init();

    let test = match Test::new() {
        Ok(test) => test,
        Err(e) => {
            error!("{:?}", e);
            std::process::exit(1);
        },
    };

    if let Err(e) = test.run() {
        error!("{:?}", e);
        std::process::exit(1);
    }
    Ok(())
}

struct Test {
    config: Config,
    network: Network,
    rpc: Client,
    db: Storage,
    miner: Address,
    user: Account,
}

impl Test {
    fn new() -> Result<Self> {
        let config = Config::load()?;
        let network = Network::from_str(config.rpc.network.as_str())?;
        let rpc = Client::new(
            config.rpc.url.as_str(),
            Auth::UserPass(
                config.rpc.username.as_str().to_string(),
                config.rpc.password.as_str().to_string(),
            ),
        )?;

        // Create a storage or open one if present
        let db = Storage::new_with_path(&PathBuf::from(&config.database.path))?;

        // Create a node wallet
        let _ = rpc.create_wallet("test_wallet", None, None, None, None);

        // Generate an address for our miner in the rpc wallet
        let miner = rpc
            .get_new_address(None, None)?
            .require_network(network)?;

        // create a user account whose keys we control and persist it to db
        let user = Account::new(network);
        db.write(
            &user.address_checked(network)?.to_string(),
            &serde_json::to_string(&user)?
        )?;
        println!("{} User address: {:#?}", style("→").cyan(), user.address_checked(network)?.to_string());

        Ok( Self { config, network, rpc, db, miner, user })
    }

    pub fn run(&self) -> Result<()> {
        // Mine blocks to collect block rewards
        self.rpc.generate_to_address(105, &self.miner)?;

        // build transactions mocks and save them to db
        let drp_transaction = self.get_drp_transaction_mock()?;
        self.db.set(drp_transaction.compute_txid().to_string(), &drp_transaction)?;
        println!("{} DRP transaction: {:#?}", style("→").cyan(), drp_transaction.compute_txid());

        let funding_transaction = self.get_funding_transaction_mock()?;
        self.send_funding_tx(&funding_transaction)?;
        self.db.set(funding_transaction.compute_txid().to_string(), &funding_transaction)?;
        println!("{} Funding transaction: {:#?}", style("→").cyan(), funding_transaction.compute_txid());

        let task_id = self.test_send_drp_transaction(&drp_transaction.compute_txid())?;
        self.test_retrieve_task(task_id)?;
        
        let (task_id, txid) = self.test_speedup_drp_transaction(drp_transaction.compute_txid(), funding_transaction.compute_txid())?;
        self.test_retrieve_task(task_id)?;
        self.test_speedup_confirmation(txid)?;
        Ok(())
    }

    /// Returns a Transaction mocking one of BitVMX DRP transactions.
    fn get_drp_transaction_mock(&self) -> Result<Transaction> {
        // build and send a mock transaction that we can spend in our drp transaction
        let tx_info = self.make_mock_output()?;

        let spent_amount = tx_info.amount.unsigned_abs();
        let fee = Amount::from_sat(self.config.dispatcher.cpfp_fee);
        let cpfp_amount = Amount::from_sat(self.config.dispatcher.cpfp_amount);
        let drp_amount = spent_amount - fee - cpfp_amount;

        // The input for the transaction we are constructing.
        let input = TxIn {
            previous_output: OutPoint {
                txid: tx_info.info.txid,
                vout: tx_info.details.first().expect("No details found for transaction").vout,
            },
            script_sig: ScriptBuf::default(), // For a p2wpkh script_sig is empty.
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::default(), // Filled in after signing.
        };

        // The drp output. For this example, we just pay back to the user.
        let drp = TxOut {
            value: drp_amount,
            script_pubkey: self.user.address_checked(self.network)?.script_pubkey(),
        };
    
        // The cpfp output is locked to a key controlled by the user.
        let cpfp = TxOut {
            value: cpfp_amount,
            script_pubkey: ScriptBuf::new_p2wpkh(&self.user.wpkh),
        };

        build_transaction(
            vec![input],
            vec![drp, cpfp],
            self.user.clone(),
            spent_amount,
        )
    }

    /// Returns a Transaction to be used as funding for speeding up a DRP transaction.
    fn get_funding_transaction_mock(&self) -> Result<Transaction> {
        // build and send a mock transaction that we can spend in our funding transaction
        let tx_info = self.make_mock_output()?;

        // The input for the transaction we are constructing.
        let input = TxIn {
            previous_output: OutPoint {
                txid: tx_info.info.txid,
                vout: tx_info.details.first().expect("No details found for transaction").vout,
            },
            script_sig: ScriptBuf::default(), // For a p2wpkh script_sig is empty.
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::default(), // Filled in after signing.
        };

        // The spend output is locked to a key controlled by the user.
        let spent_amount = tx_info.amount.unsigned_abs();
        let output = TxOut {
            value: spent_amount - DEFAULT_FEE,
            script_pubkey: self.user.address_checked(self.network)?.script_pubkey(),
        };

        build_transaction(
            vec![input],
            vec![output],
            self.user.clone(),
            tx_info.amount.unsigned_abs(),
        )
    }

    fn make_mock_output(&self) -> Result<GetTransactionResult> {
        // fund the user address
        let txid = self.rpc.send_to_address(
            &self.user.address_checked(self.network)?,
            Amount::from_sat(100_000_000), // 1 BTC
            None,
            None,
            None,
            None,
            None,
            None,
        )?;

        // mine a block to confirm transaction
        self.rpc.generate_to_address(1, &self.miner)?;

        // get transaction details
        Ok(self.rpc.get_transaction(&txid, Some(true))?)
    }

    fn create_dispatcher(&self) -> Result<TransactionDispatcher> {
        // create rpc for the dispatcher
        let rpc = Client::new(
            self.config.rpc.url.as_str(),
            Auth::UserPass(
                self.config.rpc.username.as_str().to_string(),
                self.config.rpc.password.as_str().to_string(),
            ),
        ).unwrap();

        // create a signer for the dispatcher
        let mut signer = Signer::new(None);
        signer.add_account("user".to_string(), self.user.clone());

        Ok(TransactionDispatcher::new(rpc, signer, self.network))
    }

    fn test_send_drp_transaction(&self, transaction_id: &Txid) -> Result<String>{
        println!("\nSending DRP transaction...");

        // retrieve the transaction from the database
        let saved_tx: Option<Transaction> = self.db.get(transaction_id.to_string())?;

        let tx = saved_tx.ok_or_else(||
            BitVMXError::Unexpected(format!("Transaction {} not found in database", transaction_id))
        )?;

        // create a new dispatcher
        let dispatcher = self.create_dispatcher()?;

        // create a new `Send` task for the dispatcher
        let task = DispatcherTask {
            transaction_id: tx.compute_txid(),
            child_tx: None,
            kind: DispatcherTaskKind::Send,
            status: DispatcherTaskStatus::None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let task_id = self.db.save(task)?;

        // dispatch tx!
        dispatcher.send(tx)?;

        // update task status
        let task_updates = HashMap::from([
            ("status", json!(DispatcherTaskStatus::Sent)),
            ("updated_at", json!(chrono::Utc::now())),
        ]);

        self.db.update::<DispatcherTask>(&task_id, task_updates)
            .context("While updating dispatcher task")?;

        Ok(task_id)
    }
    
    fn test_retrieve_task(&self, task_id: String) -> Result<()> {
        let task: Option<DispatcherTask> = self.db.get(task_id)?;
        assert!(task.is_some(), "Task not found in database");

        let task = task.unwrap();
        println!("{} Task: {:#?}", style("→").magenta(), task);
        assert_eq!(task.status, DispatcherTaskStatus::Sent);
        Ok(())
    }

    fn test_speedup_drp_transaction(&self, drp_txid: Txid, funding_txid: Txid) -> Result<(String, Txid)> {
        println!("Speeding up DRP transaction...");

        // get transactions from the database
        let drp_tx: Transaction = self.db.get(drp_txid.to_string())?.ok_or_else(||
            BitVMXError::Unexpected(format!("Transaction {} not found in database", drp_txid))
        )?;

        let funding_tx: Transaction = self.db.get(funding_txid.to_string())?.ok_or_else(||
            BitVMXError::Unexpected(format!("Transaction {} not found in database", funding_txid))
        )?;
        
        // create a new `Speedup` task for the dispatcher
        let task = DispatcherTask {
            transaction_id: drp_tx.compute_txid(),
            child_tx: None,
            kind: DispatcherTaskKind::Speedup,
            status: DispatcherTaskStatus::None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let task_id = self.db.save(task)?;
        
        // create a new dispatcher and send transaction
        let mut dispatcher = self.create_dispatcher()?;
        let funding_utxo = get_utxo(&funding_tx, self.user.address_checked(self.network)?)?;
        let txid = dispatcher.speed_up(&drp_tx, funding_tx.compute_txid(), funding_utxo)?;

        // update task child tx
        let task_updates = HashMap::from([
            ("child_tx", json!(txid)),
            ("status", json!(DispatcherTaskStatus::Sent)),
            ("updated_at", json!(chrono::Utc::now())),
        ]);

        self.db.update::<DispatcherTask>(&task_id, task_updates)
            .context("While updating dispatcher task")?;

        // save child tx to database
        let child_tx = self.rpc.get_raw_transaction(&txid, None)?;
        self.db.set(txid.to_string(), child_tx)?;

        Ok((task_id, txid))
    }

    fn test_speedup_confirmation(&self, txid: Txid) -> Result<()> {
        print!("Checking speedup confirmation...");

        self.rpc.generate_to_address(1, &self.miner)?;
        let tx_result = self.rpc.get_raw_transaction_info(&txid, None)?;

        assert_eq!(tx_result.confirmations, Some(1));

        println!(" {}", style("✔").cyan());
        Ok(())
    }
    
    fn send_funding_tx(&self, funding_tx: &Transaction) -> Result<()> {
        let serialized_tx = consensus::encode::serialize_hex(&funding_tx);
        let txid = self.rpc.send_raw_transaction(serialized_tx)?;

        self.rpc.generate_to_address(1, &self.miner)?;
        
        let tx_result = self.rpc.get_raw_transaction_info(&txid, None)?;
        assert_eq!(tx_result.confirmations, Some(1));
        
        Ok(())
    }
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
        input: inputs,                  // Input goes into index 0.
        output: outputs,           // cpfp output is always index 0.
    };
    let input_index = 0;

    // Get the sighash to sign.
    let sighash_type = EcdsaSighashType::All;
    let mut sighasher = SighashCache::new(&mut unsigned_tx);
    let sighash = sighasher.p2wpkh_signature_hash(
            input_index,
            &ScriptBuf::new_p2wpkh(&account.wpkh),
            spent_amount,
            sighash_type,
        ).expect("failed to create sighash");

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
fn get_utxo(tx: &Transaction, address: Address) -> Result<(u32, TxOut)>{
    for (index, output) in tx.output.iter().enumerate() {
        if address.matches_script_pubkey(&output.script_pubkey) {
            return Ok((index as u32, output.clone()));
        }
    }

    Err(BitVMXError::Unexpected(
        format!("No UTXO paying to {} found in transaction {}", address, tx.compute_txid())
    ).into())
}

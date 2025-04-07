use bitcoin::Network;
use bitcoin::{
    absolute, key::Secp256k1, secp256k1::Message, sighash::SighashCache, transaction, Amount,
    EcdsaSighashType, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
};
use bitcoincore_rpc::{json::GetTransactionResult, Auth, Client, RpcApi};

use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use console::style;
use key_manager::errors::KeyManagerError;
use key_manager::{create_file_key_store_from_config, create_key_manager_from_config};
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use storage_backend::storage::Storage;
use transaction_dispatcher::dispatcher::TransactionDispatcherApi;
use transaction_dispatcher::signer::AccountApi;
use uuid::Uuid;

use key_manager::{key_manager::KeyManager, keystorage::file::FileKeyStore};
use transaction_dispatcher::{dispatcher::TransactionDispatcher, signer::Account};

use crate::config::{Config, DispatcherConfig};
use crate::errors::TxBuilderHelperError;
use crate::types::{TransactionDispatch, FundingTransaction, TransactionFullInfo};

pub fn create_instance(
    user: &Account,
    rpc_config: &RpcConfig,
    network: Network,
    dispatcher: &DispatcherConfig,
) -> Result<TransactionDispatch<TransactionFullInfo>, TxBuilderHelperError> {
    let instance_id = Uuid::from_u128(1);

    //hardcoded transaction.
    let funding_tx_id =
        Txid::from_str("3a3f8d147abf0b9b9d25b07de7a16a4db96bda3e474ceab4c4f9e8e107d5b02f").unwrap();

    let funding_tx = Some(FundingTransaction {
        tx_id: funding_tx_id,
        utxo_index: 0,
        utxo_output: TxOut {
            value: Amount::default(),
            script_pubkey: ScriptBuf::default(),
        },
    });

    let tx_1: Transaction = generate_tx(user, rpc_config, network, dispatcher)?;
    let tx_2: Transaction = generate_tx(user, rpc_config, network, dispatcher)?;

    let txs = vec![
        TransactionFullInfo { tx: tx_1.clone() },
        TransactionFullInfo { tx: tx_2.clone() },
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

    let instance = TransactionDispatch {
        id: instance_id,
        txs,
        funding_tx,
    };

    Ok(instance)
}

pub fn generate_tx(
    user: &Account,
    rpc_config: &RpcConfig,
    network: Network,
    dispatcher: &DispatcherConfig,
) -> Result<Transaction, TxBuilderHelperError> {
    // build and send a mock transaction that we can spend in our drp transaction
    let tx_info = make_mock_output(rpc_config, user, network)?;
    let spent_amount = tx_info.amount.unsigned_abs();
    let fee = Amount::from_sat(dispatcher.cpfp_fee);
    //Child Pays For Parent Amount
    let cpfp_amount = Amount::from_sat(dispatcher.cpfp_amount);

    // reciduo.
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

pub fn make_mock_output(
    rpc_config: &RpcConfig,
    user: &Account,
    network: Network,
) -> Result<GetTransactionResult, TxBuilderHelperError> {
    let client = Client::new(
        rpc_config.url.as_str(),
        Auth::UserPass(
            rpc_config.username.as_str().to_string(),
            rpc_config.password.as_str().to_string(),
        ),
    )?;

    // fund the user address
    let txid = client.send_to_address(
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
    Ok(client.get_transaction(&txid, Some(true))?)
}

pub fn send_transaction(tx: Transaction, config: &Config) -> Result<(), TxBuilderHelperError> {
    let key_manager = create_key_manager(config)?;
    let dispatcher = TransactionDispatcher::new_with_path(&config.rpc, Rc::new(key_manager))?;

    dispatcher.send(tx)?;

    Ok(())
}

pub fn create_key_manager(config: &Config) -> Result<KeyManager<FileKeyStore>, KeyManagerError> {
    let key_storage =
        create_file_key_store_from_config(&config.key_storage, &config.key_manager.network)?;

    // TODO read from config
    let path = PathBuf::from(format!("data/development/musig_store"));
    let store = Rc::new(Storage::new_with_path(&path).unwrap());

    create_key_manager_from_config(&config.key_manager, key_storage, store)
}

/// Builds a transaction with a single input and multiple outputs.
pub fn build_transaction(
    inputs: Vec<TxIn>,
    outputs: Vec<TxOut>,
    account: Account,
    spent_amount: Amount,
) -> Result<Transaction, TxBuilderHelperError> {
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

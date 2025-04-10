use bitcoin::Network;
use bitcoin::{
    absolute, key::Secp256k1, secp256k1::Message, sighash::SighashCache, transaction, Amount,
    EcdsaSighashType, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
};
use bitcoin_coordinator::config::DispatcherConfig;
use bitcoin_coordinator::errors::TxBuilderHelperError;
use bitcoin_coordinator::{storage::BitcoinCoordinatorStore, types::FundingTransaction};
use bitcoincore_rpc::{json::GetTransactionResult, Auth, Client, RpcApi};
use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use bitvmx_transaction_monitor::{
    monitor::MockMonitorApi,
    types::{ExtraData, TransactionMonitor},
};
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use storage_backend::storage::Storage;
use transaction_dispatcher::dispatcher::MockTransactionDispatcherApi;
use transaction_dispatcher::signer::Account;
use transaction_dispatcher::signer::AccountApi;
use uuid::Uuid;

pub fn clear_output() {
    let _ = std::fs::remove_dir_all("test_output");
}

pub fn clear_db(path: &str) {
    let _ = std::fs::remove_dir_all(path);
}

pub fn generate_random_string() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..10).map(|_| rng.gen_range('a'..='z')).collect()
}

pub fn get_mocks() -> (
    MockMonitorApi,
    BitcoinCoordinatorStore,
    Account,
    MockTransactionDispatcherApi,
) {
    let mock_monitor = MockMonitorApi::new();
    let path = format!("test_output/test/{}", generate_random_string());
    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(&path)).unwrap());
    let store = BitcoinCoordinatorStore::new(storage).unwrap();
    let network = Network::from_str("regtest").unwrap();
    let account = Account::new(network);
    let mock_dispatcher = MockTransactionDispatcherApi::new();
    (mock_monitor, store, account, mock_dispatcher)
}

pub fn get_mock_data() -> (TransactionMonitor, Transaction, FundingTransaction) {
    let tx = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![],
        output: vec![],
    };

    let tx_id = tx.compute_txid();

    let group_id = Uuid::from_u128(1);

    let funding_tx = FundingTransaction {
        tx_id: tx.compute_txid(),
        utxo_index: 1,
        utxo_output: TxOut {
            value: Amount::default(),
            script_pubkey: ScriptBuf::default(),
        },
    };

    let monitor =
        TransactionMonitor::Transactions(vec![tx_id], ExtraData::Context(group_id.to_string()));

    (monitor, tx, funding_tx)
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

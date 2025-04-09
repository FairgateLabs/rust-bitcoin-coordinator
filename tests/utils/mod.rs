use bitcoin::{absolute, transaction, Amount, Network, ScriptBuf, Transaction, TxOut};
use bitcoin_coordinator::{storage::BitcoinCoordinatorStore, types::FundingTransaction};
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
use uuid::Uuid;

pub fn clear_output() {
    let _ = std::fs::remove_dir_all("test_output");
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

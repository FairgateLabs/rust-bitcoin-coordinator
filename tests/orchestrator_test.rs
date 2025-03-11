use bitcoin::{absolute, transaction, Amount, Network, ScriptBuf, Transaction, TxOut};
use bitcoin_coordinator::{
    coordinator::{BitcoinCoordinator, BitcoinCoordinatorApi},
    storage::BitcoinCoordinatorStore,
    types::{BitvmxInstance, FundingTx, InstanceId, TransactionPartialInfo},
};
use bitvmx_transaction_monitor::{monitor::MockMonitorApi, types::InstanceData};
use mockall::predicate::eq;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use storage_backend::storage::Storage;
use transaction_dispatcher::dispatcher::MockTransactionDispatcherApi;
use transaction_dispatcher::signer::Account;
use uuid::Uuid;

#[test]
fn orchastrator_is_ready_method_test() -> Result<(), anyhow::Error> {
    let (mut mock_monitor, store, account, mock_dispatcher) = get_mocks();

    mock_monitor
        .expect_is_ready()
        .times(1)
        .returning(|| Ok(false));

    mock_monitor
        .expect_is_ready()
        .times(1)
        .returning(|| Ok(true));

    let orchastrator = BitcoinCoordinator::new(mock_monitor, store, mock_dispatcher, account);

    let is_ready = orchastrator.is_ready()?;

    assert!(!is_ready);

    let is_ready = orchastrator.is_ready()?;

    assert!(is_ready);

    Ok(())
}

#[test]
fn tick_method_is_not_ready() -> Result<(), anyhow::Error> {
    let (mut mock_monitor, store, account, mock_dispatcher) = get_mocks();

    // Monitor is not ready then should call monitor tick
    mock_monitor
        .expect_is_ready()
        .times(1)
        .returning(|| Ok(false));

    mock_monitor.expect_tick().times(1).returning(|| Ok(()));

    let orchastrator = BitcoinCoordinator::new(mock_monitor, store, mock_dispatcher, account);

    orchastrator.tick()?;

    Ok(())
}

#[test]
fn monitor_instance_test() -> Result<(), anyhow::Error> {
    let (mut mock_monitor, store, account, mock_dispatcher) = get_mocks();

    let (instance_id, instance, _tx) = get_mock_data();

    let instance_data = InstanceData {
        instance_id,
        txs: vec![instance.txs[0].tx_id],
    };

    mock_monitor
        .expect_save_instances_for_tracking()
        .with(eq(vec![instance_data]))
        .returning(|_| Ok(()));

    let orchastrator = BitcoinCoordinator::new(mock_monitor, store, mock_dispatcher, account);

    orchastrator.monitor_instance(&instance)?;

    Ok(())
}

fn get_mocks() -> (
    MockMonitorApi,
    BitcoinCoordinatorStore,
    Account,
    MockTransactionDispatcherApi,
) {
    let mock_monitor = MockMonitorApi::new();
    let path = format!("data/tests/{}", generate_random_string());
    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(&path)).unwrap());
    let store = BitcoinCoordinatorStore::new(storage).unwrap();
    let network = Network::from_str("regtest").unwrap();
    let account = Account::new(network);
    let mock_dispatcher = MockTransactionDispatcherApi::new();
    (mock_monitor, store, account, mock_dispatcher)
}

fn generate_random_string() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..10).map(|_| rng.gen_range('a'..='z')).collect()
}

fn get_mock_data() -> (
    InstanceId,
    BitvmxInstance<TransactionPartialInfo>,
    Transaction,
) {
    let tx = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![],
        output: vec![],
    };

    let tx_info = TransactionPartialInfo {
        tx_id: tx.compute_txid(),
    };

    let instance_id = Uuid::from_u128(1);

    let instance = BitvmxInstance::<TransactionPartialInfo> {
        instance_id,
        txs: vec![tx_info],
        funding_tx: Some(FundingTx {
            tx_id: tx.compute_txid(),
            utxo_index: 1,
            utxo_output: TxOut {
                value: Amount::default(),
                script_pubkey: ScriptBuf::default(),
            },
        }),
    };
    (instance_id, instance, tx)
}

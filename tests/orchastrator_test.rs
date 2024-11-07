use bitcoin::absolute::LockTime;
use bitcoin::{absolute, transaction, Amount, Network, ScriptBuf, Transaction, TxOut, Txid};
use bitvmx_transaction_monitor::monitor::MockMonitorApi;
use bitvmx_transaction_monitor::types::{InstanceData, TxStatus};
use bitvmx_unstable::orchestrator::{Orchestrator, OrchestratorApi};
use bitvmx_unstable::storage::BitvmxStore;
use bitvmx_unstable::types::{BitvmxInstance, FundingTx, TransactionPartialInfo};
use mockall::predicate::eq;
use std::str::FromStr;
use transaction_dispatcher::dispatcher::MockTransactionDispatcherApi;
use transaction_dispatcher::signer::Account;

#[test]
fn orchastrator_is_ready_method_test() -> Result<(), anyhow::Error> {
    let (mut mock_monitor, mock_store, account, mock_dispatcher) = get_mocks();

    mock_monitor
        .expect_is_ready()
        .times(1)
        .returning(|| Ok(false));

    mock_monitor
        .expect_is_ready()
        .times(1)
        .returning(|| Ok(true));

    let mut orchastrator = Orchestrator::new(mock_monitor, mock_store, mock_dispatcher, account)?;

    let is_ready = orchastrator.is_ready()?;

    assert!(!is_ready);

    let is_ready = orchastrator.is_ready()?;

    assert!(is_ready);

    Ok(())
}

#[test]
fn tick_method_is_not_ready() -> Result<(), anyhow::Error> {
    let (mut mock_monitor, mock_store, account, mock_dispatcher) = get_mocks();

    // Monitor is not ready then should call monitor tick
    mock_monitor
        .expect_is_ready()
        .times(1)
        .returning(|| Ok(false));

    mock_monitor.expect_tick().times(1).returning(|| Ok(()));

    let mut orchastrator = Orchestrator::new(mock_monitor, mock_store, mock_dispatcher, account)?;

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

    let orchastrator = Orchestrator::new(mock_monitor, store, mock_dispatcher, account)?;

    orchastrator.monitor_instance(&instance)?;

    Ok(())
}

#[test]
fn speed_up_tx() -> Result<(), anyhow::Error> {
    let (mut mock_monitor, store, account, mut mock_dispatcher) = get_mocks();

    // TEST: Given a instance with one tx. Make the transaction to be dispatch and speed it up, steps:
    // Call orchastrator and start monitoring instance X with one transcation T
    let (instance_id, instance, tx) = get_mock_data();

    // Mock orchastrator it is always ready:
    mock_monitor.expect_is_ready().returning(|| Ok(true));

    // Mock Dispatch tx, it should try to send T.
    mock_dispatcher
        .expect_send()
        .times(1)
        .with(eq(tx.clone()))
        .returning(move |tx_ret| Ok(tx_ret.compute_txid()));

    mock_monitor
        .expect_get_instance_news()
        .times(2)
        .returning(move || Ok(vec![]));

    let tx_id = tx.compute_txid();

    mock_monitor
        .expect_get_instance_news()
        .times(2)
        .returning(move || Ok(vec![(instance_id, vec![tx_id])]));

    // Status for tx monitor should say that tx_was_seen is false.
    let tx_status = TxStatus {
        tx_id: tx.compute_txid(),
        tx_hex: None,
        tx_was_seen: false,
        height_tx_seen: None,
        confirmations: 0,
    };

    mock_monitor
        .expect_get_instance_tx_status()
        .times(2)
        .with(eq(instance_id), eq(tx_id))
        .returning(move |_, _| Ok(Some(tx_status.clone())));

    let tx_status_confirmed = TxStatus {
        tx_id: tx.compute_txid(),
        tx_hex: Some("Ox123".to_string()),
        tx_was_seen: true,
        height_tx_seen: Some(4),
        confirmations: 1, // is confirmed
    };

    mock_monitor
        .expect_get_instance_tx_status()
        .times(1)
        .with(eq(instance_id), eq(tx_id))
        .returning(move |_, _| Ok(Some(tx_status_confirmed.clone())));

    let tx_status_finalized = TxStatus {
        tx_id: tx.compute_txid(),
        tx_hex: Some("Ox123".to_string()),
        tx_was_seen: true,
        height_tx_seen: Some(4),
        confirmations: 7, // is Finalized
    };

    mock_monitor
        .expect_get_instance_tx_status()
        .times(1)
        .with(eq(instance_id), eq(tx_id))
        .returning(move |_, _| Ok(Some(tx_status_finalized.clone())));

    mock_dispatcher
        .expect_should_speed_up()
        .with(eq(Amount::default()))
        .times(1)
        .returning(|_| Ok(true));

    let tx_speed_up_id_1 =
        Txid::from_str(&"e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200a")
            .unwrap();

    let tx_speed_up_id_2 =
        Txid::from_str(&"e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200b")
            .unwrap();

    mock_dispatcher
        .expect_speed_up()
        .times(1)
        .with(
            eq(tx.clone()),
            eq(account.pk),
            eq(instance.funding_tx.tx_id),
            eq((
                instance.funding_tx.utxo_index,
                instance.funding_tx.utxo_output.clone(),
                account.pk,
            )),
        )
        .returning(move |_, _, _, _| Ok((tx_speed_up_id_1, Amount::default())));

    mock_dispatcher
        .expect_speed_up()
        .times(1)
        .with(
            eq(tx.clone()),
            eq(account.pk),
            eq(instance.funding_tx.tx_id), // We choose this instead previous speed up because it is not mined the speed up
            eq((
                instance.funding_tx.utxo_index, // In this case we are using utxo info from the first tx (in this cas is funding tx)
                instance.funding_tx.utxo_output.clone(),
                account.pk,
            )),
        )
        .returning(move |_, _, _, _| Ok((tx_speed_up_id_2, Amount::default())));

    let instance_data = InstanceData {
        instance_id,
        txs: vec![instance.txs[0].tx_id],
    };

    mock_monitor
        .expect_save_instances_for_tracking()
        .with(eq(vec![instance_data]))
        .returning(|_| Ok(()));

    mock_monitor
        .expect_get_current_height()
        .times(1)
        .returning(|| 0);

    mock_monitor
        .expect_get_current_height()
        .times(1)
        .returning(|| 1);

    mock_monitor
        .expect_get_current_height()
        .times(1)
        .returning(|| 2);

    mock_monitor
        .expect_get_current_height()
        .times(1)
        .returning(|| 3);

    mock_monitor
        .expect_get_current_height()
        .times(1)
        .returning(|| 10); // there was a bunch of new blocks, then we could say that tx gets a bunch of confirmations

    mock_monitor
        .expect_acknowledge_instance_tx_news()
        .with(eq(instance_id), eq(tx_id))
        .times(2)
        .returning(|_, _| Ok(()));

    mock_monitor
        .expect_save_transaction_for_tracking()
        .times(1)
        .with(eq(instance_id), eq(tx_speed_up_id_1))
        .returning(|_, _| Ok(()));

    mock_monitor
        .expect_save_transaction_for_tracking()
        .times(1)
        .with(eq(instance_id), eq(tx_speed_up_id_2))
        .returning(|_, _| Ok(()));

    let mut orchastrator = Orchestrator::new(mock_monitor, store, mock_dispatcher, account)?;
    orchastrator.monitor_instance(&instance.clone())?;

    // Call orchastrator and send tx T.
    orchastrator.send_tx_instance(instance_id, &tx)?;

    orchastrator.tick()?;

    orchastrator.tick()?;

    orchastrator.tick()?;

    orchastrator.tick()?;

    orchastrator.tick()?;

    Ok(())
}

fn get_mocks() -> (
    MockMonitorApi,
    BitvmxStore,
    Account,
    MockTransactionDispatcherApi,
) {
    let mock_monitor = MockMonitorApi::new();
    let store =
        BitvmxStore::new_with_path(&format!("data/tests/{}", generate_random_string())).unwrap();
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

fn get_mock_data() -> (u32, BitvmxInstance<TransactionPartialInfo>, Transaction) {
    let tx = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![],
        output: vec![],
    };

    let tx_info = TransactionPartialInfo {
        tx_id: tx.compute_txid(),
        owner_operator_id: 1,
    };

    let instance_id = 1;

    let instance = BitvmxInstance::<TransactionPartialInfo> {
        instance_id,
        txs: vec![tx_info],
        funding_tx: FundingTx {
            tx_id: tx.compute_txid(),
            utxo_index: 1,
            utxo_output: TxOut {
                value: Amount::default(),
                script_pubkey: ScriptBuf::default(),
            },
        },
    };
    (instance_id, instance, tx)
}

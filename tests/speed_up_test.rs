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

/*
    Test Summary: speed_up_tx

    This test verifies the orchestrator's ability to monitor a Bitcoin transaction within an instance, dispatch it for
    mining, and attempt a "speed-up" if the transaction remains unmined. The process involves sequentially dispatching
    and tracking the transaction status with each tick. If unmined, the orchestrator initiates a speed-up and continues
    monitoring. On the fourth tick, the transaction is confirmed (mined), and a final tick simulates additional block
    confirmations, marking the transaction as finalized. The test includes acknowledgment of status updates received
    from the monitor at each step.
*/

#[test]
fn speed_up_tx() -> Result<(), anyhow::Error> {
    // Setup mocks for monitor, dispatcher, account, and storage to simulate the environment.
    let (mut mock_monitor, store, account, mut mock_dispatcher) = get_mocks();

    // Setup a mock instance containing a single transaction, marked for dispatch and monitoring.
    let (instance_id, instance, tx) = get_mock_data();

    // Indicate the monitor is ready to track the instance.
    mock_monitor.expect_is_ready().returning(|| Ok(true));

    // The dispatcher is expected to attempt sending the transaction `T` once initially.
    mock_dispatcher
        .expect_send()
        .times(1)
        .with(eq(tx.clone()))
        .returning(move |tx_ret| Ok(tx_ret.compute_txid()));

    // Simulate that the monitor's instance news initially has no updates about the transaction.
    mock_monitor
        .expect_get_instance_news()
        .times(2)
        .returning(move || Ok(vec![]));

    // Update: After dispatch, the transaction ID appears in the instance news.
    let tx_id = tx.compute_txid();
    mock_monitor
        .expect_get_instance_news()
        .times(2)
        .returning(move || Ok(vec![(instance_id, vec![tx_id])]));

    // Transaction status initially shows it as unmined (not seen).
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

    // Simulate the transaction status updating as it is seen but still unconfirmed.
    let tx_status_confirmed = TxStatus {
        tx_id: tx.compute_txid(),
        tx_hex: Some("Ox123".to_string()),
        tx_was_seen: true,
        height_tx_seen: Some(4),
        confirmations: 1, // Transaction is confirmed (mined).
    };
    mock_monitor
        .expect_get_instance_tx_status()
        .times(1)
        .with(eq(instance_id), eq(tx_id))
        .returning(move |_, _| Ok(Some(tx_status_confirmed.clone())));

    // Simulate transaction reaching a finalized state after multiple confirmations.
    let tx_status_finalized = TxStatus {
        tx_id: tx.compute_txid(),
        tx_hex: Some("Ox123".to_string()),
        tx_was_seen: true,
        height_tx_seen: Some(4),
        confirmations: 7, // Transaction is finalized.
    };
    mock_monitor
        .expect_get_instance_tx_status()
        .times(1)
        .with(eq(instance_id), eq(tx_id))
        .returning(move |_, _| Ok(Some(tx_status_finalized.clone())));

    // Mock the dispatcher to check if the transaction needs to be sped up. It should decide "yes."
    mock_dispatcher
        .expect_should_speed_up()
        .with(eq(Amount::default()))
        .times(1)
        .returning(|_| Ok(true));

    // Define unique mock transaction IDs for each speed-up attempt.
    let tx_speed_up_id_1 =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200a").unwrap();
    let tx_speed_up_id_2 =
        Txid::from_str("e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200b").unwrap();

    // First speed-up attempt: Create a new speed-up transaction based on original funding transaction.
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

    // Second speed-up attempt: Re-attempt speed-up using the original UTXO data as the transaction is still unmined.
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
        .returning(move |_, _, _, _| Ok((tx_speed_up_id_2, Amount::default())));

    // Configure monitor to begin tracking the instance containing the transaction.
    let instance_data = InstanceData {
        instance_id,
        txs: vec![instance.txs[0].tx_id],
    };
    mock_monitor
        .expect_save_instances_for_tracking()
        .with(eq(vec![instance_data]))
        .returning(|_| Ok(()));

    // Simulate blockchain height changes with each tick, representing block progress.
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
        .returning(|| 10); // Simulates that more than 6 blocks have been mined since.

    // Acknowledge transaction updates twice to notify the monitor of our awareness of changes.
    mock_monitor
        .expect_acknowledge_instance_tx_news()
        .with(eq(instance_id), eq(tx_id))
        .times(2)
        .returning(|_, _| Ok(()));

    // Save both speed-up transactions in the monitor for tracking purposes.
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

    // Initialize the orchestrator with mocks and begin monitoring the instance.
    let mut orchestrator = Orchestrator::new(mock_monitor, store, mock_dispatcher, account)?;
    orchestrator.monitor_instance(&instance.clone())?;

    // Dispatch the transaction through the orchestrator.
    orchestrator.send_tx_instance(instance_id, &tx)?;

    // Simulate ticks to monitor and adjust transaction status with each blockchain height update.
    orchestrator.tick()?; // Dispatch and observe unmined status.
    orchestrator.tick()?; // First speed-up after unconfirmed status persists.
    orchestrator.tick()?; // Second speed-up due to continued unmined status.
    orchestrator.tick()?; // Confirmation observed, transaction is now mined.
    orchestrator.tick()?; // Simulate further blocks, marking the transaction as finalized.

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

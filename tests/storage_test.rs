use std::{path::PathBuf, rc::Rc, str::FromStr};

use bitcoin::{absolute::LockTime, Amount, ScriptBuf, Transaction, TxOut, Txid};
use bitcoin_coordinator::{
    storage::{BitcoinCoordinatorStore, BitcoinCoordinatorStoreApi},
    types::{
        CoordinatedTransaction, FundingTransaction, SpeedUpTx, TransactionDispatch,
        TransactionPartialInfo, TransactionState,
    },
};
use storage_backend::storage::Storage;
use uuid::Uuid;

#[test]
fn instances_store() -> Result<(), anyhow::Error> {
    let tx_id = Txid::from_str(&"e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200b")
        .unwrap();

    let tx_id_2 =
        Txid::from_str(&"3a3f8d147abf0b9b9d25b07de7a16a4db96bda3e474ceab4c4f9e8e107d5b02f")
            .unwrap();

    let storage = Rc::new(Storage::new_with_path(&PathBuf::from("test_output/test1"))?);

    let bitvmx_store = BitcoinCoordinatorStore::new(storage)?;

    let tx1_summary = TransactionPartialInfo { tx_id: tx_id };

    let tx2_summary = TransactionPartialInfo { tx_id: tx_id_2 };

    let instance = TransactionDispatch::<TransactionPartialInfo> {
        id: Uuid::from_u128(1),
        txs: vec![tx1_summary, tx2_summary],
        funding_tx: Some(FundingTransaction {
            tx_id,
            utxo_index: 1,
            utxo_output: TxOut {
                value: Amount::default(),
                script_pubkey: ScriptBuf::default(),
            },
        }),
    };

    //add instance
    bitvmx_store.coordinate(&instance)?;

    //get instances
    let instances = bitvmx_store.get_instances()?;
    assert_eq!(instances.len(), 1);
    let instace_txs = bitvmx_store.get_instance(instances[0])?;
    assert_eq!(instace_txs.len(), 2);

    //get instance by id
    let instance = bitvmx_store.get_instance(Uuid::from_u128(1))?;
    assert_eq!(instance.len(), 2);

    //remove instance
    bitvmx_store.remove_coordinator(Uuid::from_u128(1))?;
    let instances = bitvmx_store.get_instances()?;
    assert_eq!(instances.len(), 0);

    // get instance by id
    let instance_txs = bitvmx_store.get_instance(Uuid::from_u128(1))?;
    assert_eq!(instance_txs.len(), 0);

    Ok(())
}

#[test]
fn in_progress_tx_store() -> Result<(), anyhow::Error> {
    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(
        "test_output/in_progress_tx_store",
    ))?);
    let store = BitcoinCoordinatorStore::new(storage)?;

    let instance_id = Uuid::from_u128(1);
    let tx_1 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195600).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx_id_1 = tx_1.compute_txid();

    let tx_2 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195601).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx_id_2 = tx_2.compute_txid();

    let tx_instance_summary_1 = TransactionPartialInfo { tx_id: tx_id_1 };

    let tx_instance_summary_2 = TransactionPartialInfo { tx_id: tx_id_2 };

    let block_height = 2;

    let instance = TransactionDispatch::<TransactionPartialInfo> {
        id: instance_id,
        txs: vec![tx_instance_summary_1, tx_instance_summary_2],
        funding_tx: Some(FundingTransaction {
            tx_id: tx_id_1,
            utxo_index: 1,
            utxo_output: TxOut {
                value: Amount::default(),
                script_pubkey: ScriptBuf::default(),
            },
        }),
    };

    // Add instance for the first time.
    store.coordinate(&instance)?;

    // Move instances to in progress.
    store.update_instance_tx_as_sent(instance_id, &tx_id_1, block_height)?;
    store.update_instance_tx_as_sent(instance_id, &tx_id_2, block_height)?;

    //get in progress tx by id
    let instance_txs = store.get_tx(TransactionState::Sent)?;
    assert_eq!(instance_txs.len(), 1);
    let (instance_id, txs) = &instance_txs[0];
    assert_eq!(instance_id, &Uuid::from_u128(1));
    assert_eq!(txs.len(), 2);

    Ok(())
}

#[test]
fn speed_up_txs_test() -> Result<(), anyhow::Error> {
    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(
        "test_output/speed_up_txs_test",
    ))?);
    let bitvmx_store = BitcoinCoordinatorStore::new(storage)?;

    let instance_id = Uuid::from_u128(1);
    // Remove the instance 1, as a mather of cleaning the database.
    let _ = bitvmx_store.remove_coordinator(instance_id);

    let block_height = 2;
    let fee_rate = Amount::from_sat(1000);

    let tx_1 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195600).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx_id_1 = tx_1.compute_txid();

    let speed_up_tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195601).unwrap(),
        input: vec![],
        output: vec![],
    };

    let speed_up_tx_id = speed_up_tx.compute_txid();

    let speed_up_tx = SpeedUpTx {
        tx_id: speed_up_tx_id,
        deliver_block_height: block_height,
        deliver_fee_rate: fee_rate,
        child_tx_id: tx_id_1,
        utxo_index: 1,
        utxo_output: TxOut {
            value: Amount::default(),
            script_pubkey: ScriptBuf::default(),
        },
    };

    let tx_instance_summary_1 = TransactionPartialInfo { tx_id: tx_id_1 };

    let instance = TransactionDispatch::<TransactionPartialInfo> {
        id: instance_id,
        txs: vec![tx_instance_summary_1],
        funding_tx: Some(FundingTransaction {
            tx_id: tx_id_1,
            utxo_index: 1,
            utxo_output: TxOut {
                value: Amount::default(),
                script_pubkey: ScriptBuf::default(),
            },
        }),
    };

    // Add the instance
    bitvmx_store.coordinate(&instance)?;

    // Add the speed up transaction
    bitvmx_store.add_speed_up_tx(instance_id, &speed_up_tx)?;

    // Retrieve the speed up transactions associated with the given instance_id and tx_id_1
    let speed_up_tx_to_validate = bitvmx_store.get_speed_up_txs_for_child(instance_id, &tx_id_1)?;

    // Assert that the retrieved transactions match the expected speed_up_instance
    assert_eq!(speed_up_tx_to_validate, vec![speed_up_tx.clone()]);

    // Retrieve the speed up transactions associated with the given instance_id and tx_id_1
    let speed_up_tx_to_validate = bitvmx_store.get_speed_up_tx(instance_id, &speed_up_tx.tx_id)?;

    // Assert that the retrieved transactions match the expected speed_up_instance
    assert_eq!(speed_up_tx_to_validate, Some(speed_up_tx));

    Ok(())
}

#[test]
fn update_status() -> Result<(), anyhow::Error> {
    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(
        "test_output/update_status",
    ))?);
    let bitvmx_store = BitcoinCoordinatorStore::new(storage)?;

    let instance_id = Uuid::from_u128(1);
    // Remove the instance 1, as a mather of cleaning the database.
    let _ = bitvmx_store.remove_coordinator(instance_id);

    let tx_1 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195600).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx_id_1 = tx_1.compute_txid();

    let tx_instance_summary_1 = TransactionPartialInfo { tx_id: tx_id_1 };

    let instance = TransactionDispatch::<TransactionPartialInfo> {
        id: instance_id,
        txs: vec![tx_instance_summary_1],
        funding_tx: Some(FundingTransaction {
            tx_id: tx_id_1,
            utxo_index: 1,
            utxo_output: TxOut {
                value: Amount::default(),
                script_pubkey: ScriptBuf::default(),
            },
        }),
    };

    let instance_txs = bitvmx_store.get_tx(TransactionState::New)?;
    assert_eq!(instance_txs.len(), 0);

    bitvmx_store.coordinate(&instance)?;

    let transaction_info = CoordinatedTransaction {
        tx_id: tx_id_1,
        deliver_block_height: None,
        tx: None,
        state: TransactionState::New,
    };

    let instance_txs = bitvmx_store.get_tx(TransactionState::New)?;
    assert_eq!(instance_txs.len(), 1);
    assert_eq!(instance_txs[0].1, vec![transaction_info.clone()]);

    //Get instances by other status should be 0
    let instance_in_progress = bitvmx_store.get_tx(TransactionState::Sent)?;
    let instance_pending = bitvmx_store.get_tx(TransactionState::ReadyToSend)?;
    let instance_completed = bitvmx_store.get_tx(TransactionState::Confirmed)?;
    assert_eq!(instance_in_progress.len(), 0);
    assert_eq!(instance_pending.len(), 0);
    assert_eq!(instance_completed.len(), 0);

    // Move transaction to in inprogress.
    bitvmx_store.update_instance_tx_status(instance.id, &tx_id_1, TransactionState::Sent)?;

    let instance_txs = bitvmx_store.get_tx(TransactionState::New)?;
    assert_eq!(instance_txs.len(), 0);
    let instance_txs = bitvmx_store.get_tx(TransactionState::Sent)?;
    assert_eq!(instance_txs.len(), 1);

    bitvmx_store.update_instance_tx_status(instance.id, &tx_id_1, TransactionState::ReadyToSend)?;

    let instance_txs = bitvmx_store.get_tx(TransactionState::Sent)?;
    assert_eq!(instance_txs.len(), 0);

    let instance_txs = bitvmx_store.get_tx(TransactionState::ReadyToSend)?;
    assert_eq!(instance_txs.len(), 1);

    bitvmx_store.update_instance_tx_status(instance.id, &tx_id_1, TransactionState::Confirmed)?;

    let instance_txs = bitvmx_store.get_tx(TransactionState::ReadyToSend)?;
    assert_eq!(instance_txs.len(), 0);

    let instance_txs = bitvmx_store.get_tx(TransactionState::Confirmed)?;
    assert_eq!(instance_txs.len(), 1);

    Ok(())
}

#[test]
fn funding_tests() -> Result<(), anyhow::Error> {
    let storage = Rc::new(Storage::new_with_path(&PathBuf::from(
        "test_output/funding_tests",
    ))?);
    let bitvmx_store = BitcoinCoordinatorStore::new(storage)?;

    let instance_id = Uuid::from_u128(1);
    // Remove the instance 1, as a mather of cleaning the database.
    let _ = bitvmx_store.remove_coordinator(instance_id);

    let tx_1 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195600).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx_id_1 = tx_1.compute_txid();

    let tx_2 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195601).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx_id_2 = tx_2.compute_txid();

    let tx_instance_summary_1 = TransactionPartialInfo { tx_id: tx_id_1 };

    let funding_tx = FundingTransaction {
        tx_id: tx_id_1,
        utxo_index: 1,
        utxo_output: TxOut {
            value: Amount::default(),
            script_pubkey: ScriptBuf::default(),
        },
    };

    let funding_tx_2 = FundingTransaction {
        tx_id: tx_id_2,
        utxo_index: 3,
        utxo_output: TxOut {
            value: Amount::default(),
            script_pubkey: ScriptBuf::default(),
        },
    };

    let instance = TransactionDispatch::<TransactionPartialInfo> {
        id: instance_id,
        txs: vec![tx_instance_summary_1],
        funding_tx: Some(funding_tx.clone()),
    };

    //Add instance 1 with funding tx and check if funding tx exists.
    bitvmx_store.coordinate(&instance)?;
    let funding_tx_to_validate = bitvmx_store.get_funding_tx(instance_id)?;
    assert_eq!(funding_tx_to_validate.unwrap(), funding_tx);

    //Add new funding tx, then ask for funding tx. should return the new funding tx.
    bitvmx_store.fund_for_speedup(instance_id, &funding_tx_2)?;
    let funding_tx_to_validate = bitvmx_store.get_funding_tx(instance_id)?;
    assert_eq!(funding_tx_to_validate.unwrap(), funding_tx_2);

    //Remove the last funding tx and check if the first funding tx is retrieved
    bitvmx_store.remove_funding_tx(instance_id, &tx_id_2)?;
    let funding_tx_to_validate = bitvmx_store.get_funding_tx(instance_id)?;
    assert_eq!(funding_tx_to_validate.unwrap(), funding_tx);

    // Remove the first funding transaction and verify it's no longer present
    bitvmx_store.remove_funding_tx(instance_id, &tx_id_1)?;
    let funding_tx_to_validate = bitvmx_store.get_funding_tx(instance_id)?;
    assert_eq!(funding_tx_to_validate, None);

    Ok(())
}

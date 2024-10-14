use std::str::FromStr;

use bitcoin::{absolute::LockTime, Amount, ScriptBuf, Transaction, TxOut, Txid};
use bitvmx_unstable::{
    storage::{BitvmxStore, InProgressApi, InstanceApi},
    types::{BitvmxInstance, DeliverData, FundingTx, TransactionInstance},
};

#[test]
fn instances_store() -> Result<(), anyhow::Error> {
    let tx_id = Txid::from_str(&"e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200b")
        .unwrap();

    let tx_id_2 =
        Txid::from_str(&"3a3f8d147abf0b9b9d25b07de7a16a4db96bda3e474ceab4c4f9e8e107d5b02f")
            .unwrap();

    let bitvmx_store = BitvmxStore::new_with_path("test_output/test1")?;

    let tx1 = TransactionInstance {
        tx: None,
        tx_id: tx_id,
        owner_operator_id: 1,
        deliver_data: None,
        speed_up_data: None,
    };

    let tx2 = TransactionInstance {
        tx: None,
        tx_id: tx_id_2,
        owner_operator_id: 2,
        deliver_data: None,
        speed_up_data: None,
    };

    let instance = BitvmxInstance::<TransactionInstance> {
        instance_id: 1,
        txs: vec![tx1, tx2],
        funding_tx: FundingTx {
            tx_id,
            utxo_index: 1,
            utxo_output: TxOut {
                value: Amount::default(),
                script_pubkey: ScriptBuf::default(),
            },
        },
    };

    //add instance
    bitvmx_store.add_instance(&instance)?;

    //get instances
    let instances = bitvmx_store.get_instances()?;
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].txs.len(), 2);

    //get instance by id
    let instance = bitvmx_store.get_instance(1)?;
    assert_eq!(instance.unwrap().txs.len(), 2);

    //remove instance
    bitvmx_store.remove_instance(1)?;
    let instances = bitvmx_store.get_instances()?;
    assert_eq!(instances.len(), 0);

    // get instance by id
    let instance = bitvmx_store.get_instance(1)?;
    assert!(instance.is_none());

    Ok(())
}

#[test]
fn in_progress_tx_store() -> Result<(), anyhow::Error> {
    let bitvmx_store = BitvmxStore::new_with_path("test_output/test2")?;

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

    let tx_instance_1 = TransactionInstance {
        tx: Some(tx_1.clone()),
        tx_id: tx_id_1,
        owner_operator_id: 0,
        deliver_data: None,
        speed_up_data: None,
    };

    let tx_instance_2 = TransactionInstance {
        tx: Some(tx_2),
        tx_id: tx_id_2,
        owner_operator_id: 0,
        deliver_data: None,
        speed_up_data: None,
    };

    let block_height = 2;
    let fee_rate = Amount::from_sat(1000);

    let instance = BitvmxInstance::<TransactionInstance> {
        instance_id: 1,
        txs: vec![tx_instance_1, tx_instance_2],
        funding_tx: FundingTx {
            tx_id: tx_id_1,
            utxo_index: 1,
            utxo_output: TxOut {
                value: Amount::default(),
                script_pubkey: ScriptBuf::default(),
            },
        },
    };

    // Add instance for the first time.
    bitvmx_store.add_instance(&instance)?;

    // Move instances to in progress.
    bitvmx_store.add_in_progress_instance_tx(1, &tx_id_1, Amount::default(), block_height)?;
    bitvmx_store.add_in_progress_instance_tx(1, &tx_id_2, Amount::default(), block_height)?;

    //get in progress tx by id
    let instance = bitvmx_store.get_in_progress_txs(1, &tx_id_1)?;
    assert!(instance.is_some());

    //get in progress tx by id
    let instance = bitvmx_store.get_in_progress_txs(1, &tx_id_2)?;
    assert!(instance.is_some());

    // Remove in progress tx
    bitvmx_store.remove_in_progress_instance_tx(1, &tx_id_1)?;
    let instance = bitvmx_store.get_in_progress_txs(1, &tx_id_1)?;
    assert!(instance.is_none());

    // Remove in progress tx2
    bitvmx_store.remove_in_progress_instance_tx(2, &tx_id_2)?;
    let instance = bitvmx_store.get_in_progress_txs(2, &tx_id_2)?;
    assert!(instance.is_none());

    //Add the instance again :

    bitvmx_store.add_in_progress_instance_tx(1, &tx_id_1, fee_rate, block_height)?;

    let check_instance = TransactionInstance {
        tx: Some(tx_1.clone()),
        tx_id: tx_id_1,
        owner_operator_id: 0,
        deliver_data: Some(DeliverData {
            fee_rate,
            block_height,
        }),
        speed_up_data: None,
    };

    let instance = bitvmx_store.get_in_progress_txs(1, &tx_id_1)?;
    assert_eq!(instance.unwrap(), check_instance);

    Ok(())
}

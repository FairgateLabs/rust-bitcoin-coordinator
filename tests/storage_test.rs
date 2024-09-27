use std::str::FromStr;

use bitcoin::{absolute::LockTime, Amount, Transaction, Txid};
use bitvmx_unstable::{
    storage::{BitvmxStore, InProgressApi, InstanceApi},
    types::BitvmxInstance,
};

#[test]
fn instances_store() -> Result<(), anyhow::Error> {
    let tx_id = Txid::from_str(&"e9b7ad71b2f0bbce7165b5ab4a3c1e17e9189f2891650e3b7d644bb7e88f200b")
        .unwrap();

    let tx_id_2 =
        Txid::from_str(&"3a3f8d147abf0b9b9d25b07de7a16a4db96bda3e474ceab4c4f9e8e107d5b02f")
            .unwrap();

    let bitvmx_store = BitvmxStore::new_with_path("test_output")?;

    let instance = BitvmxInstance {
        instance_id: 1,
        txs: vec![
            // FundingTx {
            //     tx_id,
            //     utxo_index: 1,
            //     utxo_output: TxOut {
            //         value: Amount::default(),
            //         script_pubkey: ScriptBuf::new(),
            //     },
            // },
            // FundingTx {
            //     tx_id: tx_id_2,
            //     utxo_index: 0,
            //     utxo_output: TxOut {
            //         value: Amount::default(),
            //         script_pubkey: ScriptBuf::new(),
            //     },
            // },
        ],
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
    let bitvmx_store = BitvmxStore::new_with_path("test_output")?;
    let tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195600).unwrap(),
        input: vec![],
        output: vec![],
    };

    let tx2 = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: LockTime::from_time(1653195601).unwrap(),
        input: vec![],
        output: vec![],
    };

    //add in progress tx 2 times, should rewrite the tx because has the same tx_id
    bitvmx_store.add_in_progress_instance_tx(1, &tx, Amount::default(), 2)?;
    bitvmx_store.add_in_progress_instance_tx(1, &tx2, Amount::default(), 2)?;

    //get in progress txs
    let txs = bitvmx_store.get_in_progress_instances_txs()?;
    assert_eq!(txs.len(), 2);

    //get in progress tx by id
    let instance = bitvmx_store.get_in_progress_instance_tx(1, &tx.compute_txid())?;
    assert!(instance.is_some());

    //get in progress tx by id
    let instance = bitvmx_store.get_in_progress_instance_tx(1, &tx2.compute_txid())?;
    assert!(instance.is_some());

    //remove in progress tx
    bitvmx_store.remove_in_progress_instance_tx(1, &tx.compute_txid())?;
    let txs = bitvmx_store.get_in_progress_instances_txs()?;
    assert_eq!(txs.len(), 1);

    //remove in progress tx2
    bitvmx_store.remove_in_progress_instance_tx(2, &tx2.compute_txid())?;
    let txs = bitvmx_store.get_in_progress_instances_txs()?;
    assert_eq!(txs.len(), 0);

    Ok(())
}

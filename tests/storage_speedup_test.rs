// use bitcoin::{Amount, PublicKey, Txid};
// use bitcoin_coordinator::{
//     errors::BitcoinCoordinatorStoreError,
//     speedup::SpeedupStore,
//     storage::BitcoinCoordinatorStore,
//     types::{CoordinatedSpeedUpTransaction, SpeedupState},
// };
// use protocol_builder::types::Utxo;
// use std::{rc::Rc, str::FromStr};
// use storage_backend::{storage::Storage, storage_config::StorageConfig};
// use utils::{clear_output, generate_random_string};
// mod utils;

// fn create_store() -> BitcoinCoordinatorStore {
//     let storage_config = StorageConfig::new(
//         format!("test_output/speedup/{}", generate_random_string()),
//         None,
//     );
//     let storage = Rc::new(Storage::new(&storage_config).unwrap());
//     BitcoinCoordinatorStore::new(storage).unwrap()
// }

// fn dummy_utxo_with(txid: &Txid, vout: u32, sats: u64) -> Utxo {
//     Utxo::new(
//         *txid,
//         vout,
//         sats,
//         &PublicKey::from_str("032e58afe51f9ed8ad3cc7897f634d881fdbe49a81564629ded8156bebd2ffd1af")
//             .unwrap(),
//     )
// }

// fn dummy_utxo() -> Utxo {
//     dummy_utxo_with(
//         &Txid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
//         0,
//         1000,
//     )
// }

// fn dummy_speedup_tx(
//     txid: &Txid,
//     state: SpeedupState,
//     is_replace: bool,
//     block_height: u32,
//     context: &str,
// ) -> CoordinatedSpeedUpTransaction {
//     CoordinatedSpeedUpTransaction::new(
//         *txid,
//         vec![],
//         1.0,
//         dummy_utxo(),
//         is_replace,
//         block_height,
//         state,
//         context.to_string(),
//     )
// }

// #[test]
// fn test_add_and_get_funding() -> Result<(), anyhow::Error> {
//     let store = create_store();

//     // No funding at first
//     let funding = store.get_funding()?;
//     assert!(funding.is_none());

//     // Add funding
//     let utxo = dummy_utxo();
//     store.add_funding(utxo.clone())?;

//     // Funding should now be present
//     let funding2 = store.get_funding()?;
//     assert!(funding2.is_some());
//     assert_eq!(funding2.unwrap().txid, utxo.txid);

//     clear_output();
//     Ok(())
// }

// #[test]
// fn test_save_and_get_speedup() -> Result<(), anyhow::Error> {
//     let store = create_store();

//     // Save a speedup tx (not finalized, not replace)
//     let txid = Txid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")?;
//     let speedup = dummy_speedup_tx(&txid, SpeedupState::ToDispatch, false, 0, "ctx1");
//     store.save_speedup(speedup.clone())?;

//     // Get by id
//     let fetched = store.get_speedup(&txid)?;
//     assert_eq!(fetched.tx_id, txid);
//     assert_eq!(fetched.state, SpeedupState::ToDispatch);

//     // Get pending speedups
//     let pending = store.get_pending_speedups()?;
//     assert_eq!(pending.len(), 1);
//     assert_eq!(pending[0].tx_id, txid);

//     // can_speedup should be true (funding exists)
//     assert!(store.can_speedup()?);

//     clear_output();
//     Ok(())
// }

// #[test]
// fn test_pending_speedups_break_on_finalized() -> Result<(), anyhow::Error> {
//     let store = create_store();

//     // Add a finalized speedup (should act as checkpoint)
//     let txid1 = Txid::from_str("1111111111111111111111111111111111111111111111111111111111111111")?;
//     let s1 = dummy_speedup_tx(&txid1, SpeedupState::Finalized, false, 0, "ctx1");
//     store.save_speedup(s1.clone())?;

//     // Add a pending speedup (should not be returned, as finalized is checkpoint)
//     let txid2 = Txid::from_str("2222222222222222222222222222222222222222222222222222222222222222")?;
//     let s2 = dummy_speedup_tx(&txid2, SpeedupState::ToDispatch, false, 0, "ctx2");
//     store.save_speedup(s2.clone())?;

//     // Only the last (pending) speedup should be returned, up to the finalized checkpoint
//     let pending = store.get_pending_speedups()?;
//     assert_eq!(pending.len(), 1);
//     assert_eq!(pending[0].tx_id, txid2);

//     clear_output();
//     Ok(())
// }

// #[test]
// fn test_get_funding_with_replace_speedup_confirmed() -> Result<(), anyhow::Error> {
//     let store = create_store();

//     // Add a replace speedup, confirmed
//     let txid = Txid::from_str("3333333333333333333333333333333333333333333333333333333333333333")?;
//     let s = dummy_speedup_tx(&txid, SpeedupState::Confirmed, true, 0, "ctx3");
//     store.save_speedup(s.clone())?;

//     // Funding should be present
//     let funding = store.get_funding()?;
//     assert!(funding.is_some());
//     assert_eq!(funding.unwrap().txid, s.funding.txid);

//     clear_output();
//     Ok(())
// }

// #[test]
// fn test_get_funding_with_replace_speedup_dispatched_and_confirmed_chain() -> Result<(), anyhow::Error> {
//     let store = create_store();

//     // Add a replace speedup, dispatched
//     let txid1 = Txid::from_str("4444444444444444444444444444444444444444444444444444444444444444")?;
//     let s1 = dummy_speedup_tx(&txid1, SpeedupState::Dispatched, true, 0, "ctx4");
//     store.save_speedup(s1.clone())?;

//     // Add a replace speedup, confirmed (should be found by get_funding)
//     let txid2 = Txid::from_str("5555555555555555555555555555555555555555555555555555555555555555")?;
//     let s2 = dummy_speedup_tx(&txid2, SpeedupState::Confirmed, true, 0, "ctx5");
//     store.save_speedup(s2.clone())?;

//     let funding = store.get_funding()?;
//     assert!(funding.is_some());
//     assert_eq!(funding.unwrap().txid, s2.funding.txid);

//     clear_output();
//     Ok(())
// }

// #[test]
// fn test_get_funding_with_replace_speedup_dispatched_and_no_confirmed() -> Result<(), anyhow::Error> {
//     let store = create_store();

//     // Add a replace speedup, dispatched
//     let txid1 = Txid::from_str("6666666666666666666666666666666666666666666666666666666666666666")?;
//     let s1 = dummy_speedup_tx(&txid1, SpeedupState::Dispatched, true, 0, "ctx6");
//     store.save_speedup(s1.clone())?;

//     // Add a replace speedup, dispatched (no confirmed in chain)
//     let txid2 = Txid::from_str("7777777777777777777777777777777777777777777777777777777777777777")?;
//     let s2 = dummy_speedup_tx(&txid2, SpeedupState::Dispatched, true, 0, "ctx7");
//     store.save_speedup(s2.clone())?;

//     let funding = store.get_funding()?;
//     assert!(funding.is_none());

//     clear_output();
//     Ok(())
// }

// #[test]
// fn test_can_speedup_none() -> Result<(), anyhow::Error> {
//     let store = create_store();
//     assert!(!store.can_speedup()?);
//     clear_output();
//     Ok(())
// }

// #[test]
// fn test_update_speedup_state_and_remove_from_pending() -> Result<(), anyhow::Error> {
//     let store = create_store();

//     // Add a speedup tx
//     let txid = Txid::from_str("8888888888888888888888888888888888888888888888888888888888888888")?;
//     let s = dummy_speedup_tx(&txid, SpeedupState::ToDispatch, false, 0, "ctx8");
//     store.save_speedup(s.clone())?;

//     // Update to Finalized (should remove from pending list)
//     store.update_speedup_state(txid, SpeedupState::Finalized)?;

//     // Should not be in pending speedups
//     let pending = store.get_pending_speedups()?;
//     assert!(pending.is_empty());

//     // Should still be able to fetch by id, and state should be Finalized
//     let fetched = store.get_speedup(&txid)?;
//     assert_eq!(fetched.state, SpeedupState::Finalized);

//     clear_output();
//     Ok(())
// }

// #[test]
// fn test_update_speedup_state_not_found() -> Result<(), anyhow::Error> {
//     let store = create_store();
//     let txid = Txid::from_str("9999999999999999999999999999999999999999999999999999999999999999")?;
//     let res = store.update_speedup_state(txid, SpeedupState::Finalized);
//     assert!(matches!(
//         res,
//         Err(BitcoinCoordinatorStoreError::SpeedupNotFound)
//     ));
//     clear_output();
//     Ok(())
// }

// #[test]
// fn test_get_speedup_not_found() -> Result<(), anyhow::Error> {
//     let store = create_store();
//     let txid = Txid::from_str("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")?;
//     let res = store.get_speedup(&txid);
//     assert!(matches!(
//         res,
//         Err(BitcoinCoordinatorStoreError::SpeedupNotFound)
//     ));
//     clear_output();
//     Ok(())
// }

// #[test]
// fn test_save_speedup_overwrites() -> Result<(), anyhow::Error> {
//     let store = create_store();
//     let txid = Txid::from_str("abababababababababababababababababababababababababababababababab")?;
//     let s1 = dummy_speedup_tx(&txid, SpeedupState::ToDispatch, false, 0, "ctx9");
//     let mut s2 = s1.clone();
//     s2.state = SpeedupState::Dispatched;
//     s2.block_height = 999;

//     store.save_speedup(s1.clone())?;
//     let fetched = store.get_speedup(&txid)?;
//     assert_eq!(fetched.state, SpeedupState::ToDispatch);

//     // Overwrite
//     store.save_speedup(s2.clone())?;
//     let fetched2 = store.get_speedup(&txid)?;
//     assert_eq!(fetched2.state, SpeedupState::Dispatched);
//     assert_eq!(fetched2.block_height, 999);

//     clear_output();
//     Ok(())
// }

// #[test]
// fn test_get_speedup_to_replace_always_none() -> Result<(), anyhow::Error> {
//     let store = create_store();
//     let res = store.get_speedup_to_replace()?;
//     assert!(res.is_none());
//     clear_output();
//     Ok(())
// }

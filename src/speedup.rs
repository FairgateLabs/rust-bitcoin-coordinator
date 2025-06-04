pub fn speed_up(tx: Transaction, funding_tx: FundingTransaction) {
    let tx_id = tx.compute_txid();
    let funding_tx_id = funding_tx.compute_txid();

    let tx_info = CoordinatedTransaction::new(tx, TransactionDispatchState::PendingDispatch, None);
}

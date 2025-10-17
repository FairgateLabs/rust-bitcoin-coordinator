# Bitcoin Coordinator

Bitcoin Coordinator is a library designed to be a central component of the Bitvmx client, focusing on efficient transaction management and monitoring within the Bitcoin network. It not only monitors specific transactions but is also responsible for dispatching transactions, speeding them up when necessary, and maintaining the funding required for these speedups. Speedups can be achieved through Child-Pays-For-Parent (CPFP) or Replace-By-Fee (RBF) methods, depending on the timing and urgency.

## ⚠️ Disclaimer

This library is currently under development and may not be fully stable.
It is not production-ready, has not been audited, and future updates may introduce breaking changes without preserving backward compatibility.

## Key Features

- 🕵️ **Transaction Monitoring**: Leverages the `bitvmx-transaction-monitor` module to track and manage Bitcoin transactions effectively.
- 💾 **Data Storage**: Utilizes the `rust-bitvmx-storage-backend` for reliable and persistent data storage.
- 🔑 **Cryptographic Key Management**: Integrates with `bitvmx-key-manager` to handle cryptographic key operations securely and efficiently.

## Methods

The following is a list of all public methods available in the `BitcoinCoordinatorApi` trait:

1. **new_with_paths**: Initializes a new instance of `BitcoinCoordinator` with the provided paths and settings.

2. **is_ready**: Checks if the coordinator is ready to process transactions. Returns true if ready, false otherwise.

3. **monitor**: Registers a type of data to be monitored by the coordinator. The data will be tracked for confirmations and status changes.

4. **dispatch**: Dispatches a transaction to the Bitcoin network. Includes options for speedup and additional context.

5. **cancel**: Cancels the monitor and the dispatch of a type of data, removing it from the coordinator's store.

6. **add_funding**: Registers funding information for potential transaction speed-ups, allowing the creation of child pays for parents transactions.

7. **get_transaction**: Retrieves the status of a specific transaction by its transaction ID.

8. **get_news**: Retrieves news about monitored transactions, providing information about transaction confirmations.

9. **ack_news**: Acknowledges that news has been processed, preventing the same news from being returned in subsequent calls to `get_news()`.

## Usage Examples

Below are examples of how to use the methods provided by the `BitcoinCoordinatorApi` trait. 

```rust 
// Create a new coordinator instance with the Bitcoin RPC config, storage, and key manager
let coordinator = BitcoinCoordinator::new_with_paths(
    &config_bitcoin_client,
    storage.clone(),
    key_manager.clone(),
    None,
);

// Synchronize the coordinator with the blockchain (e.g., after startup or new blocks)
coordinator.tick();


// Verify if the coordinator is ready to process transactions
let is_ready = coordinator.is_ready();

// Define context and register transactions to be monitored
let tx_context = "My tx".to_string();
let tx_to_monitor = TypesToMonitor::Transactions(vec![txid1], tx_context.clone());
coordinator.monitor(tx_to_monitor);

// Dispatch a transaction with optional CPFP speedup data and a context string
let speedup_data = Some(SpeedupData::new(speedup_utxo));
coordinator.dispatch(transaction, speedup_data, tx_context.clone(), None);

// Provide funding UTXO for future speedup transactions (e.g., CPFP)
let utxo = Utxo::new(txid, vout_index, amount.to_sat(), &public_key);
coordinator.add_funding(utxo);

// Retrieve any available transaction-related news (e.g., confirmations)
let news = coordinator.get_news();

// Acknowledge received news so it won't be reported again
let ack_news = AckNews::Monitor(AckMonitorNews::Transaction(txid));
coordinator.ack_news(ack_news);

// Check the current status of a specific transaction
let tx_status = coordinator.get_transaction(txid);
```
## Development Setup

1. Clone the repository
2. Install dependencies: `cargo build`
3. Run tests: `cargo test -- --ignored`

## Contributing
Contributions are welcome! Please open an issue or submit a pull request on GitHub.

## License

This project is licensed under the MIT License - see [LICENSE](LICENSE) file for details.

---

## 🧩 Part of the BitVMX Ecosystem

This repository is a component of the **BitVMX Ecosystem**, an open platform for disputable computation secured by Bitcoin.  
You can find the index of all BitVMX open-source components at [**FairgateLabs/BitVMX**](https://github.com/FairgateLabs/BitVMX).

---

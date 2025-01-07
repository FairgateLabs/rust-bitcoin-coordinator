# BitVMX Orchestrator
BitVMX Orchestrator is a Rust-based project that serves as a core component for Bitvmx client for transaction management and monitoring. This project integrates several key components:

- **Transaction Dispatcher**: Uses `bitvmx-transaction-dispatcher` for sending and speeding up Bitcoin transactions.
- **Transaction Monitor**: Uses `bitvmx-transaction-monitor` for monitoring Bitvmx instances and Bitcoin transactions
- **Storage**: Integrates with `rust-bitvmx-storage-backend` for persistent data storage.
- **Key Management**: Employs `bitvmx-key-manager` for cryptographic key operations.

## Installation
Clone the repository and initialize the submodules:
```bash
$ git clone git@github.com:FairgateLabs/rust-bitvmx-orchestrator.git
```

### Tests
If you make some changes please run tests to verify everything still working as expected.

```
cargo test
```
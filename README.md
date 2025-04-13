# Bitcoin Coordinator
Bitcoin Coordinator is a Rust-based project that serves as a core component for Bitvmx client for transaction management and monitoring. This project integrates several key components:

- **Transaction Dispatcher**: Uses `bitvmx-transaction-dispatcher` for sending and speeding up Bitcoin transactions.
- **Transaction Monitor**: Uses `bitvmx-transaction-monitor` for monitoring bitcoin transaction
- **Storage**: Integrates with `rust-bitvmx-storage-backend` for persistent data storage.
- **Key Management**: Employs `bitvmx-key-manager` for cryptographic key operations.

## Installation
Clone the repository and initialize the submodules:
```bash
$ git clone git@github.com:FairgateLabs/rust-bitcoin-coordinator.git
```

### Tests
If you make some changes please run tests to verify everything still working as expected.

```
cargo test
```

## Testing Locally

**Pre-requisites:**
1. Install Docker engine
2. Install [ACT](https://nektosact.com/installation/index.html)
3. Get the GitHub token, needed to fetch repositories
4. Remove all commented code in **src/tests/docker_integration_test.rs**

**Run all tests:**
```rust
$cargo test
```

**Run job locally**

Some `act` versions might have issues caching the templates versions and not using the last one and also with some of the authentication tokens, so before locally executing the tests, please do the following:

In project root:

If you're using a Linux Based OS:
```bash
rm -rf ~/.cache/act
```
If you're using windows
```powershell
Remove-Item -Recurse -Force $env:USERPROFILE\.cache\act
```
Then to execute the test use:
```bash
$act --pull -s SSH_PRIVATE_KEY="$(cat ~/.ssh/id_rsa)" -s GITHUB_TOKEN="token" -s REPO_ACCESS_TOKEN="token" -j 'local_test'
```
The use of the `--verbose` flag at the end of the test execution command is not required but is recomended, since it gives the user a more deep info on the total execution log
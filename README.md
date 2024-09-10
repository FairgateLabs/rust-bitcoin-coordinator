# BitVMX unstable
Rust implementation of BitVMX. Unstable package for the actively developed version.

For now, this is just one big test. To execute it, run:

```bash
# start a bitcoin node
$ docker run --rm --name bitcoin-server -it \
  -p 18443:18443 \
  -p 18444:18444 \
  ruimarinho/bitcoin-core:24.0.1 \
  -printtoconsole \
  -regtest=1 \
  -rpcallowip=172.17.0.0/16 \
  -rpcbind=0.0.0.0 \
  -rpcauth='foo:337f951003371b21ba0a964464a1d34a$591adbcccece2e5bc1fdd8426c3aa9441a8a6c5cf0fa9a3ed6f7f53029e76130' \
  -fallbackfee=0.0001 \
  -minrelaytxfee=0.00001 \
  -maxtxfee=10000000 \
  -txindex

# run the test
$ BITVMX_ENV=development cargo run
```

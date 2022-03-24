# Chronik Indexer (NNG version)

## Existing Specifications
Chronik is an indexer that can index the eCash (XEC) and Lotus (XPI) blockchains.

- Indexes:
    - Block by hash and height (+metadata like size, total output, etc.)
    - Transactions by block and txid
    - Transactions by script, chronologically (by block height, then by CTOR), paginated
    - UTXOs by script
    - SLP validity and invalidity reason
- Exposes:
    - HTTP interface behind Protobuf (see [https://github.com/EyeOfPython/chronik-indexer-sample/blob/master/proto/chronik.proto](https://github.com/EyeOfPython/chronik-indexer-sample/blob/master/proto/chronik.proto))
        - `POST /broadcast-tx`
        - `POST /broadcast-txs`
        - `GET /blocks/:start/:end`
        - `GET /block/:hash_or_height`
        - `GET /tx/:txid`
        - `GET /script/:type/:payload/history`
        - `GET /script/:type/:payload/utxos`
        - `POST /validate-utxos`
    - WebSocket interface, subscribing to addresses:
        - `AddedToMempool`
        - `RemovedFromMempool`
        - `Confirmed`
        - `Reorg`

## Build
On a clean Ubuntu 20.04.3 LTS, the following packages would have to be installed:

`sudo apt-get update`

`sudo apt-get install build-essential libssl-dev pkg-config clang cmake`

1. Install [Rust](https://www.rust-lang.org/tools/install)
3. Clone this repository `chronik` into a folder
4. Also clone the `bitcoinsuite` repository into the same folder: https://github.com/LogosFoundation/bitcoinsuite
5. `cd` into `chronik/chronik-exe` and run `cargo build --release` (will take a while). You might need to install some required libraries.
6. Compiled binary will be in `chronik/target/release/chronik-exe`. Copy it to a convenient location.
7. It is recommended to run `cargo clean` in both `bitcoinsuite` and `chronik` afterwards (will delete `chronik-exe` executable), as compilation artifacts can take up a lot of space.

## Building eCash node with NNG
In order to run Chronik on eCash, you need a modified Bitcoin ABC node (which supports the NNG interface).

Currently, this has to be built manually, but binaries will be available for this soon.

For this, you need to clone and build https://github.com/raipay/bitcoin-abc/tree/nng_interface. Make sure to checkout the `nng_interface` branch. Follow the build instructions [there](https://github.com/raipay/bitcoin-abc/tree/nng_interface/doc).

On a clean Ubuntu 20.04.3 LTS, you can follow these instructions:
1. Install these packages: `sudo apt-get install bsdmainutils build-essential libssl-dev libevent-dev lld ninja-build python3 cmake libjemalloc-dev libnng-dev libboost-system-dev libboost-filesystem-dev libboost-test-dev libboost-thread-dev`
2. Install [flatbuffers 2.0.0](https://github.com/google/flatbuffers)
    1. `git clone https://github.com/google/flatbuffers`
    2. `git checkout v2.0.0`
    3. `cmake -GNinja -DCMAKE_BUILD_TYPE=Release`
    4. `ninja`
    5. `sudo ninja install`
3. Clone repo: `git clone https://github.com/raipay/bitcoin-abc`
4. Checkout NNG branch: `git checkout nng_interface`
5. Make build folder: `cd bitcoin-abc && mkdir build && cd build`
6. Generate build files: `cmake -GNinja .. -DBUILD_BITCOIN_QT=OFF -DENABLE_UPNP=OFF -DBUILD_BITCOIN_ZMQ=OFF -DBUILD_BITCOIN_WALLET=OFF`
7. Build node: `ninja`
8. Copy compiled executable somewhere nice, e.g.: `cp build/src/bitcoind /var/chronik/`

## Building Lotus node with NNG
Lotus has the NNG interface in its latest version (2.1.3), therefore you can simply download and unzip that version.

Otherwise, you can follow the same instructions as for the eCash node.

## Setting up eCash or Lotus node for Chronik

**NOTE**: It is advised to have at least 20GB of available disk space to allow Chronik to sync properly (disk usage will grow and shrink during sync time)

1. Add a file named `bitcoin.conf` (for eCash) or `lotus.conf` (for Lotus) to the appropriate datadir (e.g. `~/.bitcoin` for eCash or `~/.lotus` for Lotus) with the following contents:
  ```conf
  # Main
  listen=1
  server=1
  txindex=1
  disablewallet=1
  # RPC
  rpcuser=lotus
  rpcpassword=supersecurepassword
  rpcbind=127.0.0.1
  rpcworkqueue=10000
  rpcthreads=8
  # Chronik
  nngpub=ipc:///path/to/pub.pipe
  nngrpc=ipc:///path/to/rpc.pipe
  nngpubmsg=blkconnected
  nngpubmsg=blkdisconctd
  nngpubmsg=mempooltxadd
  nngpubmsg=mempooltxrem
  ```
**IMPORTANT**: Be sure to set real file paths (prefixed with `ipc://`) for `nngpub` and `nngrpc`; Example: `nngpub=ipc:///var/lib/chronik/pub.pipe`

**IMPORTANT**: Make sure to set a proper, random password.

2. Create new `chronik.conf` in same dir as Chronik binary with the following contents:
  ```toml
  host = "127.0.0.1:7123"
  nng_pub_url = "ipc:///path/to/pub.pipe"
  nng_rpc_url = "ipc:///path/to/rpc.pipe"
  db_path = "/path/to/index.rocksdb"
  cache_script_history = 1000000
  network = "XPI"

  [bitcoind_rpc]
  url = "http://127.0.0.1:10604"
  rpc_user = "lotus"
  rpc_pass = "supersecurepassword"
  ```
**IMPORTANT**: Be sure to set your `nng_pub_url` and `nng_rpc_url` according to your `lotus.conf`

**IMPORTANT**: Be sure to set the port in `bitcoind_rpc.url` according to the node's RPC port; on eCash by default this is 8332, on Lotus 10604

**NOTE**: Set the `network` to either `XPI` (for Lotus) or `XEC` (for eCash)

3. Launch Lotus/eCash node (you can use an existing datadir)
4. Run Chronik (can be done while node is syncing):
  ```
  ./chronik-exe chronik.conf
  ```

If everything is working correctly, you should begin seeing "`Added block ...`" lines scrolling through the terminal window.

In your `chronik.conf` file, feel free to adjust the `host` parameter to your liking. This is the IP address and port that Chronik will bind to for inbound connections.

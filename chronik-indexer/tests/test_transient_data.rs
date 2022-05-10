use std::{ffi::OsString, str::FromStr, sync::Arc};

use bitcoinsuite_bitcoind::instance::{BitcoindChain, BitcoindConf, BitcoindInstance};
use bitcoinsuite_bitcoind_nng::{PubInterface, RpcInterface};
use bitcoinsuite_core::{
    build_lotus_block, build_lotus_coinbase, lotus_txid, BitcoinCode, Hashed, LotusAddress, Net,
    Network, Script, Sha256d, ShaRmd160, TxOutput, LOTUS_PREFIX,
};
use bitcoinsuite_ecc_secp256k1::EccSecp256k1;
use bitcoinsuite_error::Result;
use bitcoinsuite_test_utils::bin_folder;
use bitcoinsuite_test_utils_blockchain::{build_tx, setup_bitcoind_coins};
use chronik_indexer::{run_transient_data_catchup, SlpIndexer};
use chronik_rocksdb::{Db, IndexDb, IndexMemData, ScriptTxsConf, TransientData};
use pretty_assertions::assert_eq;
use tempdir::TempDir;
use tokio::sync::RwLock;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_transient_data() -> Result<()> {
    bitcoinsuite_error::install()?;
    let dir = TempDir::new("slp-indexer-test-mempool")?;
    let pub_url = format!("ipc://{}", dir.path().join("pub.pipe").to_string_lossy());
    let rpc_url = format!("ipc://{}", dir.path().join("rpc.pipe").to_string_lossy());
    let conf = BitcoindConf::from_chain_regtest(
        bin_folder(),
        BitcoindChain::XPI,
        vec![
            OsString::from_str(&format!("-nngpub={}", pub_url))?,
            OsString::from_str("-nngpubmsg=blkconnected")?,
            OsString::from_str("-nngpubmsg=blkdisconctd")?,
            OsString::from_str("-nngpubmsg=mempooltxadd")?,
            OsString::from_str("-nngpubmsg=mempooltxrem")?,
            OsString::from_str(&format!("-nngrpc={}", rpc_url))?,
        ],
    )?;
    let mut instance = BitcoindInstance::setup(conf)?;
    instance.wait_for_ready()?;
    let pub_interface = PubInterface::open(&pub_url)?;
    let rpc_interface = RpcInterface::open(&rpc_url)?;
    let script_txs_conf = ScriptTxsConf { page_size: 7 };
    let db = Db::open(dir.path().join("index.rocksdb"))?;
    let transient_data = TransientData::open(&dir.path().join("transient.rocksdb"))?;
    let db = IndexDb::new(db, transient_data, script_txs_conf);
    let bitcoind = instance.cli();
    let cache = IndexMemData::new(10);
    let mut slp_indexer = SlpIndexer::new(
        db,
        instance.rpc_client().clone(),
        rpc_interface,
        pub_interface,
        cache,
        Network::XPI,
        Arc::new(EccSecp256k1::default()),
    )?;
    bitcoind.cmd_string("setmocktime", &["2000000000"])?;

    let anyone_script = Script::from_slice(&[0x51]);
    let anyone_hash = ShaRmd160::digest(anyone_script.bytecode().clone());
    let anyone_address = LotusAddress::new(LOTUS_PREFIX, Net::Regtest, Script::p2sh(&anyone_hash));

    let mut utxos = setup_bitcoind_coins(
        bitcoind,
        Network::XPI,
        10,
        anyone_address.as_str(),
        &anyone_address.script().hex(),
    )?;

    while !slp_indexer.catchup_step().await? {}
    slp_indexer.leave_catchup()?;

    // This does nothing (yet)
    let slp_indexer = RwLock::new(slp_indexer);
    run_transient_data_catchup(&slp_indexer).await?;
    let mut slp_indexer = slp_indexer.into_inner();

    {
        // Transient data caught up already
        let transient_data = slp_indexer.db().transient_data();
        for i in 0..=110 {
            assert_eq!(
                transient_data.read_block(i)?,
                Some(chronik_rocksdb::proto::TransientBlockData { tx_data: vec![] }),
                "block height = {}",
                i,
            );
        }
        assert_eq!(transient_data.read_block(111)?, None);

        // Clear transient data caught up already
        for i in 0..=110 {
            slp_indexer.db().transient_data_writer().delete_block(i)?;
        }

        assert_eq!(transient_data.read_block(0)?, None);
    }

    let txid = {
        // Add block with 1 tx
        let (outpoint, value) = utxos.pop().unwrap();
        let tx = build_tx(
            outpoint,
            &anyone_script,
            vec![TxOutput {
                value: value - 10_000,
                script: Script::opreturn(&[&[0; 100]]),
            }],
        );
        let txid_hex = bitcoind.cmd_string("sendrawtransaction", &[&tx.ser().hex()])?;
        let txid = Sha256d::from_hex_be(&txid_hex)?;
        slp_indexer.process_next_msg()?;
        bitcoind.cmd_json("generatetoaddress", &["1", anyone_address.as_str()])?;
        slp_indexer.process_next_msg()?;

        // Now, no new transient data will be indexed
        let transient_data = slp_indexer.db().transient_data();
        for i in 0..=111 {
            assert_eq!(transient_data.read_block(i)?, None);
        }

        txid
    };

    // Runs in background, continuously catching up until it's close to the tip
    let slp_indexer = RwLock::new(slp_indexer);
    run_transient_data_catchup(&slp_indexer).await?;
    let mut slp_indexer = slp_indexer.into_inner();

    {
        let transient_data = slp_indexer.db().transient_data();
        // run_transient_data_catchup stops indexing 10 blocks before tip
        for i in 0..=101 {
            assert_eq!(
                transient_data.read_block(i)?,
                Some(chronik_rocksdb::proto::TransientBlockData { tx_data: vec![] }),
                "block height = {}",
                i,
            );
        }
        assert_eq!(transient_data.read_block(102)?, None);

        // Next block will catch transient data up to tip
        bitcoind.cmd_json("generatetoaddress", &["1", anyone_address.as_str()])?;
        slp_indexer.process_next_msg()?;

        let transient_data = slp_indexer.db().transient_data();
        for i in 0..=110 {
            assert_eq!(
                transient_data.read_block(i)?,
                Some(chronik_rocksdb::proto::TransientBlockData { tx_data: vec![] }),
                "block height = {}",
                i,
            );
        }
        assert_eq!(
            transient_data.read_block(111)?,
            Some(chronik_rocksdb::proto::TransientBlockData {
                tx_data: vec![chronik_rocksdb::proto::TransientTxData {
                    txid_hash: seahash::hash(txid.as_slice()),
                    time_first_seen: 2_000_000_000,
                }],
            }),
        );
        assert_eq!(
            transient_data.read_block(112)?,
            Some(chronik_rocksdb::proto::TransientBlockData { tx_data: vec![] }),
        );
    }

    // Mine block with some first-seen timestamps known and some unknown
    let txs = (0u8..=5)
        .into_iter()
        .map(|i| {
            let (outpoint, value) = utxos.pop().unwrap();
            build_tx(
                outpoint,
                &anyone_script,
                vec![TxOutput {
                    value: value - 10_000,
                    script: Script::opreturn(&[&[i; 100]]),
                }],
            )
        })
        .collect::<Vec<_>>();

    // broadcast only txs 1, 4 and 5.
    bitcoind.cmd_string("setmocktime", &["2000000010"])?;
    bitcoind.cmd_string("sendrawtransaction", &[&txs[1].ser().hex()])?;
    slp_indexer.process_next_msg()?;
    bitcoind.cmd_string("setmocktime", &["2000000040"])?;
    bitcoind.cmd_string("sendrawtransaction", &[&txs[4].ser().hex()])?;
    slp_indexer.process_next_msg()?;
    bitcoind.cmd_string("setmocktime", &["2000000050"])?;
    bitcoind.cmd_string("sendrawtransaction", &[&txs[5].ser().hex()])?;
    slp_indexer.process_next_msg()?;

    let txids = txs.iter().map(lotus_txid).collect::<Vec<_>>();

    // mine all 6 txs
    let prev_block = Sha256d::from_hex_be(&bitcoind.cmd_string("getbestblockhash", &[])?)?;
    let height = 113;
    let mut lotus_block = build_lotus_block(
        prev_block,
        2000000150,
        height,
        build_lotus_coinbase(height, anyone_address.script().clone()).hashed(),
        txs.into_iter().map(|tx| tx.hashed()).collect(),
        Sha256d::new([0; 32]),
        vec![],
    );
    lotus_block.prepare();

    let submit_result = bitcoind.cmd_string("submitblock", &[&lotus_block.ser().hex()])?;
    assert_eq!(submit_result, "");
    slp_indexer.process_next_msg()?;
    std::mem::drop(slp_indexer);

    // re-index from genesis, and re-uses the transient data
    let cache = IndexMemData::new(10);
    let pub_interface = PubInterface::open(&pub_url)?;
    let rpc_interface = RpcInterface::open(&rpc_url)?;
    let script_txs_conf = ScriptTxsConf { page_size: 7 };
    let reindex_db = Db::open(dir.path().join("reindex.rocksdb"))?;
    let transient_data = TransientData::open(&dir.path().join("transient.rocksdb"))?;
    let db = IndexDb::new(reindex_db, transient_data, script_txs_conf);
    let mut slp_indexer = SlpIndexer::new(
        db,
        instance.rpc_client().clone(),
        rpc_interface,
        pub_interface,
        cache,
        Network::XPI,
        Arc::new(EccSecp256k1::default()),
    )?;

    while !slp_indexer.catchup_step().await? {}
    slp_indexer.leave_catchup()?;

    let tx_reader = slp_indexer.txs();
    let tfs_by_txid = |txid: &Sha256d| {
        tx_reader
            .rich_tx_by_txid(txid)
            .unwrap()
            .expect("No such tx")
            .time_first_seen
    };
    assert_eq!(tfs_by_txid(&txid), 2000000000);
    assert_eq!(tfs_by_txid(&txids[0]), 0);
    assert_eq!(tfs_by_txid(&txids[1]), 2000000010);
    assert_eq!(tfs_by_txid(&txids[2]), 0);
    assert_eq!(tfs_by_txid(&txids[3]), 0);
    assert_eq!(tfs_by_txid(&txids[4]), 2000000040);
    assert_eq!(tfs_by_txid(&txids[5]), 2000000050);

    instance.cleanup()?;
    Ok(())
}

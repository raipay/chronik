use std::{ffi::OsString, str::FromStr, time::Duration};

use bitcoinsuite_bitcoind::{
    cli::BitcoinCli,
    instance::{BitcoindChain, BitcoindConf, BitcoindInstance},
};
use bitcoinsuite_bitcoind_nng::{PubInterface, RpcInterface};
use bitcoinsuite_core::{
    build_lotus_block, build_lotus_coinbase, AddressType, BitcoinCode, Bytes, CashAddress, Hashed,
    Script, Sha256d, ShaRmd160, BCHREG,
};
use bitcoinsuite_error::Result;
use bitcoinsuite_test_utils::bin_folder;
use chronik_indexer::SlpIndexer;
use chronik_rocksdb::{BlockTx, Db, IndexDb, TxEntry};
use pretty_assertions::assert_eq;
use tempdir::TempDir;

#[test]
fn test_non_slp() -> Result<()> {
    let dir = TempDir::new("slp-indexer-test")?;
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
    let db = Db::open(dir.path().join("index.rocksdb"))?;
    let db = IndexDb::new(db);
    let bitcoin_cli = instance.cli();
    let slp_indexer = SlpIndexer::new(db, bitcoin_cli.clone(), rpc_interface, pub_interface)?;
    test_index_genesis(&slp_indexer, bitcoin_cli)?;
    test_get_out_of_ibd(&slp_indexer, bitcoin_cli)?;
    test_reorg_empty(&slp_indexer, bitcoin_cli)?;
    instance.cleanup()?;
    Ok(())
}

fn get_coinbase_txid(bitcoind: &BitcoinCli, block_hash: &Sha256d) -> Result<Sha256d> {
    let node_block = bitcoind.cmd_json("getblock", &[&block_hash.to_hex_be()])?;
    let txid = Sha256d::from_hex_be(node_block["tx"][0].as_str().unwrap())?;
    Ok(txid)
}

fn check_tx_indexed(
    slp_indexer: &SlpIndexer,
    txid: &Sha256d,
    block_height: i32,
    data_pos: u32,
    tx_size: u32,
) -> Result<()> {
    let db_txs = slp_indexer.txs()?;
    assert_eq!(
        db_txs.by_txid(txid)?,
        Some(BlockTx {
            block_height,
            entry: TxEntry {
                txid: txid.clone(),
                data_pos,
                tx_size,
            }
        })
    );
    Ok(())
}

fn test_index_genesis(slp_indexer: &SlpIndexer, bitcoind: &BitcoinCli) -> Result<()> {
    let info = bitcoind.cmd_json("getblockchaininfo", &[])?;
    assert!(info["initialblockdownload"].as_bool().unwrap());
    assert_eq!(info["blocks"], 0);
    let db_blocks = slp_indexer.blocks()?;
    assert_eq!(db_blocks.height()?, -1);
    assert_eq!(db_blocks.tip()?, None);
    // index genesis block
    assert!(!slp_indexer.catchup_step()?);
    assert_eq!(db_blocks.height()?, 0);
    let tip = db_blocks.tip()?.unwrap();
    assert_eq!(tip.hash.to_hex_be(), info["bestblockhash"]);
    check_tx_indexed(
        slp_indexer,
        &get_coinbase_txid(bitcoind, &tip.hash)?,
        0,
        170,
        217,
    )?;
    Ok(())
}

fn test_get_out_of_ibd(slp_indexer: &SlpIndexer, bitcoind: &BitcoinCli) -> Result<()> {
    let prev_info = bitcoind.cmd_json("getblockchaininfo", &[])?;
    // generate block delayed
    let gen_handle = std::thread::spawn({
        let bitcoind = bitcoind.clone();
        let address = CashAddress::from_hash(BCHREG, AddressType::P2SH, ShaRmd160::new([0; 20]));
        move || {
            std::thread::sleep(Duration::from_millis(5));
            bitcoind
                .cmd_json("generatetoaddress", &["1", address.as_str()])
                .unwrap();
        }
    });
    // will wait for IBD and then index
    assert!(!slp_indexer.catchup_step()?);
    gen_handle.join().unwrap();
    let cur_info = bitcoind.cmd_json("getblockchaininfo", &[])?;
    let db_blocks = slp_indexer.blocks()?;
    let tip = db_blocks.tip()?.unwrap();
    assert_eq!(tip.prev_hash.to_hex_be(), prev_info["bestblockhash"]);
    assert_eq!(tip.hash.to_hex_be(), cur_info["bestblockhash"]);
    assert_eq!(prev_info["initialblockdownload"], true);
    assert_eq!(cur_info["initialblockdownload"], false);
    check_tx_indexed(
        slp_indexer,
        &get_coinbase_txid(bitcoind, &tip.hash)?,
        1,
        557,
        111,
    )?;

    // catchup finished
    assert!(slp_indexer.catchup_step()?);
    slp_indexer.leave_catchup()?;

    Ok(())
}

fn test_reorg_empty(slp_indexer: &SlpIndexer, bitcoind: &BitcoinCli) -> Result<()> {
    let anyone_script = Script::new(Bytes::from_bytes(vec![0x51])).to_p2sh();
    // build two empty blocks that reorg the previous block
    let db_blocks = slp_indexer.blocks()?;
    let db_txs = slp_indexer.txs()?;
    let tip = db_blocks.tip()?.unwrap();
    let old_txid = get_coinbase_txid(bitcoind, &tip.hash)?;
    check_tx_indexed(slp_indexer, &old_txid, 1, 557, 111)?;
    let block1 = build_lotus_block(
        tip.prev_hash.clone(),
        tip.timestamp,
        tip.height,
        build_lotus_coinbase(tip.height, anyone_script.clone()).hashed(),
        vec![],
        Sha256d::default(),
        vec![],
    );
    let block2 = build_lotus_block(
        block1.header.calc_hash(),
        tip.timestamp + 1,
        tip.height + 1,
        build_lotus_coinbase(tip.height + 1, anyone_script).hashed(),
        vec![],
        Sha256d::default(),
        vec![],
    );
    let result = bitcoind.cmd_string("submitblock", &[&block1.ser().hex()])?;
    assert_eq!(result, "inconclusive");
    let result = bitcoind.cmd_string("submitblock", &[&block2.ser().hex()])?;
    assert_eq!(result, "");

    // first message is BlockDisconnected
    slp_indexer.process_next_msg()?;
    // tip is moved back one block
    let new_tip = db_blocks.tip()?.unwrap();
    assert_eq!(new_tip.hash, tip.prev_hash);
    assert_eq!(db_txs.by_txid(&old_txid)?, None);
    check_tx_indexed(
        slp_indexer,
        &get_coinbase_txid(bitcoind, &new_tip.hash)?,
        0,
        170,
        217,
    )?;

    // next message is BlockConnected for block1
    slp_indexer.process_next_msg()?;
    // tip updated to block1
    let block1_tip = db_blocks.tip()?.unwrap();
    assert_eq!(block1_tip.hash, block1.header.calc_hash());
    assert_eq!(block1_tip.prev_hash, new_tip.hash);
    assert_eq!(db_txs.by_txid(&old_txid)?, None);
    check_tx_indexed(
        slp_indexer,
        &get_coinbase_txid(bitcoind, &block1_tip.hash)?,
        1,
        838,
        180,
    )?;

    // next message is BlockConnected for block2
    slp_indexer.process_next_msg()?;
    let block2_tip = db_blocks.tip()?.unwrap();
    assert_eq!(block2_tip.hash, block2.header.calc_hash());
    assert_eq!(block2_tip.prev_hash, block1_tip.hash);
    check_tx_indexed(
        slp_indexer,
        &get_coinbase_txid(bitcoind, &block2_tip.hash)?,
        2,
        1188,
        180,
    )?;

    Ok(())
}

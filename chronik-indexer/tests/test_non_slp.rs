use std::{ffi::OsString, str::FromStr, sync::Arc, time::Duration};

use bitcoinsuite_bitcoind::{
    cli::BitcoinCli,
    instance::{BitcoindChain, BitcoindConf, BitcoindInstance},
};
use bitcoinsuite_bitcoind_nng::{PubInterface, RpcInterface};
use bitcoinsuite_core::{
    build_lotus_block, build_lotus_coinbase, AddressType, BitcoinCode, Bytes, CashAddress, Hashed,
    Network, Op, OutPoint, Script, Sha256d, ShaRmd160, TxOutput, BCHREG,
};
use bitcoinsuite_ecc_secp256k1::EccSecp256k1;
use bitcoinsuite_error::Result;
use bitcoinsuite_slp::{RichTxBlock, RichUtxo};
use bitcoinsuite_test_utils::bin_folder;
use chronik_indexer::SlpIndexer;
use chronik_rocksdb::{
    BlockTx, Db, IndexDb, IndexMemData, OutpointEntry, PayloadPrefix, ScriptPayload, ScriptTxsConf,
    ScriptTxsReader, TransientData, TxEntry, UtxoEntry, UtxosReader,
};
use pretty_assertions::assert_eq;
use tempdir::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_non_slp() -> Result<()> {
    bitcoinsuite_error::install()?;
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
    let script_txs_conf = ScriptTxsConf { page_size: 1000 };
    let db = Db::open(dir.path().join("index.rocksdb"))?;
    let transient_data = TransientData::open(&dir.path().join("transient.rocksdb"))?;
    let db = IndexDb::new(db, transient_data, script_txs_conf);
    let bitcoin_cli = instance.cli();
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
    test_index_genesis(&mut slp_indexer, bitcoin_cli).await?;
    test_get_out_of_ibd(&mut slp_indexer, bitcoin_cli).await?;
    test_reorg_empty(&mut slp_indexer, bitcoin_cli).await?;
    instance.cleanup()?;
    Ok(())
}

fn get_coinbase_txid(bitcoind: &BitcoinCli, block_hash: &Sha256d) -> Result<Sha256d> {
    let node_block = bitcoind.cmd_json("getblock", &[&block_hash.to_hex_be()])?;
    let txid = Sha256d::from_hex_be(node_block["tx"][0].as_str().unwrap())?;
    Ok(txid)
}

#[allow(clippy::too_many_arguments)]
fn check_tx_indexed(
    slp_indexer: &SlpIndexer,
    txid: &Sha256d,
    block_height: i32,
    data_pos: u32,
    tx_size: u32,
    undo_pos: u32,
    undo_size: u32,
    time_first_seen: i64,
    is_coinbase: bool,
) -> Result<()> {
    let db_txs = slp_indexer.db().txs()?;
    assert_eq!(
        db_txs.by_txid(txid)?,
        Some(BlockTx {
            block_height,
            entry: TxEntry {
                txid: txid.clone(),
                data_pos,
                tx_size,
                undo_pos,
                undo_size,
                time_first_seen,
                is_coinbase,
            }
        })
    );
    Ok(())
}

async fn test_index_genesis(slp_indexer: &mut SlpIndexer, bitcoind: &BitcoinCli) -> Result<()> {
    let info = bitcoind.cmd_json("getblockchaininfo", &[])?;
    assert!(info["initialblockdownload"].as_bool().unwrap());
    assert_eq!(info["blocks"], 0i32);
    let db_blocks = slp_indexer.db().blocks()?;
    assert_eq!(db_blocks.height()?, -1);
    assert_eq!(db_blocks.tip()?, None);
    // index genesis block
    assert!(!slp_indexer.catchup_step().await?);
    let db_blocks = slp_indexer.db().blocks()?;
    assert_eq!(db_blocks.height()?, 0);
    let tip = db_blocks.tip()?.unwrap();
    assert_eq!(tip.hash.to_hex_be(), info["bestblockhash"]);
    let block = slp_indexer.blocks().by_height(0)?.unwrap();
    let raw_header = slp_indexer.blocks().raw_header(&block)?.unwrap();
    let block_header_hex = bitcoind.cmd_string(
        "getblockheader",
        &[info["bestblockhash"].as_str().unwrap(), "false"],
    )?;
    assert_eq!(hex::encode(&raw_header), block_header_hex);
    let coinbase_txid = get_coinbase_txid(bitcoind, &tip.hash)?;
    check_tx_indexed(slp_indexer, &coinbase_txid, 0, 170, 217, 0, 0, 0, true)?;
    let genesis_payload = hex::decode(
        "04678afdb0fe5548271967f1a67130b7105cd6a828e03909a67962e0ea1f61deb649f6bc3f4cef38c4f35504e5\
         1ec112de5c384df7ba0b8d578a4c702b6bf11d5f"
    )?;
    let r = &slp_indexer.db().script_txs()?;
    let db_utxos = &slp_indexer.db().utxos()?;
    let genesis_value = 130_000_000;
    check_pages(r, PayloadPrefix::P2PKLegacy, &genesis_payload, [&[0]])?;
    check_utxos(
        db_utxos,
        PayloadPrefix::P2PKLegacy,
        &genesis_payload,
        [(0, 1, genesis_value)],
    )?;
    assert_eq!(
        slp_indexer.blocks().block_txs_by_hash(&block.hash)?,
        vec![slp_indexer.txs().rich_tx_by_txid(&coinbase_txid)?.unwrap()],
    );
    assert_eq!(
        slp_indexer.blocks().block_txs_by_hash(&block.hash)?,
        slp_indexer.blocks().block_txs_by_height(0)?,
    );
    assert_eq!(
        slp_indexer.utxos().utxos(&ScriptPayload {
            payload_prefix: PayloadPrefix::P2PKLegacy,
            payload_data: genesis_payload.clone()
        })?,
        vec![RichUtxo {
            outpoint: OutPoint {
                txid: coinbase_txid,
                out_idx: 1,
            },
            block: Some(RichTxBlock {
                height: 0,
                hash: tip.hash.clone(),
                timestamp: tip.timestamp,
            }),
            is_coinbase: true,
            output: TxOutput {
                value: genesis_value,
                script: Script::from_ops(
                    [
                        Op::Push(genesis_payload.len() as u8, genesis_payload.into()),
                        Op::Code(0xac),
                    ]
                    .into_iter()
                )?,
            },
            slp_output: None,
            time_first_seen: 0,
            network: Network::XPI,
        }],
    );
    Ok(())
}

async fn test_get_out_of_ibd(slp_indexer: &mut SlpIndexer, bitcoind: &BitcoinCli) -> Result<()> {
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
    assert!(!slp_indexer.catchup_step().await?);
    gen_handle.join().unwrap();
    let cur_info = bitcoind.cmd_json("getblockchaininfo", &[])?;
    let tip = slp_indexer.db().blocks()?.tip()?.unwrap();
    assert_eq!(tip.prev_hash.to_hex_be(), prev_info["bestblockhash"]);
    assert_eq!(tip.hash.to_hex_be(), cur_info["bestblockhash"]);
    assert_eq!(prev_info["initialblockdownload"], true);
    assert_eq!(cur_info["initialblockdownload"], false);
    let coinbase_txid = get_coinbase_txid(bitcoind, &tip.hash)?;
    check_tx_indexed(slp_indexer, &coinbase_txid, 1, 557, 111, 0, 0, 0, true)?;
    let r = &slp_indexer.db().script_txs()?;
    let db_utxos = &slp_indexer.db().utxos()?;
    check_pages(r, PayloadPrefix::P2SH, &[0; 20], [&[1]])?;
    check_utxos(
        db_utxos,
        PayloadPrefix::P2SH,
        &[0; 20],
        [(1, 1, 260_000_000)],
    )?;
    let rich_coinbase_tx = slp_indexer.txs().rich_tx_by_txid(&coinbase_txid)?.unwrap();
    assert_eq!(
        slp_indexer.blocks().block_txs_by_hash(&tip.hash)?,
        vec![rich_coinbase_tx.clone()],
    );
    assert_eq!(
        slp_indexer.blocks().block_txs_by_hash(&tip.hash)?,
        slp_indexer.blocks().block_txs_by_height(1)?,
    );
    assert_eq!(
        slp_indexer.utxos().utxos(&ScriptPayload {
            payload_prefix: PayloadPrefix::P2SH,
            payload_data: vec![0; 20],
        })?,
        vec![RichUtxo {
            outpoint: OutPoint {
                txid: coinbase_txid,
                out_idx: 1,
            },
            block: Some(RichTxBlock {
                height: 1,
                hash: tip.hash.clone(),
                timestamp: tip.timestamp,
            }),
            is_coinbase: true,
            output: rich_coinbase_tx.tx.outputs()[1].clone(),
            slp_output: None,
            time_first_seen: 0,
            network: Network::XPI,
        }],
    );

    // catchup finished
    assert!(slp_indexer.catchup_step().await?);
    slp_indexer.leave_catchup()?;

    Ok(())
}

async fn test_reorg_empty(slp_indexer: &mut SlpIndexer, bitcoind: &BitcoinCli) -> Result<()> {
    let anyone_payload = ShaRmd160::digest(&Bytes::from_bytes(vec![0x51]));
    let anyone_script = Script::p2sh(&anyone_payload);
    let anyone_payload = anyone_payload.as_slice();
    // build two empty blocks that reorg the previous block
    let tip = slp_indexer.db().blocks()?.tip()?.unwrap();
    let old_txid = get_coinbase_txid(bitcoind, &tip.hash)?;
    check_tx_indexed(slp_indexer, &old_txid, 1, 557, 111, 0, 0, 0, true)?;
    check_pages(
        &slp_indexer.db().script_txs()?,
        PayloadPrefix::P2SH,
        &[0; 20],
        [&[1]],
    )?;
    check_utxos(
        &slp_indexer.db().utxos()?,
        PayloadPrefix::P2SH,
        &[0; 20],
        [(1, 1, 260_000_000)],
    )?;
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
    let new_tip = slp_indexer.db().blocks()?.tip()?.unwrap();
    assert_eq!(new_tip.hash, tip.prev_hash);
    assert_eq!(slp_indexer.db().txs()?.by_txid(&old_txid)?, None);
    check_tx_indexed(
        slp_indexer,
        &get_coinbase_txid(bitcoind, &new_tip.hash)?,
        0,
        170,
        217,
        0,
        0,
        0,
        true,
    )?;
    check_pages(
        &slp_indexer.db().script_txs()?,
        PayloadPrefix::P2SH,
        &[0; 20],
        [],
    )?;
    check_utxos(
        &slp_indexer.db().utxos()?,
        PayloadPrefix::P2SH,
        &[0; 20],
        [],
    )?;
    check_pages(
        &slp_indexer.db().script_txs()?,
        PayloadPrefix::P2SH,
        anyone_payload,
        [],
    )?;
    check_utxos(
        &slp_indexer.db().utxos()?,
        PayloadPrefix::P2SH,
        anyone_payload,
        [],
    )?;

    // next message is BlockConnected for block1
    slp_indexer.process_next_msg()?;
    // tip updated to block1
    let block1_tip = slp_indexer.db().blocks()?.tip()?.unwrap();
    assert_eq!(block1_tip.hash, block1.header.calc_hash());
    assert_eq!(block1_tip.prev_hash, new_tip.hash);
    assert_eq!(slp_indexer.db().txs()?.by_txid(&old_txid)?, None);
    let coinbase_txid1 = get_coinbase_txid(bitcoind, &block1_tip.hash)?;
    check_tx_indexed(slp_indexer, &coinbase_txid1, 1, 838, 180, 0, 0, 0, true)?;
    check_pages(
        &slp_indexer.db().script_txs()?,
        PayloadPrefix::P2SH,
        anyone_payload,
        [&[1]],
    )?;
    check_utxos(
        &slp_indexer.db().utxos()?,
        PayloadPrefix::P2SH,
        anyone_payload,
        [(1, 1, 260_000_000)],
    )?;
    assert_eq!(
        slp_indexer.blocks().block_txs_by_hash(&block1_tip.hash)?,
        vec![slp_indexer.txs().rich_tx_by_txid(&coinbase_txid1)?.unwrap()],
    );

    // next message is BlockConnected for block2
    slp_indexer.process_next_msg()?;
    let block2_tip = slp_indexer.db().blocks()?.tip()?.unwrap();
    assert_eq!(block2_tip.hash, block2.header.calc_hash());
    assert_eq!(block2_tip.prev_hash, block1_tip.hash);
    let coinbase_txid2 = get_coinbase_txid(bitcoind, &block2_tip.hash)?;
    check_tx_indexed(slp_indexer, &coinbase_txid2, 2, 1188, 180, 0, 0, 0, true)?;
    check_pages(
        &slp_indexer.db().script_txs()?,
        PayloadPrefix::P2SH,
        anyone_payload,
        [&[1, 2]],
    )?;
    check_utxos(
        &slp_indexer.db().utxos()?,
        PayloadPrefix::P2SH,
        anyone_payload,
        [(1, 1, 260_000_000), (2, 1, 260_000_000)],
    )?;
    assert_eq!(
        slp_indexer.blocks().block_txs_by_hash(&block2_tip.hash)?,
        vec![slp_indexer.txs().rich_tx_by_txid(&coinbase_txid2)?.unwrap()],
    );

    Ok(())
}

fn check_pages<const N: usize>(
    script_txs_reader: &ScriptTxsReader,
    prefix: PayloadPrefix,
    payload_body: &[u8],
    expected_txs: [&[u64]; N],
) -> Result<()> {
    assert_eq!(
        script_txs_reader.num_pages_by_payload(prefix, payload_body)?,
        N,
    );
    for (page_num, txs) in expected_txs.into_iter().enumerate() {
        assert_eq!(
            script_txs_reader.page_txs(page_num as u32, prefix, payload_body)?,
            txs.to_vec(),
        );
    }
    Ok(())
}

fn check_utxos<const N: usize>(
    utxo_reader: &UtxosReader,
    prefix: PayloadPrefix,
    payload_body: &[u8],
    expected_txs: [(u64, u32, i64); N],
) -> Result<()> {
    assert_eq!(
        utxo_reader.utxos(prefix, payload_body)?,
        expected_txs
            .into_iter()
            .map(|(tx_num, out_idx, value)| UtxoEntry {
                outpoint: OutpointEntry { tx_num, out_idx },
                value,
                is_partial_script: false,
            })
            .collect::<Vec<_>>(),
    );
    Ok(())
}

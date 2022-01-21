use std::{collections::HashSet, ffi::OsString, str::FromStr, sync::Arc, time::Duration};

use bitcoinsuite_bitcoind::{
    cli::BitcoinCli,
    instance::{BitcoindChain, BitcoindConf, BitcoindInstance},
};
use bitcoinsuite_bitcoind_nng::{PubInterface, RpcInterface};
use bitcoinsuite_core::{
    build_lotus_block, build_lotus_coinbase, lotus_txid, AddressType, BitcoinCode, CashAddress,
    Coin, Hashed, Network, OutPoint, Script, Sha256d, ShaRmd160, TxInput, TxOutput, UnhashedTx,
    BCHREG,
};
use bitcoinsuite_ecc_secp256k1::EccSecp256k1;
use bitcoinsuite_error::Result;
use bitcoinsuite_slp::{
    genesis_opreturn, send_opreturn, RichTx, RichTxBlock, RichUtxo, SlpAmount, SlpGenesisInfo,
    SlpOutput, SlpToken, SlpTokenType, SlpTxData, SlpTxType, SlpValidTxData, TokenId,
};
use bitcoinsuite_test_utils::bin_folder;
use bitcoinsuite_test_utils_blockchain::build_tx;
use chronik_indexer::{
    broadcast::BroadcastError, subscribers::SubscribeMessage, SlpIndexer, UtxoState,
    UtxoStateVariant,
};
use chronik_rocksdb::{
    BlockStats, Db, IndexDb, IndexMemData, MempoolTxEntry, PayloadPrefix, ScriptPayload,
    ScriptTxsConf,
};
use pretty_assertions::{assert_eq, assert_ne};
use tempdir::TempDir;
use tokio::time::timeout;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mempool() -> Result<()> {
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
    let outputs_conf = ScriptTxsConf { page_size: 7 };
    let db = Db::open(dir.path().join("index.rocksdb"))?;
    let db = IndexDb::new(db, outputs_conf);
    let bitcoind = instance.cli();
    let cache = IndexMemData::new(10);
    let mut slp_indexer = SlpIndexer::new(
        db,
        bitcoind.clone(),
        rpc_interface,
        pub_interface,
        cache,
        Network::XPI,
        Arc::new(EccSecp256k1::default()),
    )?;
    bitcoind.cmd_string("setmocktime", &["2000000000"])?;
    test_index_mempool(&mut slp_indexer, bitcoind).await?;
    instance.cleanup()?;
    Ok(())
}

async fn test_index_mempool(slp_indexer: &mut SlpIndexer, bitcoind: &BitcoinCli) -> Result<()> {
    use PayloadPrefix::P2SH;
    let anyone_script = Script::from_slice(&[0x51]);
    let anyone_hash = ShaRmd160::digest(anyone_script.bytecode().clone());
    let anyone_slice = anyone_hash.as_slice();
    let anyone_address = CashAddress::from_hash(BCHREG, AddressType::P2SH, anyone_hash.clone());

    assert_eq!(
        slp_indexer
            .script_history()
            .rev_history_num_pages(P2SH, anyone_slice, 4)?,
        0,
    );

    bitcoind.cmd_json("generatetoaddress", &["10", anyone_address.as_str()])?;
    while !slp_indexer.catchup_step()? {}
    assert_eq!(
        slp_indexer
            .script_history()
            .rev_history_num_pages(P2SH, anyone_slice, 4)?,
        3,
    );

    let burn_address = CashAddress::from_hash(BCHREG, AddressType::P2SH, ShaRmd160::new([0; 20]));
    let burn_hash = [1; 20];
    let burn_address2 =
        CashAddress::from_hash(BCHREG, AddressType::P2SH, ShaRmd160::new(burn_hash));
    bitcoind.cmd_json("generatetoaddress", &["100", burn_address.as_str()])?;
    while !slp_indexer.catchup_step()? {}
    slp_indexer.leave_catchup()?;

    bitcoind.cmd_string("setmocktime", &["2100000000"])?;

    let dt_timeout = Duration::from_secs(3);
    let mut receiver = slp_indexer.subscribers_mut().subscribe(&ScriptPayload {
        payload_prefix: P2SH,
        payload_data: anyone_slice.to_vec(),
    });

    let utxo_entries = slp_indexer.db().utxos()?.utxos(P2SH, anyone_slice)?;
    assert_eq!(utxo_entries.len(), 10);

    {
        use TxIdentifier::*;
        let addrs = slp_indexer.script_history();
        let db_script_txs = slp_indexer.db().script_txs()?;
        assert_eq!(addrs.num_mempool_txs(P2SH, anyone_slice), 0);
        assert_eq!(
            db_script_txs.page_txs(0, P2SH, anyone_slice)?,
            vec![1, 2, 3, 4, 5, 6, 7],
        );
        assert_eq!(
            db_script_txs.page_txs(1, P2SH, anyone_slice)?,
            vec![8, 9, 10],
        );
        check_rev_history_pages(
            slp_indexer,
            P2SH,
            anyone_slice,
            4,
            [
                &[N(10), N(9), N(8), N(7)],
                &[N(6), N(5), N(4), N(3)],
                &[N(2), N(1)],
            ],
        )?;
        check_rich_utxos(
            slp_indexer,
            P2SH,
            anyone_slice,
            [
                (N(1), 1, true),
                (N(2), 1, true),
                (N(3), 1, true),
                (N(4), 1, true),
                (N(5), 1, true),
                (N(6), 1, true),
                (N(7), 1, true),
                (N(8), 1, true),
                (N(9), 1, true),
                (N(10), 1, true),
            ],
        )?;
    }

    let mut utxos = Vec::new();
    for utxo_entry in utxo_entries {
        let tx_entry = slp_indexer
            .db()
            .txs()?
            .by_tx_num(utxo_entry.outpoint.tx_num)?
            .unwrap();
        utxos.push((
            OutPoint {
                txid: tx_entry.entry.txid,
                out_idx: utxo_entry.outpoint.out_idx,
            },
            260_000_000,
        ));
    }
    assert_eq!(
        slp_indexer.utxos().utxo_state(&utxos[0].0)?,
        UtxoState {
            height: Some(1),
            state: UtxoStateVariant::Unspent,
        },
    );

    let (outpoint, value) = utxos.pop().unwrap();
    let leftover_value = value - 20_000;
    let tx1 = build_tx(
        outpoint.clone(),
        &anyone_script,
        vec![
            TxOutput {
                value: 10_000,
                script: burn_address2.to_script(),
            },
            TxOutput {
                value: leftover_value,
                script: anyone_script.to_p2sh(),
            },
        ],
    );
    slp_indexer.broadcast().test_mempool_accept(&tx1, true)??;
    let txid1 = slp_indexer.broadcast().broadcast_tx(&tx1, true)?;
    assert_eq!(
        slp_indexer.broadcast().test_mempool_accept(&tx1, true)?,
        Err(BroadcastError::BitcoindRejectedTx(
            "txn-already-in-mempool".to_string()
        )),
    );
    let mut rich_tx1 = RichTx {
        tx: tx1.clone().hashed(),
        txid: txid1.clone(),
        block: None,
        slp_tx_data: None,
        spent_coins: Some(vec![Coin {
            tx_output: TxOutput {
                value,
                script: anyone_script.to_p2sh(),
            },
            height: Some(10),
            is_coinbase: true,
        }]),
        spends: vec![None, None],
        slp_burns: vec![None],
        slp_error_msg: None,
        time_first_seen: 2_100_000_000,
        network: Network::XPI,
    };
    slp_indexer.process_next_msg()?;
    match timeout(dt_timeout, receiver.recv()).await?? {
        SubscribeMessage::AddedToMempool(txid) => assert_eq!(txid, txid1),
        _ => panic!("Wrong message received"),
    }
    assert_eq!(
        slp_indexer.db_mempool().tx(&txid1),
        Some(&MempoolTxEntry {
            tx: tx1.clone(),
            spent_coins: rich_tx1.spent_coins.clone().unwrap(),
            time_first_seen: 2_100_000_000,
        }),
    );
    assert_eq!(slp_indexer.db_mempool_slp().slp_tx_data(&txid1), None);
    assert_eq!(slp_indexer.db_mempool_slp().slp_tx_error(&txid1), None);
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid1)?,
        Some(rich_tx1.clone())
    );
    assert_eq!(slp_indexer.txs().raw_tx_by_id(&txid1)?, Some(tx1.ser()));
    {
        use TxIdentifier::*;
        let addrs = slp_indexer.script_history();
        assert_eq!(addrs.num_mempool_txs(P2SH, anyone_slice), 1);
        check_rev_history_pages(
            slp_indexer,
            P2SH,
            anyone_slice,
            4,
            [
                &[H(&txid1), N(10), N(9), N(8)],
                &[N(7), N(6), N(5), N(4)],
                &[N(3), N(2), N(1)],
            ],
        )?;
        check_rev_history_pages(slp_indexer, P2SH, &burn_hash, 4, [&[H(&txid1)]])?;
        check_rich_utxos(
            slp_indexer,
            P2SH,
            anyone_slice,
            [
                (N(1), 1, true),
                (N(2), 1, true),
                (N(3), 1, true),
                (N(4), 1, true),
                (N(5), 1, true),
                (N(6), 1, true),
                (N(7), 1, true),
                (N(8), 1, true),
                (N(9), 1, true),
                (H(&txid1), 1, false),
            ],
        )?;
    }
    assert_eq!(
        slp_indexer.utxos().utxo_state(&outpoint)?,
        UtxoState {
            height: Some(10),
            state: UtxoStateVariant::Spent,
        },
    );
    assert_eq!(
        slp_indexer.utxos().utxo_state(&OutPoint {
            txid: txid1.clone(),
            out_idx: 1
        })?,
        UtxoState {
            height: None,
            state: UtxoStateVariant::Unspent,
        },
    );
    assert_eq!(
        slp_indexer.utxos().utxo_state(&OutPoint {
            txid: txid1.clone(),
            out_idx: 2
        })?,
        UtxoState {
            height: None,
            state: UtxoStateVariant::NoSuchOutput,
        },
    );

    bitcoind.cmd_string("setmocktime", &["2100000001"])?;
    let (outpoint, value) = utxos.pop().unwrap();
    let tx2 = build_tx(
        outpoint,
        &anyone_script,
        vec![
            TxOutput {
                value: 0,
                script: genesis_opreturn(
                    &SlpGenesisInfo::default(),
                    SlpTokenType::Fungible,
                    None,
                    100,
                ),
            },
            TxOutput {
                value: leftover_value,
                script: anyone_script.to_p2sh(),
            },
        ],
    );
    slp_indexer.broadcast().test_mempool_accept(&tx2, true)??;
    let txid2 = slp_indexer.broadcast().broadcast_tx(&tx2, true)?;
    let token_id = TokenId::new(txid2.clone());
    let slp_tx_data2 = SlpValidTxData {
        slp_tx_data: SlpTxData {
            input_tokens: vec![SlpToken::EMPTY],
            output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(100)],
            slp_token_type: SlpTokenType::Fungible,
            slp_tx_type: SlpTxType::Genesis(SlpGenesisInfo::default().into()),
            token_id: token_id.clone(),
            group_token_id: None,
        },
        slp_burns: vec![None],
    };
    let mut rich_tx2 = RichTx {
        tx: tx2.clone().hashed(),
        txid: txid2.clone(),
        block: None,
        slp_tx_data: Some(slp_tx_data2.slp_tx_data.clone().into()),
        spent_coins: Some(vec![Coin {
            tx_output: TxOutput {
                value,
                script: anyone_script.to_p2sh(),
            },
            height: Some(9),
            is_coinbase: true,
        }]),
        spends: vec![None, None],
        slp_burns: vec![None],
        slp_error_msg: None,
        time_first_seen: 2_100_000_001,
        network: Network::XPI,
    };
    slp_indexer.process_next_msg()?;
    match timeout(dt_timeout, receiver.recv()).await?? {
        SubscribeMessage::AddedToMempool(txid) => assert_eq!(txid, txid2),
        _ => panic!("Wrong message received"),
    }
    assert_eq!(
        slp_indexer.db_mempool().tx(&txid2),
        Some(&MempoolTxEntry {
            tx: tx2.clone(),
            spent_coins: rich_tx2.spent_coins.clone().unwrap(),
            time_first_seen: 2_100_000_001,
        }),
    );
    assert_eq!(
        slp_indexer.db_mempool_slp().slp_tx_data(&txid2),
        Some(&slp_tx_data2),
    );
    assert_eq!(slp_indexer.db_mempool_slp().slp_tx_error(&txid2), None);
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid1)?,
        Some(rich_tx1.clone())
    );
    assert_eq!(slp_indexer.txs().raw_tx_by_id(&txid1)?, Some(tx1.ser()));
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid2)?,
        Some(rich_tx2.clone())
    );
    assert_eq!(slp_indexer.txs().raw_tx_by_id(&txid2)?, Some(tx2.ser()));
    {
        use TxIdentifier::*;
        let addrs = slp_indexer.script_history();
        assert_eq!(addrs.num_mempool_txs(P2SH, anyone_slice), 2);
        check_rev_history_pages(
            slp_indexer,
            P2SH,
            anyone_slice,
            4,
            [
                &[H(&txid2), H(&txid1), N(10), N(9)],
                &[N(8), N(7), N(6), N(5)],
                &[N(4), N(3), N(2), N(1)],
            ],
        )?;
        check_rich_utxos(
            slp_indexer,
            P2SH,
            anyone_slice,
            [
                (N(1), 1, true),
                (N(2), 1, true),
                (N(3), 1, true),
                (N(4), 1, true),
                (N(5), 1, true),
                (N(6), 1, true),
                (N(7), 1, true),
                (N(8), 1, true),
                (H(&txid1), 1, false),
                (H(&txid2), 1, false),
            ],
        )?;
    }

    bitcoind.cmd_string("setmocktime", &["2100000002"])?;
    let (outpoint, value) = utxos.pop().unwrap();
    let send_value = leftover_value * 2 + value - 20_000;
    let mut tx3 = UnhashedTx {
        version: 1,
        inputs: vec![
            TxInput {
                prev_out: outpoint.clone(),
                script: Script::new(anyone_script.bytecode().ser()),
                ..Default::default()
            },
            TxInput {
                prev_out: OutPoint {
                    txid: txid1.clone(),
                    out_idx: 1,
                },
                script: Script::new(anyone_script.bytecode().ser()),
                ..Default::default()
            },
            TxInput {
                prev_out: OutPoint {
                    txid: txid2.clone(),
                    out_idx: 1,
                },
                script: Script::new(anyone_script.bytecode().ser()),
                ..Default::default()
            },
        ],
        outputs: vec![
            TxOutput {
                value: 0,
                script: send_opreturn(&token_id, SlpTokenType::Fungible, &[SlpAmount::new(100)]),
            },
            TxOutput {
                value: send_value,
                script: anyone_address.to_script(),
            },
        ],
        lock_time: 0,
    };
    slp_indexer.broadcast().test_mempool_accept(&tx3, true)??;
    let txid3 = slp_indexer.broadcast().broadcast_tx(&tx3, true)?;
    let slp_tx_data3 = SlpValidTxData {
        slp_tx_data: SlpTxData {
            input_tokens: vec![SlpToken::EMPTY, SlpToken::EMPTY, SlpToken::amount(100)],
            output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(100)],
            slp_token_type: SlpTokenType::Fungible,
            slp_tx_type: SlpTxType::Send,
            token_id,
            group_token_id: None,
        },
        slp_burns: vec![None, None, None],
    };
    let mut rich_tx3 = RichTx {
        tx: tx3.clone().hashed(),
        txid: txid3.clone(),
        block: None,
        slp_tx_data: Some(slp_tx_data3.slp_tx_data.clone().into()),
        spent_coins: Some(vec![
            Coin {
                tx_output: TxOutput {
                    value,
                    script: anyone_script.to_p2sh(),
                },
                height: Some(8),
                is_coinbase: true,
            },
            Coin {
                tx_output: TxOutput {
                    value: leftover_value,
                    script: anyone_script.to_p2sh(),
                },
                height: None,
                is_coinbase: false,
            },
            Coin {
                tx_output: TxOutput {
                    value: leftover_value,
                    script: anyone_script.to_p2sh(),
                },
                height: None,
                is_coinbase: false,
            },
        ]),
        spends: vec![None, None],
        slp_burns: vec![None, None, None],
        slp_error_msg: None,
        time_first_seen: 2_100_000_002,
        network: Network::XPI,
    };
    slp_indexer.process_next_msg()?;
    match timeout(dt_timeout, receiver.recv()).await?? {
        SubscribeMessage::AddedToMempool(txid) => assert_eq!(txid, txid3),
        _ => panic!("Wrong message received"),
    }
    assert_eq!(
        slp_indexer.db_mempool().tx(&txid3),
        Some(&MempoolTxEntry {
            tx: tx3.clone(),
            spent_coins: rich_tx3.spent_coins.clone().unwrap_or_default(),
            time_first_seen: 2_100_000_002,
        }),
    );
    assert_eq!(
        slp_indexer.db_mempool_slp().slp_tx_data(&txid3),
        Some(&slp_tx_data3),
    );
    assert_eq!(slp_indexer.db_mempool_slp().slp_tx_error(&txid3), None);
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid3)?,
        Some(rich_tx3.clone())
    );
    assert_eq!(slp_indexer.txs().raw_tx_by_id(&txid1)?, Some(tx1.ser()));
    assert_eq!(slp_indexer.txs().raw_tx_by_id(&txid2)?, Some(tx2.ser()));
    assert_eq!(slp_indexer.txs().raw_tx_by_id(&txid3)?, Some(tx3.ser()));
    // tx1 and tx2 have now spends
    rich_tx1.spends[1] = Some(OutPoint {
        txid: txid3.clone(),
        out_idx: 1,
    });
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid1)?,
        Some(rich_tx1.clone())
    );
    rich_tx2.spends[1] = Some(OutPoint {
        txid: txid3.clone(),
        out_idx: 2,
    });
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid2)?,
        Some(rich_tx2.clone())
    );
    {
        use TxIdentifier::*;
        let addrs = slp_indexer.script_history();
        assert_eq!(addrs.num_mempool_txs(P2SH, anyone_slice), 3);
        check_rev_history_pages(
            slp_indexer,
            P2SH,
            anyone_slice,
            4,
            [
                &[H(&txid3), H(&txid2), H(&txid1), N(10)],
                &[N(9), N(8), N(7), N(6)],
                &[N(5), N(4), N(3), N(2)],
                &[N(1)],
            ],
        )?;
        check_rev_history_pages(
            slp_indexer,
            P2SH,
            anyone_slice,
            3,
            [
                &[H(&txid3), H(&txid2), H(&txid1)],
                &[N(10), N(9), N(8)],
                &[N(7), N(6), N(5)],
                &[N(4), N(3), N(2)],
                &[N(1)],
            ],
        )?;
        check_rev_history_pages(
            slp_indexer,
            P2SH,
            anyone_slice,
            2,
            [
                &[H(&txid3), H(&txid2)],
                &[H(&txid1), N(10)],
                &[N(9), N(8)],
                &[N(7), N(6)],
                &[N(5), N(4)],
                &[N(3), N(2)],
                &[N(1)],
            ],
        )?;
        check_rich_utxos(
            slp_indexer,
            P2SH,
            anyone_slice,
            [
                (N(1), 1, true),
                (N(2), 1, true),
                (N(3), 1, true),
                (N(4), 1, true),
                (N(5), 1, true),
                (N(6), 1, true),
                (N(7), 1, true),
                (H(&txid3), 1, false),
            ],
        )?;
    }
    assert_eq!(
        slp_indexer.utxos().utxo_state(&outpoint)?,
        UtxoState {
            height: Some(8),
            state: UtxoStateVariant::Spent,
        },
    );
    assert_eq!(
        slp_indexer.utxos().utxo_state(&OutPoint {
            txid: txid1.clone(),
            out_idx: 1,
        })?,
        UtxoState {
            height: None,
            state: UtxoStateVariant::Spent,
        },
    );
    assert_eq!(
        slp_indexer.utxos().utxo_state(&OutPoint {
            txid: txid3.clone(),
            out_idx: 1,
        })?,
        UtxoState {
            height: None,
            state: UtxoStateVariant::Unspent,
        },
    );

    let tip = slp_indexer.db().blocks()?.tip()?.unwrap();
    let tx1 = tx1.hashed();
    let coinbase_tx = build_lotus_coinbase(tip.height + 1, anyone_script.to_p2sh());
    let coinbase_txid = lotus_txid(&coinbase_tx);
    let block1 = build_lotus_block(
        tip.hash.clone(),
        tip.timestamp + 1,
        tip.height + 1,
        coinbase_tx.hashed(),
        vec![tx1.clone()],
        Sha256d::default(),
        vec![],
    );
    let result = bitcoind.cmd_string("submitblock", &[&block1.ser().hex()])?;
    assert_eq!(result, "");

    slp_indexer.process_next_msg()?;

    let mut subbed_txids = HashSet::new();
    for _ in 0..2 {
        subbed_txids.insert(match timeout(dt_timeout, receiver.recv()).await?? {
            SubscribeMessage::Confirmed(txid) => txid,
            _ => panic!("Wrong message received"),
        });
    }
    assert_eq!(
        subbed_txids,
        [&coinbase_txid, &txid1].into_iter().cloned().collect()
    );
    let block_tx = slp_indexer.db().txs()?.by_txid(&txid1)?.unwrap();
    assert_eq!(block_tx.entry.txid, txid1);
    assert_eq!(block_tx.entry.tx_size, tx1.raw().len() as u32);
    assert_eq!(block_tx.block_height, 111);
    assert_eq!(slp_indexer.db_mempool().tx(&txid1), None);
    assert!(slp_indexer.db_mempool().tx(&txid2).is_some());
    assert!(slp_indexer.db_mempool_slp().slp_tx_data(&txid2).is_some());
    assert!(slp_indexer.db_mempool().tx(&txid3).is_some());
    assert!(slp_indexer.db_mempool_slp().slp_tx_data(&txid3).is_some());
    rich_tx1.block = Some(RichTxBlock {
        height: 111,
        hash: block1.header.calc_hash(),
        timestamp: block1.header.timestamp,
    });
    rich_tx1.spent_coins.as_mut().unwrap()[0].height = Some(10);
    rich_tx1.spent_coins.as_mut().unwrap()[0].is_coinbase = true;
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid1)?,
        Some(rich_tx1.clone())
    );
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid2)?,
        Some(rich_tx2.clone())
    );
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid3)?,
        Some(rich_tx3.clone())
    );
    assert_eq!(slp_indexer.txs().raw_tx_by_id(&txid1)?, Some(tx1.ser()));
    assert_eq!(slp_indexer.txs().raw_tx_by_id(&txid2)?, Some(tx2.ser()));
    assert_eq!(slp_indexer.txs().raw_tx_by_id(&txid3)?, Some(tx3.ser()));
    assert_eq!(
        slp_indexer
            .blocks()
            .block_txs_by_hash(&block1.header.calc_hash())?[1..],
        [rich_tx1.clone()],
    );
    assert_eq!(
        slp_indexer
            .blocks()
            .block_txs_by_hash(&block1.header.calc_hash())?,
        slp_indexer.blocks().block_txs_by_height(111)?,
    );
    {
        use TxIdentifier::*;
        check_rich_utxos(
            slp_indexer,
            P2SH,
            anyone_slice,
            [
                (N(1), 1, true),
                (N(2), 1, true),
                (N(3), 1, true),
                (N(4), 1, true),
                (N(5), 1, true),
                (N(6), 1, true),
                (N(7), 1, true),
                (N(111), 1, true),
                (H(&txid3), 1, false),
            ],
        )?;
    }

    // modify tx3
    tx3.outputs[1].value -= 1;

    let tip = slp_indexer.db().blocks()?.tip()?.unwrap();
    let tx2 = tx2.hashed();
    let tx3 = tx3.hashed();
    let txid3_modified = lotus_txid(tx3.unhashed_tx());
    let coinbase_tx = build_lotus_coinbase(tip.height + 1, anyone_script.to_p2sh());
    let coinbase_txid = lotus_txid(&coinbase_tx);
    let block2 = build_lotus_block(
        tip.hash.clone(),
        tip.timestamp + 1,
        tip.height + 1,
        coinbase_tx.hashed(),
        vec![tx2.clone(), tx3.clone()],
        Sha256d::default(),
        vec![],
    );
    let result = bitcoind.cmd_string("submitblock", &[&block2.ser().hex()])?;
    assert_eq!(result, "");

    // Remove tx3 from mempool
    slp_indexer.process_next_msg()?;
    match timeout(dt_timeout, receiver.recv()).await?? {
        SubscribeMessage::RemovedFromMempool(txid) => assert_eq!(txid, txid3),
        _ => panic!("Wrong message received"),
    }
    // Process block
    slp_indexer.process_next_msg()?;
    let mut subbed_txids = HashSet::new();
    for _ in 0..3 {
        subbed_txids.insert(match timeout(dt_timeout, receiver.recv()).await?? {
            SubscribeMessage::Confirmed(txid) => txid,
            _ => panic!("Wrong message received"),
        });
    }
    assert_eq!(
        subbed_txids,
        [&coinbase_txid, &txid2, &txid3_modified]
            .into_iter()
            .cloned()
            .collect()
    );

    assert_eq!(slp_indexer.db_mempool().tx(&txid1), None);
    assert_eq!(slp_indexer.db_mempool().tx(&txid2), None);
    assert_eq!(slp_indexer.db_mempool().tx(&txid3), None);

    let block_tx = slp_indexer.db().txs()?.by_txid(&txid2)?.unwrap();
    assert_eq!(block_tx.entry.txid, txid2);
    assert_eq!(block_tx.entry.tx_size, tx2.raw().len() as u32);
    assert_eq!(block_tx.block_height, 112);

    assert_eq!(slp_indexer.db().txs()?.by_txid(&txid3)?, None);

    assert_ne!(txid3, txid3_modified);
    let block_tx = slp_indexer.db().txs()?.by_txid(&txid3_modified)?.unwrap();
    assert_eq!(block_tx.entry.txid, txid3_modified);
    assert_eq!(block_tx.entry.tx_size, tx3.raw().len() as u32);
    assert_eq!(block_tx.block_height, 112);

    assert_eq!(slp_indexer.txs().raw_tx_by_id(&txid1)?, Some(tx1.ser()));
    assert_eq!(slp_indexer.txs().raw_tx_by_id(&txid2)?, Some(tx2.ser()));
    assert_eq!(slp_indexer.txs().raw_tx_by_id(&txid3)?, None);
    assert_eq!(
        slp_indexer.txs().raw_tx_by_id(&txid3_modified)?,
        Some(tx3.ser())
    );

    // tx1 and tx2 have now different spends
    rich_tx1.spends[1] = Some(OutPoint {
        txid: txid3_modified.clone(),
        out_idx: 1,
    });
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid1)?,
        Some(rich_tx1.clone())
    );
    assert_eq!(rich_tx1.timestamp(), 2_100_000_000);
    rich_tx2.spends[1] = Some(OutPoint {
        txid: txid3_modified.clone(),
        out_idx: 2,
    });
    rich_tx2.block = Some(RichTxBlock {
        height: 112,
        hash: block2.header.calc_hash(),
        timestamp: block2.header.timestamp,
    });
    rich_tx2.spent_coins.as_mut().unwrap()[0].height = Some(9);
    rich_tx2.spent_coins.as_mut().unwrap()[0].is_coinbase = true;
    assert_eq!(rich_tx2.timestamp(), 2_100_000_001);
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid2)?,
        Some(rich_tx2.clone())
    );
    rich_tx3.tx = tx3;
    rich_tx3.txid = txid3_modified.clone();
    rich_tx3.block = Some(RichTxBlock {
        height: 112,
        hash: block2.header.calc_hash(),
        timestamp: block2.header.timestamp,
    });
    {
        let spent_coins = rich_tx3.spent_coins.as_mut().unwrap();
        spent_coins[0].height = Some(8);
        spent_coins[0].is_coinbase = true;
        spent_coins[1].height = Some(111);
        spent_coins[2].height = Some(112);
    }
    rich_tx3.time_first_seen = 0; // tx not first seen in mempool
    let spent_coins = rich_tx3.spent_coins.as_mut().unwrap();
    spent_coins[1].height = Some(111);
    spent_coins[2].height = Some(112);
    assert_eq!(rich_tx3.timestamp(), block2.header.timestamp);
    assert_eq!(slp_indexer.txs().rich_tx_by_txid(&txid3)?, None);
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid3_modified)?.as_ref(),
        Some(&rich_tx3)
    );

    let mut expected_txs = [rich_tx2, rich_tx3];
    expected_txs.sort_by_key(|tx| tx.txid.clone());
    assert_eq!(
        slp_indexer
            .blocks()
            .block_txs_by_hash(&block2.header.calc_hash())?[1..],
        expected_txs,
    );
    assert_eq!(
        slp_indexer
            .blocks()
            .block_txs_by_hash(&block2.header.calc_hash())?,
        slp_indexer.blocks().block_txs_by_height(112)?,
    );
    {
        use TxIdentifier::*;
        check_rich_utxos(
            slp_indexer,
            P2SH,
            anyone_slice,
            [
                (N(1), 1, true),
                (N(2), 1, true),
                (N(3), 1, true),
                (N(4), 1, true),
                (N(5), 1, true),
                (N(6), 1, true),
                (N(7), 1, true),
                (N(111), 1, true),
                (N(113), 1, true),
                (H(&txid3_modified), 1, false),
            ],
        )?;
        check_rich_utxos(
            slp_indexer,
            P2SH,
            anyone_slice,
            [
                (N(1), 1, true),
                (N(2), 1, true),
                (N(3), 1, true),
                (N(4), 1, true),
                (N(5), 1, true),
                (N(6), 1, true),
                (N(7), 1, true),
                (N(111), 1, true),
                (N(113), 1, true),
                (N(115), 1, false),
            ],
        )?;
    }
    assert_eq!(
        slp_indexer.utxos().utxo_state(&OutPoint {
            txid: txid3,
            out_idx: 1,
        })?,
        UtxoState {
            height: None,
            state: UtxoStateVariant::NoSuchTx,
        },
    );

    let block_stats_reader = slp_indexer.db().block_stats()?;
    // Check genesis stats
    assert_eq!(
        block_stats_reader.by_height(0)?,
        Some(BlockStats {
            block_size: 379,
            num_txs: 1,
            num_inputs: 1,
            num_outputs: 2,
            sum_input_sats: 0,
            sum_coinbase_output_sats: 260_000_000,
            sum_normal_output_sats: 0,
            sum_burned_sats: 130_000_000,
        })
    );
    // Check generated block stats
    for block_height in 1..=110 {
        assert_eq!(
            block_stats_reader.by_height(block_height)?,
            Some(BlockStats {
                block_size: 272
                    + match block_height {
                        1..=16 => 1,
                        17..=110 => 2,
                        _ => unreachable!(),
                    },
                num_txs: 1,
                num_inputs: 1,
                num_outputs: 2,
                sum_input_sats: 0,
                sum_coinbase_output_sats: 260_000_000,
                sum_normal_output_sats: 0,
                sum_burned_sats: 0,
            }),
            "different at height {}",
            block_height
        );
    }
    // Check manually mined block stats
    assert_eq!(
        block_stats_reader.by_height(111)?,
        Some(BlockStats {
            block_size: 460,
            num_txs: 2,
            num_inputs: 2,
            num_outputs: 4,
            sum_input_sats: 260_000_000,
            sum_coinbase_output_sats: 260_000_000,
            sum_normal_output_sats: 259_990_000,
            sum_burned_sats: 0,
        }),
    );
    assert_eq!(
        block_stats_reader.by_height(112)?,
        Some(BlockStats {
            block_size: 709,
            num_txs: 3,
            num_inputs: 5,
            num_outputs: 6,
            sum_input_sats: 1_039_960_000,
            sum_coinbase_output_sats: 260_000_000,
            sum_normal_output_sats: 1_039_919_999,
            sum_burned_sats: 0,
        }),
    );
    assert_eq!(block_stats_reader.by_height(113)?, None);

    Ok(())
}

enum TxIdentifier<'a> {
    N(u64),
    H(&'a Sha256d),
}

fn check_rev_history_pages<const M: usize>(
    slp_indexer: &SlpIndexer,
    prefix: PayloadPrefix,
    payload: &[u8],
    page_size: usize,
    tx_id_pages: [&[TxIdentifier]; M],
) -> Result<()> {
    let addrs = slp_indexer.script_history();
    assert_eq!(addrs.rev_history_num_pages(prefix, payload, page_size)?, M);
    let tx_reader = slp_indexer.db().txs()?;
    for (page_num, tx_ids) in tx_id_pages.into_iter().enumerate() {
        let actual_rich_txs = addrs.rev_history_page(prefix, payload, page_num, page_size)?;
        let expected_txids_and_heights = tx_ids
            .iter()
            .map(|id| match *id {
                TxIdentifier::N(tx_num) => {
                    let block_tx = tx_reader.by_tx_num(tx_num)?.unwrap();
                    Ok((Some(block_tx.block_height), block_tx.entry.txid))
                }
                TxIdentifier::H(txid) => {
                    let block_height = slp_indexer
                        .txs()
                        .rich_tx_by_txid(txid)?
                        .unwrap()
                        .block
                        .map(|block| block.height);
                    Ok((block_height, txid.clone()))
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let actual_txids_and_heights = actual_rich_txs
            .iter()
            .map(|tx| (tx.block.as_ref().map(|block| block.height), tx.txid.clone()))
            .collect::<Vec<_>>();
        assert_eq!(expected_txids_and_heights, actual_txids_and_heights);
        let expected_rich_txs = tx_ids
            .iter()
            .map(|id| {
                let txid = match *id {
                    TxIdentifier::N(tx_num) => tx_reader.txid_by_tx_num(tx_num)?.unwrap(),
                    TxIdentifier::H(txid) => txid.clone(),
                };
                Ok(slp_indexer.txs().rich_tx_by_txid(&txid)?.unwrap())
            })
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(actual_rich_txs, expected_rich_txs);
    }
    Ok(())
}

fn check_rich_utxos<const M: usize>(
    slp_indexer: &SlpIndexer,
    prefix: PayloadPrefix,
    payload: &[u8],
    outpoints: [(TxIdentifier, u32, bool); M],
) -> Result<()> {
    let tx_reader = slp_indexer.db().txs()?;
    let actual_outpoints = outpoints
        .into_iter()
        .map(|(tx_id, out_idx, is_coinbase)| {
            let txid = match tx_id {
                TxIdentifier::N(tx_num) => tx_reader.txid_by_tx_num(tx_num)?.unwrap(),
                TxIdentifier::H(txid) => txid.clone(),
            };
            let rich_tx = slp_indexer.txs().rich_tx_by_txid(&txid)?.unwrap();
            Ok(RichUtxo {
                outpoint: OutPoint { txid, out_idx },
                block: rich_tx.block,
                is_coinbase,
                output: rich_tx.tx.outputs()[out_idx as usize].clone(),
                slp_output: rich_tx.slp_tx_data.map(|slp_data| {
                    Box::new(SlpOutput {
                        token_id: slp_data.token_id,
                        tx_type: slp_data.slp_tx_type.tx_type_variant(),
                        token_type: slp_data.slp_token_type,
                        token: slp_data
                            .output_tokens
                            .get(out_idx as usize)
                            .cloned()
                            .unwrap_or_default(),
                        group_token_id: slp_data.group_token_id,
                    })
                }),
                time_first_seen: rich_tx.time_first_seen,
                network: rich_tx.network,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    assert_eq!(
        slp_indexer.utxos().utxos(&ScriptPayload {
            payload_prefix: prefix,
            payload_data: payload.to_vec(),
        })?,
        actual_outpoints,
    );
    Ok(())
}

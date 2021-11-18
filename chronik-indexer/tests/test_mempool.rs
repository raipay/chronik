use std::{ffi::OsString, str::FromStr, sync::Arc};

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
    genesis_opreturn, send_opreturn, RichTx, RichTxBlock, SlpAmount, SlpGenesisInfo, SlpToken,
    SlpTokenType, SlpTxData, SlpTxType, SlpValidTxData, TokenId,
};
use bitcoinsuite_test_utils::bin_folder;
use bitcoinsuite_test_utils_blockchain::build_tx;
use chronik_indexer::SlpIndexer;
use chronik_rocksdb::{Db, IndexDb, IndexMemData, MempoolTxEntry, PayloadPrefix};
use pretty_assertions::{assert_eq, assert_ne};
use tempdir::TempDir;

#[test]
fn test_mempool() -> Result<()> {
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
    let db = Db::open(dir.path().join("index.rocksdb"))?;
    let db = IndexDb::new(db);
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
    bitcoind.cmd_string("setmocktime", &["1600000000"])?;
    test_index_mempool(&mut slp_indexer, bitcoind)?;
    instance.cleanup()?;
    Ok(())
}

fn test_index_mempool(slp_indexer: &mut SlpIndexer, bitcoind: &BitcoinCli) -> Result<()> {
    let anyone_script = Script::from_slice(&[0x51]);
    let anyone_hash = ShaRmd160::digest(anyone_script.bytecode().clone());
    let anyone_address = CashAddress::from_hash(BCHREG, AddressType::P2SH, anyone_hash.clone());
    bitcoind.cmd_json("generatetoaddress", &["10", anyone_address.as_str()])?;
    let burn_address = CashAddress::from_hash(BCHREG, AddressType::P2SH, ShaRmd160::new([0; 20]));
    bitcoind.cmd_json("generatetoaddress", &["100", burn_address.as_str()])?;
    while !slp_indexer.catchup_step()? {}
    slp_indexer.leave_catchup()?;

    let utxo_entries = slp_indexer
        .db()
        .utxos()?
        .utxos(PayloadPrefix::P2SH, anyone_hash.as_slice())?;
    assert_eq!(utxo_entries.len(), 10);

    let mut utxos = Vec::new();
    for utxo_entry in utxo_entries {
        let tx_entry = slp_indexer
            .db()
            .txs()?
            .by_tx_num(utxo_entry.tx_num)?
            .unwrap();
        utxos.push((
            OutPoint {
                txid: tx_entry.entry.txid,
                out_idx: utxo_entry.out_idx,
            },
            260_000_000,
        ));
    }

    let (outpoint, value) = utxos.pop().unwrap();
    let leftover_value = value - 20_000;
    let tx1 = build_tx(
        outpoint,
        &anyone_script,
        vec![
            TxOutput {
                value: 10_000,
                script: burn_address.to_script(),
            },
            TxOutput {
                value: leftover_value,
                script: anyone_script.to_p2sh(),
            },
        ],
    );
    let txid_hex = bitcoind.cmd_string("sendrawtransaction", &[&tx1.ser().hex()])?;
    let txid1 = Sha256d::from_hex_be(&txid_hex)?;
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
            ..Default::default()
        }]),
        spends: vec![None, None],
        slp_burns: vec![None],
        slp_error_msg: None,
        time_first_seen: 1_600_000_000,
        network: Network::XPI,
    };
    slp_indexer.process_next_msg()?;
    assert_eq!(
        slp_indexer.db_mempool().tx(&txid1),
        Some(&MempoolTxEntry {
            tx: tx1.clone(),
            spent_outputs: vec![TxOutput {
                value,
                script: anyone_script.to_p2sh(),
            }],
            time_first_seen: 1_600_000_000,
        }),
    );
    assert_eq!(slp_indexer.db_mempool_slp().slp_tx_data(&txid1), None);
    assert_eq!(slp_indexer.db_mempool_slp().slp_tx_error(&txid1), None);
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid1)?,
        Some(rich_tx1.clone())
    );
    assert_eq!(slp_indexer.txs().raw_tx_by_id(&txid1)?, Some(tx1.ser()));

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
    let txid_hex = bitcoind.cmd_string("sendrawtransaction", &[&tx2.ser().hex()])?;
    let txid2 = Sha256d::from_hex_be(&txid_hex)?;
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
            ..Default::default()
        }]),
        spends: vec![None, None],
        slp_burns: vec![None],
        slp_error_msg: None,
        time_first_seen: 1_600_000_000,
        network: Network::XPI,
    };
    slp_indexer.process_next_msg()?;
    assert_eq!(
        slp_indexer.db_mempool().tx(&txid2),
        Some(&MempoolTxEntry {
            tx: tx2.clone(),
            spent_outputs: vec![TxOutput {
                value,
                script: anyone_script.to_p2sh(),
            }],
            time_first_seen: 1_600_000_000,
        }),
    );
    assert_eq!(
        slp_indexer.db_mempool_slp().slp_tx_data(&txid2),
        Some(&SlpValidTxData {
            slp_tx_data: SlpTxData {
                input_tokens: vec![SlpToken::EMPTY],
                output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(100)],
                slp_token_type: SlpTokenType::Fungible,
                slp_tx_type: SlpTxType::Genesis(SlpGenesisInfo::default().into()),
                token_id: token_id.clone(),
                group_token_id: None,
            },
            slp_burns: vec![None],
        })
    );
    assert_eq!(
        slp_indexer.db_mempool().tx(&txid2),
        Some(&MempoolTxEntry {
            tx: tx2.clone(),
            spent_outputs: vec![TxOutput {
                value,
                script: anyone_script.to_p2sh(),
            }],
            time_first_seen: 1_600_000_000,
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

    let (outpoint, value) = utxos.pop().unwrap();
    let send_value = leftover_value * 2 + value - 20_000;
    let mut tx3 = UnhashedTx {
        version: 1,
        inputs: vec![
            TxInput {
                prev_out: outpoint,
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
                script: burn_address.to_script(),
            },
        ],
        lock_time: 0,
    };
    let txid_hex = bitcoind.cmd_string("sendrawtransaction", &[&tx3.ser().hex()])?;
    let txid3 = Sha256d::from_hex_be(&txid_hex)?;
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
                ..Default::default()
            },
            Coin {
                tx_output: TxOutput {
                    value: leftover_value,
                    script: anyone_script.to_p2sh(),
                },
                ..Default::default()
            },
            Coin {
                tx_output: TxOutput {
                    value: leftover_value,
                    script: anyone_script.to_p2sh(),
                },
                ..Default::default()
            },
        ]),
        spends: vec![None, None],
        slp_burns: vec![None, None, None],
        slp_error_msg: None,
        time_first_seen: 1_600_000_000,
        network: Network::XPI,
    };
    slp_indexer.process_next_msg()?;
    assert_eq!(
        slp_indexer.db_mempool().tx(&txid3),
        Some(&MempoolTxEntry {
            tx: tx3.clone(),
            spent_outputs: vec![
                TxOutput {
                    value,
                    script: anyone_script.to_p2sh(),
                },
                TxOutput {
                    value: leftover_value,
                    script: anyone_script.to_p2sh(),
                },
                TxOutput {
                    value: leftover_value,
                    script: anyone_script.to_p2sh(),
                },
            ],
            time_first_seen: 1_600_000_000,
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

    let tip = slp_indexer.db().blocks()?.tip()?.unwrap();
    let tx1 = tx1.hashed();
    let block1 = build_lotus_block(
        tip.hash.clone(),
        tip.timestamp + 1,
        tip.height + 1,
        build_lotus_coinbase(tip.height + 1, anyone_script.to_p2sh()).hashed(),
        vec![tx1.clone()],
        Sha256d::default(),
        vec![],
    );
    let result = bitcoind.cmd_string("submitblock", &[&block1.ser().hex()])?;
    assert_eq!(result, "");

    slp_indexer.process_next_msg()?;
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

    // modify tx3
    tx3.outputs[1].value -= 1;

    let tip = slp_indexer.db().blocks()?.tip()?.unwrap();
    let tx2 = tx2.hashed();
    let tx3 = tx3.hashed();
    let block2 = build_lotus_block(
        tip.hash.clone(),
        tip.timestamp + 1,
        tip.height + 1,
        build_lotus_coinbase(tip.height + 1, anyone_script.to_p2sh()).hashed(),
        vec![tx2.clone(), tx3.clone()],
        Sha256d::default(),
        vec![],
    );
    let result = bitcoind.cmd_string("submitblock", &[&block2.ser().hex()])?;
    assert_eq!(result, "");

    // Remove tx3 from mempool
    slp_indexer.process_next_msg()?;
    // Process block
    slp_indexer.process_next_msg()?;

    assert_eq!(slp_indexer.db_mempool().tx(&txid1), None);
    assert_eq!(slp_indexer.db_mempool().tx(&txid2), None);
    assert_eq!(slp_indexer.db_mempool().tx(&txid3), None);

    let block_tx = slp_indexer.db().txs()?.by_txid(&txid2)?.unwrap();
    assert_eq!(block_tx.entry.txid, txid2);
    assert_eq!(block_tx.entry.tx_size, tx2.raw().len() as u32);
    assert_eq!(block_tx.block_height, 112);

    assert_eq!(slp_indexer.db().txs()?.by_txid(&txid3)?, None);

    let txid3_modified = lotus_txid(tx3.unhashed_tx());
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
    rich_tx2.spends[1] = Some(OutPoint {
        txid: txid3_modified.clone(),
        out_idx: 2,
    });
    rich_tx2.block = Some(RichTxBlock {
        height: 112,
        hash: block2.header.calc_hash(),
    });
    rich_tx2.spent_coins.as_mut().unwrap()[0].height = Some(9);
    rich_tx2.spent_coins.as_mut().unwrap()[0].is_coinbase = true;
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid2)?,
        Some(rich_tx2.clone())
    );
    rich_tx3.tx = tx3;
    rich_tx3.txid = txid3_modified.clone();
    rich_tx3.block = Some(RichTxBlock {
        height: 112,
        hash: block2.header.calc_hash(),
    });
    {
        let spent_coins = rich_tx3.spent_coins.as_mut().unwrap();
        spent_coins[0].height = Some(8);
        spent_coins[0].is_coinbase = true;
        spent_coins[1].height = Some(111);
        spent_coins[2].height = Some(112);
    }
    rich_tx3.time_first_seen = 0; // tx not first seen in mempool
    assert_eq!(slp_indexer.txs().rich_tx_by_txid(&txid3)?, None);
    assert_eq!(
        slp_indexer.txs().rich_tx_by_txid(&txid3_modified)?,
        Some(rich_tx3)
    );

    Ok(())
}

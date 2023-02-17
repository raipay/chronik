use std::{ffi::OsString, str::FromStr, sync::Arc};

use bitcoinsuite_bitcoind::{
    cli::BitcoinCli,
    instance::{BitcoindChain, BitcoindConf, BitcoindInstance},
};
use bitcoinsuite_bitcoind_nng::{PubInterface, RpcInterface};
use bitcoinsuite_core::{
    AddressType, CashAddress, Coin, Hashed, Network, OutPoint, Script, ShaRmd160, TxOutput, BCHREG,
};
use bitcoinsuite_ecc_secp256k1::EccSecp256k1;
use bitcoinsuite_error::Result;
use bitcoinsuite_slp::{
    genesis_opreturn, send_opreturn, RichTx, SlpAmount, SlpBurn, SlpError, SlpGenesisInfo,
    SlpToken, SlpTokenType, SlpTxData, SlpTxType, TokenId,
};
use bitcoinsuite_test_utils::bin_folder;
use bitcoinsuite_test_utils_blockchain::build_tx;
use chronik_indexer::{
    broadcast::{BroadcastError, SlpBurns},
    SlpIndexer,
};
use chronik_rocksdb::{Db, IndexDb, IndexMemData, PayloadPrefix, ScriptTxsConf, TransientData};
use pretty_assertions::assert_eq;
use tempdir::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slp() -> Result<()> {
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
    bitcoind.cmd_string("setmocktime", &["2100000000"])?;
    test_index_slp(&mut slp_indexer, bitcoind).await?;
    instance.cleanup()?;
    Ok(())
}

async fn test_index_slp(slp_indexer: &mut SlpIndexer, bitcoind: &BitcoinCli) -> Result<()> {
    use PayloadPrefix::P2SH;
    let anyone_script = Script::from_slice(&[0x51]);
    let anyone_hash = ShaRmd160::digest(anyone_script.bytecode().clone());
    let anyone_slice = anyone_hash.as_slice();
    let anyone_address = CashAddress::from_hash(BCHREG, AddressType::P2SH, anyone_hash.clone());

    bitcoind.cmd_json("generatetoaddress", &["10", anyone_address.as_str()])?;

    let burn_address = CashAddress::from_hash(BCHREG, AddressType::P2SH, ShaRmd160::new([0; 20]));
    bitcoind.cmd_json("generatetoaddress", &["100", burn_address.as_str()])?;
    while !slp_indexer.catchup_step().await? {}
    slp_indexer.leave_catchup()?;

    let utxo_entries = slp_indexer.db().utxos()?.utxos(P2SH, anyone_slice)?;
    assert_eq!(utxo_entries.len(), 10);

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

    let (outpoint, value) = utxos.pop().unwrap();
    let leftover_value = value - 20_000;

    // invalid GENESIS (invalid mint baton out idx)
    let tx = build_tx(
        outpoint.clone(),
        &anyone_script,
        vec![TxOutput {
            value: 0,
            script: genesis_opreturn(
                &SlpGenesisInfo::default(),
                SlpTokenType::Fungible,
                Some(1),
                200,
            ),
        }],
    );
    let slp_error = slp_indexer
        .broadcast()
        .broadcast_tx(&tx, true)
        .await
        .unwrap_err();
    let slp_error = slp_error.downcast::<BroadcastError>()?;
    assert_eq!(
        slp_error,
        BroadcastError::InvalidSlpTx(SlpError::InvalidMintBatonIdx { actual: 1 }),
    );

    let tx1 = build_tx(
        outpoint,
        &anyone_script,
        vec![
            TxOutput {
                value: 0,
                script: genesis_opreturn(
                    &SlpGenesisInfo::default(),
                    SlpTokenType::Fungible,
                    Some(2),
                    200,
                ),
            },
            TxOutput {
                value: leftover_value,
                script: anyone_script.to_p2sh(),
            },
            TxOutput {
                value: 10_000,
                script: anyone_script.to_p2sh(),
            },
        ],
    );
    let txid1 = slp_indexer.broadcast().broadcast_tx(&tx1, true).await?;
    let token_id1 = TokenId::new(txid1.clone());
    slp_indexer.process_next_msg()?;
    let rich_tx1 = RichTx {
        tx: tx1.hashed(),
        txid: txid1.clone(),
        block: None,
        slp_tx_data: Some(Box::new(SlpTxData {
            input_tokens: vec![SlpToken::EMPTY],
            output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(200), SlpToken::MINT_BATON],
            slp_token_type: SlpTokenType::Fungible,
            slp_tx_type: SlpTxType::Genesis(Default::default()),
            token_id: token_id1.clone(),
            group_token_id: None,
        })),
        spent_coins: Some(vec![Coin {
            tx_output: TxOutput {
                value,
                script: anyone_script.to_p2sh(),
            },
            height: Some(10),
            is_coinbase: true,
        }]),
        spends: vec![None, None, None],
        slp_burns: vec![None],
        slp_error_msg: None,
        time_first_seen: 2_100_000_000,
        network: Network::XPI,
    };
    assert_eq!(slp_indexer.txs().rich_tx_by_txid(&txid1)?, Some(rich_tx1.clone()));

    let (outpoint, value) = utxos.pop().unwrap();
    let leftover_value = value - 20_000;
    let tx2 = build_tx(
        outpoint,
        &anyone_script,
        vec![
            TxOutput {
                value: 0,
                script: genesis_opreturn(
                    &SlpGenesisInfo::default(),
                    SlpTokenType::Fungible,
                    Some(2),
                    300,
                ),
            },
            TxOutput {
                value: leftover_value,
                script: anyone_script.to_p2sh(),
            },
        ],
    );
    let txid2 = slp_indexer.broadcast().broadcast_tx(&tx2, true).await?;
    let token_id2 = TokenId::new(txid2.clone());
    slp_indexer.process_next_msg()?;

    let mut tx_burn = build_tx(
        OutPoint {
            txid: txid1.clone(),
            out_idx: 1,
        },
        &anyone_script,
        vec![TxOutput {
            value: 0,
            script: send_opreturn(&token_id1, SlpTokenType::Fungible, &[SlpAmount::new(170)]),
        }],
    );
    {
        let mut input = tx_burn.inputs[0].clone();
        let outpoint = utxos.pop().unwrap().0;
        input.prev_out = outpoint;
        tx_burn.inputs.push(input);
    }
    {
        let mut input = tx_burn.inputs[0].clone();
        input.prev_out.out_idx = 2;
        tx_burn.inputs.push(input);
    }
    {
        let mut input = tx_burn.inputs[0].clone();
        input.prev_out = OutPoint {
            txid: txid2,
            out_idx: 1,
        };
        tx_burn.inputs.push(input);
    }
    let slp_error = slp_indexer
        .broadcast()
        .broadcast_tx(&tx_burn, true)
        .await
        .unwrap_err();
    let slp_error = slp_error.downcast::<BroadcastError>()?;
    assert_eq!(
        slp_error,
        BroadcastError::InvalidSlpBurns(SlpBurns(vec![
            Some(Box::new(SlpBurn {
                token: SlpToken::amount(30),
                token_id: token_id1.clone(),
            })),
            None,
            Some(Box::new(SlpBurn {
                token: SlpToken::MINT_BATON,
                token_id: token_id1.clone(),
            })),
            Some(Box::new(SlpBurn {
                token: SlpToken::amount(300),
                token_id: token_id2.clone(),
            })),
        ])),
    );
    assert_eq!(
        slp_error.to_string(),
        format!(
            "Invalid SLP burns: \
            input at index 0 burns 30 base tokens of token ID {}, \
            input at index 2 burns mint baton of token ID {}, \
            input at index 3 burns 300 base tokens of token ID {}",
            token_id1.hash(),
            token_id1.hash(),
            token_id2.hash(),
        ),
    );

    let broadcast_error = slp_indexer
        .broadcast()
        .broadcast_tx(&tx_burn, false)
        .await
        .unwrap_err();
    let broadcast_error = broadcast_error.downcast::<BroadcastError>()?;
    assert_eq!(
        broadcast_error,
        BroadcastError::BitcoindRejectedTx(
            "Fee exceeds maximum configured by user (e.g. -maxtxfee, maxfeerate)".to_string()
        ),
    );

    // mine previous txs
    bitcoind.cmd_json("generatetoaddress", &["1", burn_address.as_str()])?;
    slp_indexer.process_next_msg()?;

    let recv1_redeem_script = Script::from_slice(&[0x52]);
    let recv1_hash = ShaRmd160::digest(recv1_redeem_script.bytecode().clone());
    let recv1_script = Script::p2sh(&recv1_hash);

    let tx3 = build_tx(
        OutPoint {
            txid: txid1,
            out_idx: 1,
        },
        &anyone_script,
        vec![
            TxOutput {
                value: 0,
                script: send_opreturn(&token_id1, SlpTokenType::Fungible, &[SlpAmount::new(200)]),
            },
            TxOutput {
                value: leftover_value - 20_000,
                script: recv1_script,
            },
        ],
    );

    let txid3 = slp_indexer.broadcast().broadcast_tx(&tx3, true).await?;
    slp_indexer.process_next_msg()?;

    let rich_tx3 = RichTx {
        tx: tx3.hashed(),
        txid: txid3.clone(),
        block: None,
        slp_tx_data: Some(Box::new(SlpTxData {
            input_tokens: vec![SlpToken::amount(200)],
            output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(200)],
            slp_token_type: SlpTokenType::Fungible,
            slp_tx_type: SlpTxType::Send,
            token_id: token_id1.clone(),
            group_token_id: None,
        })),
        spent_coins: Some(vec![Coin {
            tx_output: TxOutput {
                value: leftover_value,
                script: anyone_script.to_p2sh(),
            },
            height: Some(111),
            is_coinbase: false,
        }]),
        spends: vec![None, None],
        slp_burns: vec![None],
        slp_error_msg: None,
        time_first_seen: 2_100_000_000,
        network: Network::XPI,
    };
    assert_eq!(slp_indexer.txs().rich_tx_by_txid(&txid3)?, Some(rich_tx3));

    // Mine tx3
    let block_hashes = bitcoind.cmd_json("generatetoaddress", &["1", burn_address.as_str()])?;
    slp_indexer.process_next_msg()?;

    // Invalidate last block
    let block_hash = block_hashes[0].as_str().unwrap();
    bitcoind.cmd_string("invalidateblock", &[block_hash])?;
    slp_indexer.process_next_msg()?;

    // tx3 is gone now
    assert_eq!(slp_indexer.txs().rich_tx_by_txid(&txid3)?, None);

    // token1 is still valid
    assert_eq!(slp_indexer.txs().rich_tx_by_txid(token_id1.hash())?, Some(rich_tx1));

    Ok(())
}

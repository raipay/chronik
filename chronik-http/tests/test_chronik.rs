use std::{ffi::OsString, str::FromStr, sync::Arc, time::Duration};

use bitcoinsuite_bitcoind::instance::{BitcoindChain, BitcoindConf, BitcoindInstance};
use bitcoinsuite_bitcoind_nng::{PubInterface, RpcInterface};
use bitcoinsuite_core::{
    lotus_txid, AddressType, BitcoinCode, Bytes, CashAddress, Hashed, Network, Script, Sha256d,
    ShaRmd160, TxOutput, BCHREG,
};
use bitcoinsuite_ecc_secp256k1::EccSecp256k1;
use bitcoinsuite_error::Result;
use bitcoinsuite_slp::{genesis_opreturn, SlpGenesisInfo, SlpTokenType};
use bitcoinsuite_test_utils::{bin_folder, is_free_tcp, pick_ports};
use bitcoinsuite_test_utils_blockchain::build_tx;
use chronik_http::{proto, ChronikServer, CONTENT_TYPE_PROTOBUF};
use chronik_indexer::SlpIndexer;
use chronik_rocksdb::{
    Db, IndexDb, IndexMemData, PayloadPrefix, ScriptPayload, ScriptTxsConf, TransientData,
};
use futures::{SinkExt, StreamExt};
use hyper::{header::CONTENT_TYPE, StatusCode};
use pretty_assertions::assert_eq;
use prost::Message;
use reqwest::Response;
use tempdir::TempDir;
use tokio::{sync::RwLock, time::timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_server() -> Result<()> {
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
    let transient_data = TransientData::open(&dir.path().join("transient.rocksdb"))?;
    let db = IndexDb::new(db, transient_data, outputs_conf);
    let bitcoind = instance.cli();
    let cache = IndexMemData::new(10);
    let slp_indexer = SlpIndexer::new(
        db,
        instance.rpc_client().clone(),
        rpc_interface,
        pub_interface,
        cache,
        Network::XPI,
        Arc::new(EccSecp256k1::default()),
    )?;
    bitcoind.cmd_string("setmocktime", &["2100000000"])?;

    let slp_indexer = Arc::new(RwLock::new(slp_indexer));
    let port = pick_ports(1)?[0];
    let server = ChronikServer {
        addr: ([127, 0, 0, 1], port).into(),
        slp_indexer: Arc::clone(&slp_indexer),
    };
    tokio::spawn(server.run());
    let mut attempt = 0i32;
    while is_free_tcp(port) {
        if attempt == 100 {
            panic!("Unable to start Chronik server");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        attempt += 1;
    }
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}", port);
    let ws_url = format!("ws://127.0.0.1:{}", port);

    let response = client
        .get(format!("{}/blockchain-info", url))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], CONTENT_TYPE_PROTOBUF);
    assert_eq!(
        proto::BlockchainInfo::decode(response.bytes().await?)?,
        proto::BlockchainInfo {
            tip_hash: vec![0; 32],
            tip_height: -1,
        }
    );

    let anyone1_script = Script::from_slice(&[0x51]);
    let anyone1_hash = ShaRmd160::digest(&anyone1_script.bytecode());
    let anyone1_slice = anyone1_hash.as_slice();
    let anyone1_address = CashAddress::from_hash(BCHREG, AddressType::P2SH, anyone1_hash.clone());
    bitcoind.cmd_json("generatetoaddress", &["10", anyone1_address.as_str()])?;
    let burn_address = CashAddress::from_hash(BCHREG, AddressType::P2SH, ShaRmd160::new([0; 20]));
    bitcoind.cmd_json("generatetoaddress", &["100", burn_address.as_str()])?;

    while !slp_indexer.write().await.catchup_step().await? {}
    slp_indexer.write().await.leave_catchup()?;

    let mut utxos = slp_indexer.read().await.utxos().utxos(&ScriptPayload {
        payload_prefix: PayloadPrefix::P2SH,
        payload_data: anyone1_slice.to_vec(),
    })?;
    assert_eq!(utxos.len(), 10);

    let anyone2_script = Script::from_slice(&[0x52]);
    let anyone2_hash = ShaRmd160::digest(&anyone2_script.bytecode());
    let anyone2_slice = anyone2_hash.as_slice();
    let anyone2_address = CashAddress::from_hash(BCHREG, AddressType::P2SH, anyone2_hash.clone());

    let (mut ws_client, response) = connect_async(format!("{}/ws", ws_url)).await?;
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    ws_client
        .send(WsMessage::binary(
            proto::Subscription {
                script_type: "p2sh".to_string(),
                payload: anyone1_slice.to_vec(),
                is_subscribe: true,
            }
            .encode_to_vec(),
        ))
        .await?;

    let utxo = utxos.pop().unwrap();
    let leftover_value = utxo.output.value - 20_000;
    let tx = build_tx(
        utxo.outpoint.clone(),
        &anyone1_script,
        vec![
            TxOutput {
                value: 10_000,
                script: burn_address.to_script(),
            },
            TxOutput {
                value: leftover_value,
                script: anyone2_script.to_p2sh(),
            },
        ],
    );

    let response = client
        .post(format!("{}/broadcast-tx", url))
        .header(CONTENT_TYPE, CONTENT_TYPE_PROTOBUF)
        .body(
            proto::BroadcastTxRequest {
                raw_tx: tx.ser().to_vec(),
                skip_slp_check: false,
            }
            .encode_to_vec(),
        )
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], CONTENT_TYPE_PROTOBUF);
    let proto_tx_response = proto::BroadcastTxResponse::decode(response.bytes().await?)?;
    let txid = Sha256d::from_slice(&proto_tx_response.txid)?;
    slp_indexer.write().await.process_next_msg()?;

    // msg from ws (within 50ms)
    let msg = timeout(Duration::from_millis(50), ws_client.next())
        .await?
        .unwrap()?;
    let msg = msg.into_data();
    let msg = proto::SubscribeMsg::decode(msg.as_slice())?;
    match msg.msg_type.unwrap() {
        proto::subscribe_msg::MsgType::AddedToMempool(added_to_mempool) => {
            assert_eq!(added_to_mempool.txid, txid.as_slice());
        }
        msg => panic!("Unexpected message: {:?}", msg),
    }

    let response = client.get(format!("{}/tx/ab", url)).send().await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    check_proto_error(
        response,
        "invalid-field",
        "Invalid txid: Invalid size: expected 32, got 1",
        true,
    )
    .await?;

    let response = client.get(format!("{}/raw-tx/ab", url)).send().await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    check_proto_error(
        response,
        "invalid-field",
        "Invalid txid: Invalid size: expected 32, got 1",
        true,
    )
    .await?;

    let response = client.get(format!("{}/tx/{}", url, txid)).send().await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], CONTENT_TYPE_PROTOBUF);
    let proto_tx = proto::Tx::decode(response.bytes().await?)?;
    let mut expected_tx = proto::Tx {
        txid: txid.as_slice().to_vec(),
        version: tx.version,
        inputs: vec![proto::TxInput {
            prev_out: Some(proto::OutPoint {
                txid: tx.inputs[0].prev_out.txid.as_slice().to_vec(),
                out_idx: tx.inputs[0].prev_out.out_idx,
            }),
            input_script: tx.inputs[0].script.bytecode().to_vec(),
            output_script: anyone1_address.to_script().bytecode().to_vec(),
            value: utxo.output.value,
            sequence_no: tx.inputs[0].sequence.as_u32(),
            slp_burn: None,
            slp_token: None,
        }],
        outputs: vec![
            proto::TxOutput {
                value: 10_000,
                output_script: burn_address.to_script().bytecode().to_vec(),
                slp_token: None,
                spent_by: None,
            },
            proto::TxOutput {
                value: leftover_value,
                output_script: anyone2_address.to_script().bytecode().to_vec(),
                slp_token: None,
                spent_by: None,
            },
        ],
        lock_time: tx.lock_time,
        slp_tx_data: None,
        slp_error_msg: "".to_string(),
        block: None,
        time_first_seen: 2_100_000_000,
        size: 117,
        is_coinbase: false,
        network: proto::Network::Xpi as i32,
    };

    assert_eq!(proto_tx, expected_tx.clone());

    let coinbase_utxo = utxos.pop().unwrap();
    let response = client
        .get(format!("{}/tx/{}", url, coinbase_utxo.outpoint.txid))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let proto_tx = proto::Tx::decode(response.bytes().await?)?;
    assert!(proto_tx.is_coinbase);

    let response = client
        .get(format!("{}/raw-tx/{}", url, txid))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "application/octet-stream");
    let raw_tx = response.bytes().await?;
    assert_eq!(Bytes::from_bytes(raw_tx), tx.ser());

    let response = client
        .get(format!(
            "{}/script/bork/{}/history",
            url,
            hex::encode(anyone2_slice),
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    check_proto_error(response, "invalid-field", "Invalid script_type: bork", true).await?;

    let response = client
        .get(format!(
            "{}/script/p2sh/{}/history",
            url,
            hex::encode(&[0; 19]),
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    check_proto_error(
        response,
        "invalid-script-payload-length",
        "Invalid script payload: length expected to be one of [20], got 19",
        false,
    )
    .await?;

    let response = client
        .get(format!(
            "{}/script/p2sh/{}/history?page=ab&page_num=10",
            url,
            hex::encode(anyone2_slice),
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    check_proto_error(response, "invalid-field", "Invalid page: ab", true).await?;

    let response = client
        .get(format!(
            "{}/script/p2sh/{}/history?page=0&page_size=cd",
            url,
            hex::encode(anyone2_slice),
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    check_proto_error(response, "invalid-field", "Invalid page_size: cd", true).await?;

    let response = client
        .get(format!(
            "{}/script/p2sh/{}/history?page=0&page_size=10",
            url,
            hex::encode(anyone2_slice),
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], CONTENT_TYPE_PROTOBUF);
    let proto_page = proto::TxHistoryPage::decode(response.bytes().await?)?;
    assert_eq!(proto_page.txs, vec![expected_tx.clone()]);

    let response = client
        .get(format!(
            "{}/script/p2sh/{}/utxos",
            url,
            hex::encode(anyone2_slice),
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], CONTENT_TYPE_PROTOBUF);
    let proto_utxos = proto::Utxos::decode(response.bytes().await?)?;
    assert_eq!(
        proto_utxos,
        proto::Utxos {
            script_utxos: vec![proto::ScriptUtxos {
                output_script: anyone2_address.to_script().bytecode().to_vec(),
                utxos: vec![proto::Utxo {
                    outpoint: Some(proto::OutPoint {
                        txid: txid.as_slice().to_vec(),
                        out_idx: 1,
                    }),
                    block_height: -1,
                    is_coinbase: false,
                    value: leftover_value,
                    slp_meta: None,
                    slp_token: None,
                    network: proto::Network::Xpi as i32,
                }],
            }],
        }
    );

    let response = client
        .post(format!("{}/validate-utxos", url))
        .header(CONTENT_TYPE, CONTENT_TYPE_PROTOBUF)
        .body(
            proto::ValidateUtxoRequest {
                outpoints: vec![
                    proto::OutPoint {
                        txid: utxo.outpoint.txid.as_slice().to_vec(),
                        out_idx: utxo.outpoint.out_idx,
                    },
                    proto::OutPoint {
                        txid: txid.as_slice().to_vec(),
                        out_idx: 1,
                    },
                    proto::OutPoint {
                        txid: txid.as_slice().to_vec(),
                        out_idx: 2,
                    },
                    proto::OutPoint {
                        txid: vec![3; 32],
                        out_idx: 0,
                    },
                ],
            }
            .encode_to_vec(),
        )
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], CONTENT_TYPE_PROTOBUF);
    let proto_utxos = proto::ValidateUtxoResponse::decode(response.bytes().await?)?;
    assert_eq!(
        proto_utxos,
        proto::ValidateUtxoResponse {
            utxo_states: vec![
                proto::UtxoState {
                    height: 10,
                    is_confirmed: true,
                    state: proto::UtxoStateVariant::Spent as i32,
                },
                proto::UtxoState {
                    height: -1,
                    is_confirmed: false,
                    state: proto::UtxoStateVariant::Unspent as i32,
                },
                proto::UtxoState {
                    height: -1,
                    is_confirmed: false,
                    state: proto::UtxoStateVariant::NoSuchOutput as i32,
                },
                proto::UtxoState {
                    height: -1,
                    is_confirmed: false,
                    state: proto::UtxoStateVariant::NoSuchTx as i32,
                }
            ],
        }
    );

    let hashes = bitcoind.cmd_json("generatetoaddress", &["1", burn_address.as_str()])?;
    slp_indexer.write().await.process_next_msg()?;

    let mut n_attempt = 0;
    loop {
        n_attempt += 1;
        if n_attempt > 100 {
            panic!("Too many attempts");
        }
        // msg from ws (within 50ms)
        let msg = timeout(Duration::from_millis(50), ws_client.next())
            .await?
            .unwrap()?;
        let msg = msg.into_data();
        let msg = proto::SubscribeMsg::decode(msg.as_slice())?;
        match msg.msg_type.unwrap() {
            proto::subscribe_msg::MsgType::BlockConnected(block_connected) => {
                assert_eq!(
                    Sha256d::from_slice(&block_connected.block_hash)?,
                    Sha256d::from_hex_be(hashes[0].as_str().unwrap())?,
                );
                break;
            }
            proto::subscribe_msg::MsgType::Confirmed(_) => {}
            msg => panic!("Unexpected message: {:?}", msg),
        }
    }

    for (path, error_code, msg) in [
        ("/blocks/-1/10", "invalid-field", "Invalid start_height: -1"),
        ("/blocks/10/-1", "invalid-field", "Invalid end_height: -1"),
        ("/blocks/10/5", "invalid-field", "Invalid end_height: 5"),
        (
            "/blocks/10/510",
            "page-size-too-large",
            "Page size too large",
        ),
    ] {
        let response = client.get(format!("{}{}", url, path)).send().await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        check_proto_error(response, error_code, msg, true).await?;
    }

    let response = client.get(format!("{}/blocks/0/200", url)).send().await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], CONTENT_TYPE_PROTOBUF);
    let proto_blocks = proto::Blocks::decode(response.bytes().await?)?;
    assert_eq!(proto_blocks.blocks.len(), 112);
    {
        let mut prev_hash = Sha256d::from_hex_be(&bitcoind.cmd_string("getblockhash", &["0"])?)?;
        assert_eq!(
            proto_blocks.blocks[0],
            proto::BlockInfo {
                hash: prev_hash.as_slice().to_vec(),
                prev_hash: vec![0; 32],
                height: 0,
                n_bits: 0x207fffff,
                timestamp: 1_600_000_000,
                block_size: 379,
                num_txs: 1,
                num_inputs: 1,
                num_outputs: 2,
                sum_input_sats: 0,
                sum_coinbase_output_sats: 260_000_000,
                sum_normal_output_sats: 0,
                sum_burned_sats: 130_000_000,
            }
        );
        for block_height in 1..=110 {
            let cur_hash = Sha256d::from_hex_be(
                &bitcoind.cmd_string("getblockhash", &[&block_height.to_string()])?,
            )?;
            assert!(proto_blocks.blocks[block_height].timestamp >= 2_100_000_000);
            assert_eq!(
                proto_blocks.blocks[block_height],
                proto::BlockInfo {
                    hash: cur_hash.as_slice().to_vec(),
                    prev_hash: prev_hash.as_slice().to_vec(),
                    height: block_height as i32,
                    n_bits: 0x207fffff,
                    timestamp: proto_blocks.blocks[block_height].timestamp,
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
                },
            );
            prev_hash = cur_hash;
        }
        let cur_hash = Sha256d::from_hex_be(&bitcoind.cmd_string("getblockhash", &["111"])?)?;
        let block_info = proto::BlockInfo {
            hash: cur_hash.as_slice().to_vec(),
            prev_hash: prev_hash.as_slice().to_vec(),
            height: 111,
            n_bits: 0x207fffff,
            timestamp: 2100000020,
            block_size: 391,
            num_txs: 2,
            num_inputs: 2,
            num_outputs: 4,
            sum_input_sats: 260_000_000,
            sum_coinbase_output_sats: 260_005_000,
            sum_normal_output_sats: 259990000,
            sum_burned_sats: 0,
        };
        assert_eq!(proto_blocks.blocks[111], block_info);

        let response = client
            .get(format!("{}/block/{}", url, cur_hash))
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], CONTENT_TYPE_PROTOBUF);
        let proto_block = proto::Block::decode(response.bytes().await?)?;
        expected_tx.block = Some(proto::BlockMetadata {
            height: 111,
            hash: cur_hash.as_slice().to_vec(),
            timestamp: 2100000020,
        });
        let block_details = proto::BlockDetails {
            version: 1,
            merkle_root: Sha256d::from_hex_be(
                "824e3cc681067cb6fff43cec4281021906c37312ae19c019d71dbffd18f5fc14",
            )?
            .as_slice()
            .to_vec(),
            nonce: 0,
            median_timestamp: 2100000019,
        };
        let raw_header = hex::decode(
            "d72912f6f5e54f2ef8d4bdf97fdab91f23457997a996aa3dff0a73edf1f8ca58ffff7f2014752b7d000\
             000000000000000000000\
             01870100000000006f0000000000000000000000000000000000000000000000000000000000000000000000\
             14fcf518fdbf1dd719c019ae1273c30619028142ec3cf4ffb67c0681c63c4e82\
             1406e05881e299367766d313e26c05564ec91bf721d31726bd6e46e60689539a"
        )?;
        assert_eq!(
            proto_block,
            proto::Block {
                block_info: Some(block_info),
                block_details: Some(block_details),
                raw_header,
                txs: vec![proto_block.txs[0].clone(), expected_tx],
            }
        );

        let response = client
            .get(format!("{}/blockchain-info", url))
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], CONTENT_TYPE_PROTOBUF);
        assert_eq!(
            proto::BlockchainInfo::decode(response.bytes().await?)?,
            proto::BlockchainInfo {
                tip_hash: cur_hash.as_slice().to_vec(),
                tip_height: 111,
            }
        );
    }

    let response = client.get(format!("{}/blocks/10/20", url)).send().await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], CONTENT_TYPE_PROTOBUF);
    let proto_blocks_smaller = proto::Blocks::decode(response.bytes().await?)?;
    assert_eq!(proto_blocks_smaller.blocks, proto_blocks.blocks[10..=20]);

    // Test atomic multi-tx broadcast
    let utxo = utxos.pop().unwrap();
    let leftover_value = utxo.output.value - 20_000;
    let tx1 = build_tx(
        utxo.outpoint.clone(),
        &anyone1_script,
        vec![
            TxOutput {
                value: 6_000,
                script: burn_address.to_script(),
            },
            TxOutput {
                value: leftover_value,
                script: anyone2_script.to_p2sh(),
            },
        ],
    );
    let utxo = utxos.pop().unwrap();
    let leftover_value = utxo.output.value - 20_000;
    let mut tx2 = build_tx(
        utxo.outpoint.clone(),
        &anyone1_script,
        vec![
            TxOutput {
                value: 25_000,
                script: burn_address.to_script(),
            },
            TxOutput {
                value: leftover_value,
                script: anyone2_script.to_p2sh(),
            },
        ],
    );
    let response = client
        .post(format!("{}{}", url, "/broadcast-txs"))
        .header(CONTENT_TYPE, CONTENT_TYPE_PROTOBUF)
        .body(
            proto::BroadcastTxsRequest {
                raw_txs: vec![tx1.ser().to_vec(), tx2.ser().to_vec()],
                skip_slp_check: false,
            }
            .encode_to_vec(),
        )
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    check_proto_error(
        response,
        "bitcoind-rejected-tx",
        "Bitcoind rejected tx: bad-txns-in-belowout",
        true,
    )
    .await?;
    tx2.outputs[0].value = 10_000;
    let response = client
        .post(format!("{}{}", url, "/broadcast-txs"))
        .header(CONTENT_TYPE, CONTENT_TYPE_PROTOBUF)
        .body(
            proto::BroadcastTxsRequest {
                raw_txs: vec![tx1.ser().to_vec(), tx2.ser().to_vec()],
                skip_slp_check: false,
            }
            .encode_to_vec(),
        )
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], CONTENT_TYPE_PROTOBUF);
    assert_eq!(
        proto::BroadcastTxsResponse::decode(response.bytes().await?)?,
        proto::BroadcastTxsResponse {
            txids: vec![
                lotus_txid(&tx1).as_slice().to_vec(),
                lotus_txid(&tx2).as_slice().to_vec(),
            ],
        },
    );
    slp_indexer.write().await.process_next_msg()?;
    slp_indexer.write().await.process_next_msg()?;

    {
        // Test SLP
        let utxo = utxos.pop().unwrap();
        let leftover_value = utxo.output.value - 10_000;
        let genesis_info = SlpGenesisInfo {
            token_ticker: b"HTW".as_slice().into(),
            token_name: b"Hello token world".as_slice().into(),
            token_document_url: b"https://htw.io".as_slice().into(),
            token_document_hash: Some([4; 32].into()),
            decimals: 4,
        };
        let tx = build_tx(
            utxo.outpoint,
            &anyone1_script,
            vec![
                TxOutput {
                    value: 0,
                    script: genesis_opreturn(&genesis_info, SlpTokenType::Fungible, Some(2), 1234),
                },
                TxOutput {
                    value: leftover_value,
                    script: anyone2_script.to_p2sh(),
                },
            ],
        );
        let response = client
            .post(format!("{}/broadcast-tx", url))
            .header(CONTENT_TYPE, CONTENT_TYPE_PROTOBUF)
            .body(
                proto::BroadcastTxRequest {
                    raw_tx: tx.ser().to_vec(),
                    skip_slp_check: false,
                }
                .encode_to_vec(),
            )
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let response = proto::BroadcastTxResponse::decode(response.bytes().await?)?;
        let txid = Sha256d::from_slice(&response.txid)?;
        slp_indexer.write().await.process_next_msg()?;

        let response = client.get(format!("{}/token/{}", url, txid)).send().await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            proto::Token::decode(response.bytes().await?)?,
            proto::Token {
                slp_tx_data: Some(proto::SlpTxData {
                    slp_meta: Some(proto::SlpMeta {
                        token_type: proto::SlpTokenType::Fungible as i32,
                        tx_type: proto::SlpTxType::Genesis as i32,
                        token_id: txid.to_vec_be(),
                        group_token_id: vec![],
                    }),
                    genesis_info: Some(proto::SlpGenesisInfo {
                        token_ticker: genesis_info.token_ticker.to_vec(),
                        token_name: genesis_info.token_name.to_vec(),
                        token_document_url: genesis_info.token_document_url.to_vec(),
                        token_document_hash: genesis_info
                            .token_document_hash
                            .map(|hash| hash.to_vec())
                            .unwrap(),
                        decimals: genesis_info.decimals,
                    }),
                }),
                token_stats: Some(proto::TokenStats {
                    total_minted: "1234".to_string(),
                    total_burned: "0".to_string(),
                }),
                block: None,
                time_first_seen: 2_100_000_000,
                initial_token_quantity: 1234,
                contains_baton: false,
                network: proto::Network::Xpi.into(),
            },
        );
    }

    instance.cleanup()?;

    Ok(())
}

async fn check_proto_error(
    response: Response,
    error_code: &str,
    msg: &str,
    is_user_error: bool,
) -> Result<()> {
    assert_eq!(response.headers()[CONTENT_TYPE], CONTENT_TYPE_PROTOBUF);
    let mut body = response.bytes().await?;
    let actual_error = proto::Error::decode(&mut body)?;
    let expected_error = proto::Error {
        error_code: error_code.to_string(),
        msg: msg.to_string(),
        is_user_error,
    };
    assert_eq!(actual_error, expected_error);
    Ok(())
}

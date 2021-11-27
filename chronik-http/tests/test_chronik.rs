use std::{ffi::OsString, str::FromStr, sync::Arc, time::Duration};

use bitcoinsuite_bitcoind::instance::{BitcoindChain, BitcoindConf, BitcoindInstance};
use bitcoinsuite_bitcoind_nng::{PubInterface, RpcInterface};
use bitcoinsuite_core::{
    AddressType, BitcoinCode, CashAddress, Hashed, Network, Script, Sha256d, ShaRmd160, TxOutput,
    BCHREG,
};
use bitcoinsuite_ecc_secp256k1::EccSecp256k1;
use bitcoinsuite_error::Result;
use bitcoinsuite_test_utils::{bin_folder, is_free_tcp, pick_ports};
use bitcoinsuite_test_utils_blockchain::build_tx;
use chronik_http::{proto, ChronikServer, CONTENT_TYPE_PROTOBUF};
use chronik_indexer::SlpIndexer;
use chronik_rocksdb::{Db, IndexDb, IndexMemData, PayloadPrefix, ScriptPayload, ScriptTxsConf};
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
    bitcoind.cmd_string("setmocktime", &["2100000000"])?;

    let anyone1_script = Script::from_slice(&[0x51]);
    let anyone1_hash = ShaRmd160::digest(anyone1_script.bytecode().clone());
    let anyone1_slice = anyone1_hash.as_slice();
    let anyone1_address = CashAddress::from_hash(BCHREG, AddressType::P2SH, anyone1_hash.clone());
    bitcoind.cmd_json("generatetoaddress", &["10", anyone1_address.as_str()])?;
    let burn_address = CashAddress::from_hash(BCHREG, AddressType::P2SH, ShaRmd160::new([0; 20]));
    bitcoind.cmd_json("generatetoaddress", &["100", burn_address.as_str()])?;

    while !slp_indexer.catchup_step()? {}
    slp_indexer.leave_catchup()?;

    let mut utxos = slp_indexer.utxos().utxos(&ScriptPayload {
        payload_prefix: PayloadPrefix::P2SH,
        payload_data: anyone1_slice.to_vec(),
    })?;
    assert_eq!(utxos.len(), 10);

    let anyone2_script = Script::from_slice(&[0x52]);
    let anyone2_hash = ShaRmd160::digest(anyone2_script.bytecode().clone());
    let anyone2_slice = anyone2_hash.as_slice();
    let anyone2_address = CashAddress::from_hash(BCHREG, AddressType::P2SH, anyone2_hash.clone());

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
    let txid_hex = bitcoind.cmd_string("sendrawtransaction", &[&tx.ser().hex()])?;
    let txid = Sha256d::from_hex_be(&txid_hex)?;
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
        _ => panic!("Unexpected message"),
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

    let response = client
        .get(format!("{}/tx/{}", url, txid_hex))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], CONTENT_TYPE_PROTOBUF);
    let proto_tx = proto::Tx::decode(response.bytes().await?)?;
    let expected_tx = proto::Tx {
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
        network: proto::Network::Xpi as i32,
    };

    assert_eq!(proto_tx, expected_tx.clone());

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
    assert_eq!(proto_page.txs, vec![expected_tx]);

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
                    height: 0,
                    is_confirmed: false,
                    state: proto::UtxoStateVariant::Unspent as i32,
                },
                proto::UtxoState {
                    height: 0,
                    is_confirmed: false,
                    state: proto::UtxoStateVariant::NoSuchOutput as i32,
                },
                proto::UtxoState {
                    height: 0,
                    is_confirmed: false,
                    state: proto::UtxoStateVariant::NoSuchTx as i32,
                }
            ],
        }
    );

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

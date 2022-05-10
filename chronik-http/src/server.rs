use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    extract::{
        ws::{self, WebSocket, WebSocketUpgrade},
        Extension, Path, Query,
    },
    response::IntoResponse,
    routing::{self, MethodFilter},
    AddExtensionLayer, Router,
};
use bitcoinsuite_core::{BitcoinCode, BitcoinSuiteError, Hashed, OutPoint, Sha256d, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Report, WrapErr};
use bitcoinsuite_slp::{SlpTokenType, SlpTxTypeVariant, TokenId};
use chronik_indexer::{
    subscribers::{SubscribeBlockMessage, SubscribeScriptMessage},
    SlpIndexer, UtxoStateVariant,
};
use chronik_rocksdb::ScriptPayload;
use futures::future::select_all;
use itertools::Itertools;
use prost::Message;
use thiserror::Error;
use tokio::sync::{broadcast, RwLock};
use tower_http::compression::CompressionLayer;

pub const DEFAULT_PAGE_SIZE: usize = 25;
pub const MAX_HISTORY_PAGE_SIZE: usize = 200;
pub const MAX_BLOCKS_PAGE_SIZE: usize = 500;

pub type SlpIndexerRef = Arc<RwLock<SlpIndexer>>;

#[derive(Clone)]
pub struct ChronikServer {
    pub addr: SocketAddr,
    pub slp_indexer: SlpIndexerRef,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum ChronikServerError {
    #[not_found()]
    #[error("Txid not found: {0}")]
    TxNotFound(Sha256d),

    #[not_found()]
    #[error("Block not found: {0}")]
    BlockNotFound(String),

    #[not_found()]
    #[error("Token txid not found: {0}")]
    TokenTxidNotFound(Sha256d),

    #[not_found()]
    #[error("Token txid is not a GENESIS tx: {0}")]
    TokenTxNotGenesis(Sha256d),

    #[invalid_user_input()]
    #[error("Invalid hash or height: {0}")]
    InvalidHashOrHeight(String),

    #[invalid_user_input()]
    #[error("Invalid {name}: {value}")]
    InvalidField { name: &'static str, value: String },

    #[invalid_client_input()]
    #[error("Unexpected message type {0}")]
    UnexpectedMessageType(&'static str),

    #[invalid_user_input()]
    #[error("Page size too large")]
    PageSizeTooLarge,

    #[invalid_user_input()]
    #[error("Invalid tx encoding: {0}")]
    InvalidTxEncoding(BitcoinSuiteError),

    #[critical()]
    #[error("Unexpected JSON from bitcoind: {0}")]
    BitcoindBadJson(&'static str),
}

use crate::{
    convert::{
        block_to_info_proto, network_to_proto, parse_payload_prefix, rich_tx_to_proto,
        slp_token_to_proto, slp_tx_data_to_proto,
    },
    error::{report_to_status_proto, ReportError},
    proto,
    protobuf::Protobuf,
};

use self::ChronikServerError::*;

impl ChronikServer {
    pub async fn run(self) -> Result<(), Report> {
        let addr = self.addr;
        let app = Router::new()
            .route(
                "/broadcast-tx",
                routing::post(handle_broadcast_tx).on(MethodFilter::OPTIONS, handle_post_options),
            )
            .route(
                "/broadcast-txs",
                routing::post(handle_broadcast_txs).on(MethodFilter::OPTIONS, handle_post_options),
            )
            .route("/blocks/:start/:end", routing::get(handle_blocks))
            .route("/block/:hash_or_height", routing::get(handle_block))
            .route("/tx/:txid", routing::get(handle_tx))
            .route("/token/:token_id", routing::get(handle_token))
            .route(
                "/script/:type/:payload/history",
                routing::get(handle_script_history),
            )
            .route(
                "/script/:type/:payload/utxos",
                routing::get(handle_script_utxos),
            )
            .route(
                "/validate-utxos",
                routing::post(handle_validate_utxos).on(MethodFilter::OPTIONS, handle_post_options),
            )
            .route("/ws", routing::get(handle_subscribe))
            .layer(AddExtensionLayer::new(self))
            .layer(CompressionLayer::new());

        axum::Server::bind(&addr)
            .serve(app.into_make_service())
            .await?;

        Ok(())
    }
}

async fn handle_post_options() -> Result<http::Response<axum::body::Body>, ReportError> {
    http::Response::builder()
        .header("Allow", "OPTIONS, HEAD, POST")
        .body(axum::body::Body::empty())
        .map_err(|err| ReportError(err.into()))
}

async fn handle_broadcast_tx(
    Protobuf(broadcast_request): Protobuf<proto::BroadcastTxRequest>,
    Extension(server): Extension<ChronikServer>,
) -> Result<Protobuf<proto::BroadcastTxResponse>, ReportError> {
    let tx = UnhashedTx::deser(&mut broadcast_request.raw_tx.into()).map_err(InvalidTxEncoding)?;
    let slp_indexer = server.slp_indexer.read().await;
    let check_slp = !broadcast_request.skip_slp_check;
    let txid = slp_indexer.broadcast().broadcast_tx(&tx, check_slp).await?;
    Ok(Protobuf(proto::BroadcastTxResponse {
        txid: txid.as_slice().to_vec(),
    }))
}

async fn handle_broadcast_txs(
    Protobuf(broadcast_request): Protobuf<proto::BroadcastTxsRequest>,
    Extension(server): Extension<ChronikServer>,
) -> Result<Protobuf<proto::BroadcastTxsResponse>, ReportError> {
    let check_slp = !broadcast_request.skip_slp_check;
    let slp_indexer = server.slp_indexer.read().await;
    let broadcast = slp_indexer.broadcast();
    let mut txs = Vec::new();
    for raw_tx in broadcast_request.raw_txs {
        let tx = UnhashedTx::deser(&mut raw_tx.into()).map_err(InvalidTxEncoding)?;
        broadcast
            .test_mempool_accept(&tx, check_slp)
            .await?
            .map_err(Report::from)?;
        txs.push(tx);
    }
    let mut txids = Vec::new();
    for tx in txs {
        let txid = slp_indexer.broadcast().broadcast_tx(&tx, check_slp).await?;
        txids.push(txid);
    }
    Ok(Protobuf(proto::BroadcastTxsResponse {
        txids: txids.iter().map(|txid| txid.as_slice().to_vec()).collect(),
    }))
}

async fn handle_blocks(
    Path((start_height, end_height)): Path<(i32, i32)>,
    Extension(server): Extension<ChronikServer>,
) -> Result<Protobuf<proto::Blocks>, ReportError> {
    if start_height < 0 {
        return Err(InvalidField {
            name: "start_height",
            value: start_height.to_string(),
        }
        .into());
    }
    if end_height < start_height {
        return Err(InvalidField {
            name: "end_height",
            value: end_height.to_string(),
        }
        .into());
    }
    let num_blocks = end_height - start_height + 1;
    if num_blocks as usize > MAX_BLOCKS_PAGE_SIZE {
        return Err(PageSizeTooLarge.into());
    }
    let slp_indexer = server.slp_indexer.read().await;
    let block_stats_reader = slp_indexer.db().block_stats()?;
    let block_reader = slp_indexer.db().blocks()?;
    let mut blocks = Vec::new();
    for block_height in start_height..=end_height {
        let block = block_reader.by_height(block_height)?;
        let block_stats = block_stats_reader.by_height(block_height)?;
        let (block, block_stats) = match block.zip(block_stats) {
            Some(tuple) => tuple,
            None => break,
        };
        blocks.push(block_to_info_proto(&block, &block_stats));
    }
    Ok(Protobuf(proto::Blocks { blocks }))
}

async fn handle_block(
    Path(hash_or_height): Path<String>,
    Extension(server): Extension<ChronikServer>,
) -> Result<Protobuf<proto::Block>, ReportError> {
    let slp_indexer = server.slp_indexer.read().await;
    let block_reader = slp_indexer.db().blocks()?;
    let block_stats_reader = slp_indexer.db().block_stats()?;
    let block = match hash_or_height.parse::<i32>() {
        Ok(height) => block_reader.by_height(height)?,
        Err(_) => {
            let hash = Sha256d::from_hex_be(&hash_or_height)
                .map_err(|_| InvalidHashOrHeight(hash_or_height.clone()))?;
            block_reader.by_hash(&hash)?
        }
    };
    let block = match block {
        Some(block) => block,
        None => return Err(BlockNotFound(hash_or_height).into()),
    };
    let block_stats = block_stats_reader
        .by_height(block.height)?
        .expect("Inconsistent index");
    let block_info = Some(block_to_info_proto(&block, &block_stats));
    let txs = slp_indexer.blocks().block_txs_by_height(block.height)?;
    let txs = txs.into_iter().map(rich_tx_to_proto).collect();
    let bitcoind_rpc = slp_indexer.bitcoind_rpc().clone();
    std::mem::drop(slp_indexer);
    let block_header_json = bitcoind_rpc
        .cmd_json("getblockheader", &[block.hash.to_string().into()])
        .await?;
    let version = block_header_json["version"]
        .as_i32()
        .ok_or(BitcoindBadJson("Missing/ill-typed version"))?;
    let merkle_root = block_header_json["merkleroot"]
        .as_str()
        .ok_or(BitcoindBadJson("Missing/ill-typed merkleroot"))?;
    let merkle_root =
        Sha256d::from_hex_be(merkle_root).wrap_err(BitcoindBadJson("Invalid merkleroot length"))?;
    let nonce = block_header_json["nonce"]
        .as_u64()
        .ok_or(BitcoindBadJson("Missing/ill-typed nonce"))?;
    let median_timestamp = block_header_json["mediantime"]
        .as_i64()
        .ok_or(BitcoindBadJson("Missing/ill-typed mediantime"))?;
    let block_details = Some(proto::BlockDetails {
        version,
        merkle_root: merkle_root.as_slice().to_vec(),
        nonce,
        median_timestamp,
    });
    Ok(Protobuf(proto::Block {
        txs,
        block_info,
        block_details,
    }))
}

async fn handle_tx(
    Path(txid): Path<String>,
    Extension(server): Extension<ChronikServer>,
) -> Result<Protobuf<proto::Tx>, ReportError> {
    let txid = Sha256d::from_hex_be(&txid).map_err(|err| InvalidField {
        name: "txid",
        value: err.to_string(),
    })?;
    let indexer = server.slp_indexer.read().await;
    let rich_tx = indexer
        .txs()
        .rich_tx_by_txid(&txid)
        .map_err(ReportError)?
        .ok_or(TxNotFound(txid))?;
    Ok(Protobuf(rich_tx_to_proto(rich_tx)))
}

async fn handle_token(
    Path(token_id): Path<String>,
    Extension(server): Extension<ChronikServer>,
) -> Result<Protobuf<proto::Token>, ReportError> {
    let token_id = TokenId::from_token_id_hex(&token_id).map_err(|err| InvalidField {
        name: "token_id",
        value: err.to_string(),
    })?;
    let indexer = server.slp_indexer.read().await;
    let rich_tx = indexer
        .txs()
        .rich_tx_by_txid(token_id.hash())
        .map_err(ReportError)?
        .ok_or_else(|| TokenTxidNotFound(token_id.hash().clone()))?;
    let slp_tx_data = rich_tx
        .slp_tx_data
        .ok_or_else(|| TokenTxNotGenesis(token_id.hash().clone()))?;
    let token_stats = indexer
        .tokens()
        .token_stats_by_token_id(&token_id)?
        .unwrap_or_default();
    Ok(Protobuf(proto::Token {
        slp_tx_data: Some(slp_tx_data_to_proto(slp_tx_data)),
        token_stats: Some(proto::TokenStats {
            total_minted: token_stats.total_minted.to_string(),
            total_burned: token_stats.total_burned.to_string(),
        }),
    }))
}

async fn handle_script_history(
    Path((script_type, payload)): Path<(String, String)>,
    Query(query_params): Query<HashMap<String, String>>,
    Extension(server): Extension<ChronikServer>,
) -> Result<Protobuf<proto::TxHistoryPage>, ReportError> {
    let payload = hex::decode(&payload).map_err(|_| InvalidField {
        name: "script payload",
        value: payload.clone(),
    })?;
    let prefix = parse_payload_prefix(script_type, payload.len())?;
    let page_size: usize = match query_params.get("page_size") {
        Some(page_size) => page_size.parse().map_err(|_| InvalidField {
            name: "page_size",
            value: page_size.clone(),
        })?,
        None => DEFAULT_PAGE_SIZE,
    };
    if page_size > MAX_HISTORY_PAGE_SIZE {
        return Err(PageSizeTooLarge.into());
    }
    let page_num: usize = match query_params.get("page") {
        Some(page_num) => page_num.parse().map_err(|_| InvalidField {
            name: "page",
            value: page_num.clone(),
        })?,
        None => 0,
    };
    let slp_indexer = server.slp_indexer.read().await;
    let script_history = slp_indexer.script_history();
    let txs = script_history.rev_history_page(prefix, &payload, page_num, page_size)?;
    let num_pages = script_history.rev_history_num_pages(prefix, &payload, page_size)?;
    Ok(Protobuf(proto::TxHistoryPage {
        txs: txs.into_iter().map(rich_tx_to_proto).collect(),
        num_pages: num_pages as u32,
    }))
}

async fn handle_script_utxos(
    Path((script_type, payload)): Path<(String, String)>,
    Extension(server): Extension<ChronikServer>,
) -> Result<Protobuf<proto::Utxos>, ReportError> {
    let payload = hex::decode(&payload).map_err(|_| InvalidField {
        name: "payload",
        value: payload.clone(),
    })?;
    let prefix = parse_payload_prefix(script_type, payload.len())?;
    let slp_indexer = server.slp_indexer.read().await;
    let mut utxos = slp_indexer.utxos().utxos(&ScriptPayload {
        payload_prefix: prefix,
        payload_data: payload,
    })?;
    utxos.sort_by_key(|utxo| utxo.output.script.bytecode().clone());

    let groups = Itertools::group_by(utxos.into_iter(), |utxo| {
        utxo.output.script.bytecode().clone()
    });
    let script_utxos = groups
        .into_iter()
        .map(|(output_script, utxos)| {
            let utxos = utxos
                .map(|utxo| proto::Utxo {
                    outpoint: Some(proto::OutPoint {
                        txid: utxo.outpoint.txid.as_slice().to_vec(),
                        out_idx: utxo.outpoint.out_idx,
                    }),
                    block_height: utxo.block.map(|block| block.height).unwrap_or(-1),
                    is_coinbase: utxo.is_coinbase,
                    value: utxo.output.value,
                    slp_token: utxo
                        .slp_output
                        .as_ref()
                        .and_then(|slp_output| slp_token_to_proto(slp_output.token)),
                    slp_meta: utxo.slp_output.map(|slp_output| proto::SlpMeta {
                        token_type: match slp_output.token_type {
                            SlpTokenType::Fungible => proto::SlpTokenType::Fungible as i32,
                            SlpTokenType::Nft1Group => proto::SlpTokenType::Nft1Group as i32,
                            SlpTokenType::Nft1Child => proto::SlpTokenType::Nft1Child as i32,
                            SlpTokenType::Unknown => proto::SlpTokenType::UnknownTokenType as i32,
                        },
                        tx_type: match &slp_output.tx_type {
                            SlpTxTypeVariant::Genesis => proto::SlpTxType::Genesis as i32,
                            SlpTxTypeVariant::Send => proto::SlpTxType::Send as i32,
                            SlpTxTypeVariant::Mint => proto::SlpTxType::Mint as i32,
                            SlpTxTypeVariant::Unknown => proto::SlpTxType::UnknownTxType as i32,
                        },
                        token_id: slp_output.token_id.as_slice_be().to_vec(),
                        group_token_id: slp_output
                            .group_token_id
                            .map(|token_id| token_id.as_slice_be().to_vec())
                            .unwrap_or_default(),
                    }),
                    network: network_to_proto(utxo.network) as i32,
                })
                .collect();
            proto::ScriptUtxos {
                output_script: output_script.to_vec(),
                utxos,
            }
        })
        .collect();
    Ok(Protobuf(proto::Utxos { script_utxos }))
}

async fn handle_validate_utxos(
    Protobuf(request): Protobuf<proto::ValidateUtxoRequest>,
    Extension(server): Extension<ChronikServer>,
) -> Result<Protobuf<proto::ValidateUtxoResponse>, ReportError> {
    let slp_indexer = server.slp_indexer.read().await;
    let utxo_states = request
        .outpoints
        .iter()
        .map(|outpoint| {
            let utxo_state = slp_indexer.utxos().utxo_state(&OutPoint {
                txid: Sha256d::from_slice(&outpoint.txid)?,
                out_idx: outpoint.out_idx,
            })?;
            Ok(proto::UtxoState {
                height: utxo_state.height.unwrap_or(-1),
                is_confirmed: utxo_state.height.is_some(),
                state: match utxo_state.state {
                    UtxoStateVariant::Unspent => proto::UtxoStateVariant::Unspent,
                    UtxoStateVariant::Spent => proto::UtxoStateVariant::Spent,
                    UtxoStateVariant::NoSuchTx => proto::UtxoStateVariant::NoSuchTx,
                    UtxoStateVariant::NoSuchOutput => proto::UtxoStateVariant::NoSuchOutput,
                } as i32,
            })
        })
        .collect::<Result<Vec<_>, Report>>()?;
    Ok(Protobuf(proto::ValidateUtxoResponse { utxo_states }))
}

async fn handle_subscribe(
    ws: WebSocketUpgrade,
    Extension(server): Extension<ChronikServer>,
) -> impl IntoResponse {
    ws.on_upgrade(|ws| handle_subscribe_socket(ws, server))
}

enum SubscribeAction {
    Close,
    Message(ws::Message),
    Subscribe {
        script_payload: ScriptPayload,
        is_subscribe: bool,
    },
    Nothing,
}

fn subscribe_client_msg_action(
    client_msg: Option<Result<ws::Message, axum::Error>>,
) -> Result<SubscribeAction, Report> {
    let client_msg = match client_msg {
        Some(client_msg) => client_msg,
        None => return Ok(SubscribeAction::Close),
    };
    match client_msg {
        Ok(ws::Message::Binary(client_msg)) => {
            let subscription = proto::Subscription::decode(client_msg.as_slice())?;
            let payload_prefix =
                parse_payload_prefix(subscription.script_type, subscription.payload.len())?;
            Ok(SubscribeAction::Subscribe {
                script_payload: ScriptPayload {
                    payload_prefix,
                    payload_data: subscription.payload,
                },
                is_subscribe: subscription.is_subscribe,
            })
        }
        Ok(ws::Message::Ping(ping)) => Ok(SubscribeAction::Message(ws::Message::Pong(ping))),
        Ok(ws::Message::Text(_)) => Err(UnexpectedMessageType("Text").into()),
        Ok(ws::Message::Pong(_pong)) => Ok(SubscribeAction::Nothing),
        Ok(ws::Message::Close(_)) | Err(_) => Ok(SubscribeAction::Close),
    }
}

fn subscribe_script_msg_action(
    script_msg: Result<SubscribeScriptMessage, broadcast::error::RecvError>,
) -> Result<SubscribeAction, Report> {
    use proto::subscribe_msg::MsgType;
    let script_msg = match script_msg {
        Ok(script_msg) => script_msg,
        Err(_) => return Ok(SubscribeAction::Nothing),
    };
    let msg_type = Some(match script_msg {
        SubscribeScriptMessage::AddedToMempool(txid) => {
            MsgType::AddedToMempool(proto::MsgAddedToMempool {
                txid: txid.as_slice().to_vec(),
            })
        }
        SubscribeScriptMessage::RemovedFromMempool(txid) => {
            MsgType::RemovedFromMempool(proto::MsgRemovedFromMempool {
                txid: txid.as_slice().to_vec(),
            })
        }
        SubscribeScriptMessage::Confirmed(txid) => MsgType::Confirmed(proto::MsgConfirmed {
            txid: txid.as_slice().to_vec(),
        }),
        SubscribeScriptMessage::Reorg(txid) => MsgType::Reorg(proto::MsgReorg {
            txid: txid.as_slice().to_vec(),
        }),
    });
    let msg_proto = proto::SubscribeMsg { msg_type };
    let msg = ws::Message::Binary(msg_proto.encode_to_vec());
    Ok(SubscribeAction::Message(msg))
}

fn subscribe_block_msg_action(
    block_msg: Result<SubscribeBlockMessage, broadcast::error::RecvError>,
) -> Result<SubscribeAction, Report> {
    use proto::subscribe_msg::MsgType;
    let script_msg = match block_msg {
        Ok(script_msg) => script_msg,
        Err(_) => return Ok(SubscribeAction::Nothing),
    };
    let msg_type = Some(match script_msg {
        SubscribeBlockMessage::BlockConnected(block_hash) => {
            MsgType::BlockConnected(proto::MsgBlockConnected {
                block_hash: block_hash.as_slice().to_vec(),
            })
        }
        SubscribeBlockMessage::BlockDisconnected(block_hash) => {
            MsgType::BlockDisconnected(proto::MsgBlockDisconnected {
                block_hash: block_hash.as_slice().to_vec(),
            })
        }
    });
    let msg_proto = proto::SubscribeMsg { msg_type };
    let msg = ws::Message::Binary(msg_proto.encode_to_vec());
    Ok(SubscribeAction::Message(msg))
}

async fn handle_subscribe_socket(mut socket: WebSocket, server: ChronikServer) {
    let mut subbed_scripts =
        HashMap::<ScriptPayload, broadcast::Receiver<SubscribeScriptMessage>>::new();
    let mut blocks_receiver = {
        let mut slp_indexer = server.slp_indexer.write().await;
        slp_indexer.subscribers_mut().subscribe_to_blocks()
    };
    loop {
        let subscribe_action = if subbed_scripts.is_empty() {
            let client_msg = socket.recv().await;
            subscribe_client_msg_action(client_msg)
        } else {
            let script_receivers = select_all(
                subbed_scripts
                    .values_mut()
                    .map(|receiver| Box::pin(receiver.recv())),
            );
            tokio::select! {
                client_msg = socket.recv() => subscribe_client_msg_action(client_msg),
                block_msg = blocks_receiver.recv() => subscribe_block_msg_action(block_msg),
                (script_msg, _, _) = script_receivers => subscribe_script_msg_action(script_msg),
            }
        };

        let subscribe_action = match subscribe_action {
            Ok(subscribe_action) => subscribe_action,
            // Turn Err into Message
            Err(report) => {
                let (_, Protobuf(error_proto)) = report_to_status_proto(&report);
                SubscribeAction::Message(ws::Message::Binary(error_proto.encode_to_vec()))
            }
        };

        let subscribe_action = match subscribe_action {
            // Send Message, do either Close or Nothing
            SubscribeAction::Message(msg) => match socket.send(msg).await {
                Ok(()) => SubscribeAction::Nothing,
                Err(_) => SubscribeAction::Close,
            },
            other => other,
        };

        match subscribe_action {
            SubscribeAction::Close => {
                if !subbed_scripts.is_empty() {
                    let mut slp_indexer = server.slp_indexer.write().await;
                    for (script_payload, receiver) in subbed_scripts {
                        std::mem::drop(receiver);
                        slp_indexer
                            .subscribers_mut()
                            .unsubscribe_from_script(&script_payload);
                    }
                }
                return;
            }
            SubscribeAction::Message(_) => unreachable!(),
            SubscribeAction::Subscribe {
                script_payload,
                is_subscribe,
            } => {
                let mut slp_indexer = server.slp_indexer.write().await;
                if is_subscribe {
                    let receiver = slp_indexer
                        .subscribers_mut()
                        .subscribe_to_script(&script_payload);
                    subbed_scripts.insert(script_payload, receiver);
                } else {
                    std::mem::drop(subbed_scripts.remove(&script_payload));
                    slp_indexer
                        .subscribers_mut()
                        .unsubscribe_from_script(&script_payload);
                }
            }
            SubscribeAction::Nothing => {}
        }
    }
}

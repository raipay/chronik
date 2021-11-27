use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    extract::{
        ws::{self, WebSocket, WebSocketUpgrade},
        Extension, Path, Query,
    },
    response::IntoResponse,
    routing, AddExtensionLayer, Router,
};
use bitcoinsuite_core::{Hashed, OutPoint, Sha256d};
use bitcoinsuite_error::{ErrorMeta, Report};
use bitcoinsuite_slp::{SlpTokenType, SlpTxTypeVariant};
use chronik_indexer::{subscribers::SubscribeMessage, SlpIndexer, UtxoStateVariant};
use chronik_rocksdb::ScriptPayload;
use futures::future::select_all;
use itertools::Itertools;
use prost::Message;
use thiserror::Error;
use tokio::sync::{broadcast, RwLock};
use tower_http::compression::CompressionLayer;

pub const DEFAULT_PAGE_SIZE: usize = 25;
pub const MAX_PAGE_SIZE: usize = 200;

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

    #[invalid_user_input()]
    #[error("Invalid {name}: {value}")]
    InvalidField { name: &'static str, value: String },

    #[invalid_client_input()]
    #[error("Unexpected message type {0}")]
    UnexpectedMessageType(&'static str),
}

use crate::{
    convert::{network_to_proto, parse_payload_prefix, rich_tx_to_proto, slp_token_to_proto},
    error::{report_to_status_proto, ReportError},
    proto,
    protobuf::Protobuf,
};

use self::ChronikServerError::*;

impl ChronikServer {
    pub async fn run(self) -> Result<(), Report> {
        let addr = self.addr;
        let app = Router::new()
            .route("/tx/:txid", routing::get(handle_tx))
            .route(
                "/script/:type/:payload/history",
                routing::get(handle_script_history),
            )
            .route(
                "/script/:type/:payload/utxos",
                routing::get(handle_script_utxos),
            )
            .route("/validate-utxos", routing::post(handle_validate_utxos))
            .route("/ws", routing::get(handle_subscribe))
            .layer(AddExtensionLayer::new(self))
            .layer(CompressionLayer::new());

        axum::Server::bind(&addr)
            .serve(app.into_make_service())
            .await?;

        Ok(())
    }
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
    if page_size > MAX_PAGE_SIZE {
        return Err(InvalidField {
            name: "page_size",
            value: page_size.to_string(),
        }
        .into());
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
                height: utxo_state.height.unwrap_or_default(),
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
    script_msg: Result<SubscribeMessage, broadcast::error::RecvError>,
) -> Result<SubscribeAction, Report> {
    use proto::subscribe_msg::MsgType;
    let script_msg = match script_msg {
        Ok(script_msg) => script_msg,
        Err(_) => return Ok(SubscribeAction::Nothing),
    };
    let msg_type = Some(match script_msg {
        SubscribeMessage::AddedToMempool(txid) => {
            MsgType::AddedToMempool(proto::MsgAddedToMempool {
                txid: txid.as_slice().to_vec(),
            })
        }
        SubscribeMessage::RemovedFromMempool(txid) => {
            MsgType::RemovedFromMempool(proto::MsgRemovedFromMempool {
                txid: txid.as_slice().to_vec(),
            })
        }
        SubscribeMessage::Confirmed(txid) => MsgType::Confirmed(proto::MsgConfirmed {
            txid: txid.as_slice().to_vec(),
        }),
        SubscribeMessage::Reorg(txid) => MsgType::Reorg(proto::MsgReorg {
            txid: txid.as_slice().to_vec(),
        }),
    });
    let msg_proto = proto::SubscribeMsg { msg_type };
    let msg = ws::Message::Binary(msg_proto.encode_to_vec());
    Ok(SubscribeAction::Message(msg))
}

async fn handle_subscribe_socket(mut socket: WebSocket, server: ChronikServer) {
    let mut subbed_scripts = HashMap::<ScriptPayload, broadcast::Receiver<SubscribeMessage>>::new();
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
                        slp_indexer.subscribers_mut().unsubscribe(&script_payload);
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
                    let receiver = slp_indexer.subscribers_mut().subscribe(&script_payload);
                    subbed_scripts.insert(script_payload, receiver);
                } else {
                    std::mem::drop(subbed_scripts.remove(&script_payload));
                    slp_indexer.subscribers_mut().unsubscribe(&script_payload);
                }
            }
            SubscribeAction::Nothing => {}
        }
    }
}

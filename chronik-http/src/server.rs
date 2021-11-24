use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    extract::{Extension, Path, Query},
    routing, AddExtensionLayer, Router,
};
use bitcoinsuite_core::{Hashed, Sha256d};
use bitcoinsuite_slp::{SlpTokenType, SlpTxTypeVariant};

use bitcoinsuite_error::{ErrorMeta, Report};
use chronik_indexer::SlpIndexer;

use thiserror::Error;
use tokio::sync::RwLock;

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
}

use crate::{
    convert::{network_to_proto, parse_payload_prefix, rich_tx_to_proto, slp_token_to_proto},
    error::ReportError,
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
            .layer(AddExtensionLayer::new(self));

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
    let utxos = slp_indexer.utxos().utxos(prefix, &payload)?;
    let utxos = utxos
        .into_iter()
        .map(|utxo| proto::Utxo {
            outpoint: Some(proto::OutPoint {
                txid: utxo.outpoint.txid.as_slice().to_vec(),
                out_idx: utxo.outpoint.out_idx,
            }),
            block: utxo.block.map(|block| proto::BlockMetadata {
                height: block.height,
                hash: block.hash.as_slice().to_vec(),
                timestamp: block.timestamp,
            }),
            is_coinbase: utxo.is_coinbase,
            output_script: utxo.output.script.bytecode().to_vec(),
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
            time_first_seen: utxo.time_first_seen,
            network: network_to_proto(utxo.network) as i32,
        })
        .collect::<Vec<_>>();
    Ok(Protobuf(proto::Utxos { utxos }))
}

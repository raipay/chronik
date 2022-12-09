use bitcoinsuite_core::{ecc::PUBKEY_LENGTH, Hashed, Network, Sha256, ShaRmd160};
use bitcoinsuite_slp::{RichTx, SlpToken, SlpTokenType, SlpTxData, SlpTxType};

use bitcoinsuite_error::{ErrorMeta, Report};

use chronik_rocksdb::{Block, BlockStats, PayloadPrefix};
use thiserror::Error;

use crate::proto;

#[derive(Debug, Error, ErrorMeta)]
pub enum ChronikConvertError {
    #[invalid_user_input()]
    #[error("Invalid {name}: {value}")]
    InvalidField { name: &'static str, value: String },

    #[invalid_client_input()]
    #[error("Invalid script payload: length expected to be one of {expected:?}, got {actual}")]
    InvalidScriptPayloadLength { expected: Vec<usize>, actual: usize },
}

use self::ChronikConvertError::*;

pub fn block_to_info_proto(block: &Block, block_stats: &BlockStats) -> proto::BlockInfo {
    proto::BlockInfo {
        hash: block.hash.as_slice().to_vec(),
        prev_hash: block.prev_hash.as_slice().to_vec(),
        height: block.height,
        n_bits: block.n_bits,
        timestamp: block.timestamp,
        block_size: block_stats.block_size,
        num_txs: block_stats.num_txs,
        num_inputs: block_stats.num_inputs,
        num_outputs: block_stats.num_outputs,
        sum_input_sats: block_stats.sum_input_sats,
        sum_coinbase_output_sats: block_stats.sum_coinbase_output_sats,
        sum_normal_output_sats: block_stats.sum_normal_output_sats,
        sum_burned_sats: block_stats.sum_burned_sats,
    }
}

pub fn rich_tx_to_proto(rich_tx: RichTx) -> proto::Tx {
    proto::Tx {
        txid: rich_tx.txid.as_slice().to_vec(),
        version: rich_tx.tx.version(),
        inputs: rich_tx
            .inputs()
            .map(|input| proto::TxInput {
                prev_out: Some(proto::OutPoint {
                    txid: input.tx_input.prev_out.txid.as_slice().to_vec(),
                    out_idx: input.tx_input.prev_out.out_idx,
                }),
                input_script: input.tx_input.script.bytecode().to_vec(),
                output_script: input
                    .spent_coin
                    .map(|coin| coin.tx_output.script.bytecode().to_vec())
                    .unwrap_or_default(),
                value: input
                    .spent_coin
                    .map(|coin| coin.tx_output.value)
                    .unwrap_or_default(),
                sequence_no: input.tx_input.sequence.as_u32(),
                slp_burn: input.slp_burn.map(|slp_burn| proto::SlpBurn {
                    token: Some(proto::SlpToken {
                        amount: slp_burn.token.amount.base_amount() as u64,
                        is_mint_baton: slp_burn.token.is_mint_baton,
                    }),
                    token_id: slp_burn.token_id.as_slice_be().to_vec(),
                }),
                slp_token: slp_token_to_proto(input.slp_token),
            })
            .collect(),
        outputs: rich_tx
            .outputs()
            .map(|output| proto::TxOutput {
                value: output.tx_output.value,
                output_script: output.tx_output.script.bytecode().to_vec(),
                slp_token: slp_token_to_proto(output.slp_token),
                spent_by: output.spent_by.map(|spent_by| proto::OutPoint {
                    txid: spent_by.txid.as_slice().to_vec(),
                    out_idx: spent_by.out_idx,
                }),
            })
            .collect(),
        lock_time: rich_tx.tx.lock_time(),
        slp_tx_data: rich_tx.slp_tx_data.map(slp_tx_data_to_proto),
        slp_error_msg: rich_tx.slp_error_msg.unwrap_or_default(),
        block: rich_tx.block.map(|block| proto::BlockMetadata {
            height: block.height,
            hash: block.hash.as_slice().to_vec(),
            timestamp: block.timestamp,
        }),
        time_first_seen: rich_tx.time_first_seen,
        size: rich_tx.tx.raw().len() as u32,
        is_coinbase: rich_tx.tx.inputs()[0].prev_out.is_coinbase(),
        network: network_to_proto(rich_tx.network) as i32,
    }
}

#[allow(clippy::boxed_local)]
pub fn slp_tx_data_to_proto(slp_tx_data: Box<SlpTxData>) -> proto::SlpTxData {
    proto::SlpTxData {
        slp_meta: Some(proto::SlpMeta {
            token_type: match slp_tx_data.slp_token_type {
                SlpTokenType::Fungible => proto::SlpTokenType::Fungible as i32,
                SlpTokenType::Nft1Group => proto::SlpTokenType::Nft1Group as i32,
                SlpTokenType::Nft1Child => proto::SlpTokenType::Nft1Child as i32,
                SlpTokenType::Unknown => proto::SlpTokenType::UnknownTokenType as i32,
            },
            tx_type: match &slp_tx_data.slp_tx_type {
                SlpTxType::Genesis(_) => proto::SlpTxType::Genesis as i32,
                SlpTxType::Send => proto::SlpTxType::Send as i32,
                SlpTxType::Mint => proto::SlpTxType::Mint as i32,
                SlpTxType::Burn(_) => proto::SlpTxType::Burn as i32,
                SlpTxType::Unknown => proto::SlpTxType::UnknownTxType as i32,
            },
            token_id: slp_tx_data.token_id.as_slice_be().to_vec(),
            group_token_id: slp_tx_data
                .group_token_id
                .map(|token_id| token_id.as_slice_be().to_vec())
                .unwrap_or_default(),
        }),
        genesis_info: match slp_tx_data.slp_tx_type {
            SlpTxType::Genesis(genesis_info) => Some(proto::SlpGenesisInfo {
                token_ticker: genesis_info.token_ticker.to_vec(),
                token_name: genesis_info.token_name.to_vec(),
                token_document_url: genesis_info.token_document_url.to_vec(),
                token_document_hash: genesis_info
                    .token_document_hash
                    .map(|arr| arr.to_vec())
                    .unwrap_or_default(),
                decimals: genesis_info.decimals,
            }),
            _ => None,
        },
    }
}

pub fn network_to_proto(network: Network) -> proto::Network {
    match network {
        Network::BCH => proto::Network::Bch,
        Network::XEC => proto::Network::Xec,
        Network::XPI => proto::Network::Xpi,
        Network::XRG => proto::Network::Xrg,
    }
}

pub fn slp_token_to_proto(slp_token: SlpToken) -> Option<proto::SlpToken> {
    if slp_token == SlpToken::EMPTY {
        return None;
    }
    Some(proto::SlpToken {
        amount: slp_token.amount.base_amount() as u64,
        is_mint_baton: slp_token.is_mint_baton,
    })
}

pub fn parse_payload_prefix(
    script_type: String,
    payload_len: usize,
) -> Result<PayloadPrefix, Report> {
    fn pl_err(expected: Vec<usize>, actual: usize) -> Report {
        InvalidScriptPayloadLength { expected, actual }.into()
    }
    match script_type.as_str() {
        "other" => Ok(PayloadPrefix::Other),
        "p2pk" if payload_len == PUBKEY_LENGTH => Ok(PayloadPrefix::P2PK),
        "p2pk" if payload_len == 65 => Ok(PayloadPrefix::P2PKLegacy),
        "p2pk" => Err(pl_err(vec![PUBKEY_LENGTH, 65], payload_len)),
        "p2pkh" if payload_len == ShaRmd160::size() => Ok(PayloadPrefix::P2PKH),
        "p2sh" if payload_len == ShaRmd160::size() => Ok(PayloadPrefix::P2SH),
        "p2pkh" | "p2sh" => Err(pl_err(vec![ShaRmd160::size()], payload_len)),
        "p2tr-commitment" if payload_len == PUBKEY_LENGTH => Ok(PayloadPrefix::P2TRCommitment),
        "p2tr-commitment" => Err(pl_err(vec![PUBKEY_LENGTH], payload_len)),
        "p2tr-state" if payload_len == Sha256::size() => Ok(PayloadPrefix::P2TRState),
        "p2tr-state" => Err(pl_err(vec![Sha256::size()], payload_len)),
        _ => Err(InvalidField {
            name: "script_type",
            value: script_type,
        }
        .into()),
    }
}

use bitcoinsuite_core::{BitcoinCode, Bytes, OutPoint, Sha256d, TxOutput, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Result};
use bitcoinsuite_slp::{RichTxBlock, RichUtxo, SlpOutput};
use chronik_rocksdb::{BlockHeight, ScriptPayload, TxNum, UtxoDelta};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use thiserror::Error;

use crate::SlpIndexer;

pub struct Utxos<'a> {
    indexer: &'a SlpIndexer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UtxoState {
    pub height: Option<BlockHeight>,
    pub state: UtxoStateVariant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UtxoStateVariant {
    Unspent,
    Spent,
    NoSuchTx,
    NoSuchOutput,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum UtxosError {
    #[critical()]
    #[error("Inconsistent db, tx_num doesn't exist: {0}")]
    InconsistentNoSuchTxNum(TxNum),

    #[critical()]
    #[error("Inconsistent db, txid doesn't exist in mempool: {0}")]
    InconsistentNoSuchMempoolTx(Sha256d),

    #[critical()]
    #[error("Couldn't reconstruct script in output: {0:?}")]
    CouldntReconstructScript(OutPoint),
}

use self::UtxosError::*;

impl<'a> Utxos<'a> {
    pub fn new(indexer: &'a SlpIndexer) -> Self {
        Utxos { indexer }
    }

    pub fn utxos(&self, script_payload: &ScriptPayload) -> Result<Vec<RichUtxo>> {
        let prefix = script_payload.payload_prefix;
        let payload = &script_payload.payload_data;
        let tx_reader = self.indexer.db().txs()?;
        let block_reader = self.indexer.db().blocks()?;
        let slp_reader = self.indexer.db().slp()?;
        let db_utxos = self.indexer.db().utxos()?.utxos(prefix, payload)?;
        let default_utxo_delta = UtxoDelta::default();
        let mempool_delta = self
            .indexer
            .db_mempool()
            .utxos(prefix, payload)
            .unwrap_or(&default_utxo_delta);
        let mut utxos = db_utxos
            .into_par_iter()
            .map(|db_utxo| -> Result<Option<_>> {
                let block_tx = tx_reader
                    .by_tx_num(db_utxo.outpoint.tx_num)?
                    .ok_or(InconsistentNoSuchTxNum(db_utxo.outpoint.tx_num))?;
                let outpoint = OutPoint {
                    txid: block_tx.entry.txid.clone(),
                    out_idx: db_utxo.outpoint.out_idx,
                };
                let out_idx = outpoint.out_idx as usize;
                if mempool_delta.deletes.contains(&outpoint) {
                    return Ok(None);
                }
                let block = block_reader
                    .by_height(block_tx.block_height)?
                    .expect("Inconsistent db");
                let output = if db_utxo.is_partial_script {
                    let raw_tx = self.indexer.rpc_interface.get_block_slice(
                        block.file_num,
                        block_tx.entry.data_pos,
                        block_tx.entry.tx_size,
                    )?;
                    let mut raw_tx = Bytes::from_bytes(raw_tx);
                    let tx = UnhashedTx::deser(&mut raw_tx)?;
                    tx.outputs[out_idx].clone()
                } else {
                    TxOutput {
                        script: script_payload
                            .reconstruct_script()
                            .ok_or_else(|| CouldntReconstructScript(outpoint.clone()))?,
                        value: db_utxo.value,
                    }
                };
                let slp_output =
                    slp_reader
                        .slp_data_by_tx_num(db_utxo.outpoint.tx_num)?
                        .map(|(slp_data, _)| {
                            Box::new(SlpOutput {
                                token_id: slp_data.token_id,
                                tx_type: slp_data.slp_tx_type.tx_type_variant(),
                                token_type: slp_data.slp_token_type,
                                token: slp_data.output_tokens[out_idx],
                                group_token_id: slp_data.group_token_id,
                            })
                        });
                let rich_utxo = RichUtxo {
                    outpoint,
                    block: Some(RichTxBlock {
                        height: block_tx.block_height,
                        hash: block.hash.clone(),
                        timestamp: block.timestamp,
                    }),
                    is_coinbase: block_tx.entry.is_coinbase,
                    output,
                    slp_output,
                    time_first_seen: block_tx.entry.time_first_seen,
                    network: self.indexer.network,
                };
                Ok(Some(rich_utxo))
            })
            .filter_map(|result| result.transpose())
            .collect::<Result<Vec<_>>>()?;
        for outpoint in mempool_delta.inserts.iter().cloned() {
            let out_idx = outpoint.out_idx as usize;
            let entry = self
                .indexer
                .db_mempool()
                .tx(&outpoint.txid)
                .ok_or_else(|| InconsistentNoSuchMempoolTx(outpoint.txid.clone()))?;
            let output = entry.tx.outputs[out_idx].clone();
            let slp_output = self
                .indexer
                .db_mempool_slp()
                .slp_tx_data(&outpoint.txid)
                .map(|slp_data| {
                    Box::new(SlpOutput {
                        token_id: slp_data.slp_tx_data.token_id.clone(),
                        tx_type: slp_data.slp_tx_data.slp_tx_type.tx_type_variant(),
                        token_type: slp_data.slp_tx_data.slp_token_type,
                        token: slp_data.slp_tx_data.output_tokens[out_idx],
                        group_token_id: slp_data.slp_tx_data.group_token_id.clone(),
                    })
                });
            let rich_utxo = RichUtxo {
                outpoint,
                block: None,
                is_coinbase: false,
                output,
                slp_output,
                time_first_seen: entry.time_first_seen,
                network: self.indexer.network,
            };
            utxos.push(rich_utxo);
        }
        Ok(utxos)
    }

    pub fn utxo_state(&self, outpoint: &OutPoint) -> Result<UtxoState> {
        let mempool = self.indexer.db_mempool();
        let mut is_spent_in_mempool = false;
        if let Some(spends) = mempool.spends(&outpoint.txid) {
            if spends
                .iter()
                .any(|&(out_idx, _, _)| out_idx == outpoint.out_idx)
            {
                if mempool.tx(&outpoint.txid).is_some() {
                    return Ok(UtxoState {
                        height: None,
                        state: UtxoStateVariant::Spent,
                    });
                }
                is_spent_in_mempool = true;
            }
        }
        if !is_spent_in_mempool {
            if let Some(tx) = mempool.tx(&outpoint.txid) {
                if outpoint.out_idx as usize >= tx.tx.outputs.len() {
                    return Ok(UtxoState {
                        height: None,
                        state: UtxoStateVariant::NoSuchOutput,
                    });
                }
                return Ok(UtxoState {
                    height: None,
                    state: UtxoStateVariant::Unspent,
                });
            }
        }
        let tx_reader = self.indexer.db().txs()?;
        let spends_reader = self.indexer.db().spends()?;
        let (tx_num, block_tx) = match tx_reader.tx_and_num_by_txid(&outpoint.txid)? {
            Some(tx) => tx,
            None => {
                return Ok(UtxoState {
                    height: None,
                    state: UtxoStateVariant::NoSuchTx,
                })
            }
        };
        if is_spent_in_mempool {
            return Ok(UtxoState {
                height: Some(block_tx.block_height),
                state: UtxoStateVariant::Spent,
            });
        }
        let spends = spends_reader.spends_by_tx_num(tx_num)?;
        if spends.iter().any(|spend| spend.out_idx == outpoint.out_idx) {
            return Ok(UtxoState {
                height: Some(block_tx.block_height),
                state: UtxoStateVariant::Spent,
            });
        }
        let block_reader = self.indexer.db().blocks()?;
        let block = block_reader
            .by_height(block_tx.block_height)?
            .expect("Inconsistent db");
        let raw_tx = self.indexer.rpc_interface.get_block_slice(
            block.file_num,
            block_tx.entry.data_pos,
            block_tx.entry.tx_size,
        )?;
        let tx = UnhashedTx::deser(&mut Bytes::from_bytes(raw_tx))?;
        if outpoint.out_idx as usize >= tx.outputs.len() {
            return Ok(UtxoState {
                height: Some(block_tx.block_height),
                state: UtxoStateVariant::NoSuchOutput,
            });
        }
        Ok(UtxoState {
            height: Some(block_tx.block_height),
            state: UtxoStateVariant::Unspent,
        })
    }
}

use bitcoinsuite_core::{BitcoinCode, Bytes, OutPoint, Sha256d, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Result};
use bitcoinsuite_slp::{RichTxBlock, RichUtxo, SlpOutput};
use chronik_rocksdb::{PayloadPrefix, TxNum, UtxoDelta};
use thiserror::Error;

use crate::SlpIndexer;

pub struct Utxos<'a> {
    indexer: &'a SlpIndexer,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum UtxosError {
    #[critical()]
    #[error("Inconsistent db, tx_num doesn't exist: {0}")]
    InconsistentNoSuchTxNum(TxNum),

    #[critical()]
    #[error("Inconsistent db, txid doesn't exist in mempool: {0}")]
    InconsistentNoSuchMempoolTx(Sha256d),
}

use self::UtxosError::*;

impl<'a> Utxos<'a> {
    pub fn new(indexer: &'a SlpIndexer) -> Self {
        Utxos { indexer }
    }

    pub fn utxos(&self, prefix: PayloadPrefix, payload: &[u8]) -> Result<Vec<RichUtxo>> {
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
        let mut utxos = Vec::new();
        for db_utxo in db_utxos {
            let block_tx = tx_reader
                .by_tx_num(db_utxo.tx_num)?
                .ok_or(InconsistentNoSuchTxNum(db_utxo.tx_num))?;
            let outpoint = OutPoint {
                txid: block_tx.entry.txid.clone(),
                out_idx: db_utxo.out_idx,
            };
            let out_idx = outpoint.out_idx as usize;
            if mempool_delta.deletes.contains(&outpoint) {
                continue;
            }
            let block = block_reader
                .by_height(block_tx.block_height)?
                .expect("Inconsistent db");
            let raw_tx = self.indexer.rpc_interface.get_block_slice(
                block.file_num,
                block_tx.entry.data_pos,
                block_tx.entry.tx_size,
            )?;
            let mut raw_tx = Bytes::from_bytes(raw_tx);
            let tx = UnhashedTx::deser(&mut raw_tx)?;
            let output = tx.outputs[out_idx].clone();
            let slp_output = slp_reader
                .slp_data_by_tx_num(db_utxo.tx_num)?
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
                is_coinbase: tx.inputs[0].prev_out.is_coinbase(),
                output,
                slp_output,
                time_first_seen: block_tx.entry.time_first_seen,
                network: self.indexer.network,
            };
            utxos.push(rich_utxo);
        }
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
}

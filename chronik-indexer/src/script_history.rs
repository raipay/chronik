use bitcoinsuite_core::Sha256d;
use bitcoinsuite_error::{ErrorMeta, Result};
use bitcoinsuite_slp::RichTx;
use chronik_rocksdb::{PayloadPrefix, TxNum};
use thiserror::Error;

use crate::SlpIndexer;

pub struct ScriptHistory<'a> {
    indexer: &'a SlpIndexer,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum ScriptHistoryError {
    #[critical()]
    #[error("Inconsistent mempool, txid doesn't exist: {0}")]
    InconsistentNoSuchMempoolTx(Sha256d),

    #[critical()]
    #[error("Inconsistent db, tx_num doesn't exist: {0}")]
    InconsistentNoSuchBlockTxNum(TxNum),
}

use self::ScriptHistoryError::*;

impl<'a> ScriptHistory<'a> {
    pub fn new(indexer: &'a SlpIndexer) -> Self {
        ScriptHistory { indexer }
    }

    /// Tx history in reverse order, i.e. most recent first and oldest last.
    pub fn rev_history_page(
        &self,
        prefix: PayloadPrefix,
        payload: &[u8],
        history_page_num: usize,
        history_page_size: usize,
    ) -> Result<Vec<RichTx>> {
        let mempool = self.indexer.db_mempool();
        let mut page_txs = Vec::new();
        if let Some(address_mempool_by_time) = mempool.outputs(prefix, payload) {
            page_txs = address_mempool_by_time
                .iter()
                .rev()
                .skip(history_page_num * history_page_size)
                .take(history_page_size)
                .map(|(_, txid)| -> Result<_> {
                    let entry = self
                        .indexer
                        .db_mempool()
                        .tx(txid)
                        .ok_or_else(|| InconsistentNoSuchMempoolTx(txid.clone()))?;
                    self.indexer.txs().rich_mempool_tx(txid, entry)
                })
                .collect::<Result<Vec<_>>>()?;
        }
        let num_page_mempool_txs = page_txs.len();
        if num_page_mempool_txs == history_page_size {
            return Ok(page_txs);
        }
        let num_block_txs = self.num_block_txs(prefix, payload)?;
        let num_mempool_txs = self.num_mempool_txs(prefix, payload);
        let total_num_txs = num_mempool_txs + num_block_txs;
        // Index of first tx to query (chonological order)
        let first_tx_idx = match total_num_txs.checked_sub(history_page_num * history_page_size + 1)
        {
            Some(first_tx_idx_no_mempool) => first_tx_idx_no_mempool - num_page_mempool_txs,
            None => return Ok(page_txs),
        };
        let db_outputs = self.indexer.db().outputs()?;
        let db_page_num_start = first_tx_idx / db_outputs.page_size();
        let mut first_inner_idx = first_tx_idx % db_outputs.page_size();
        let tx_reader = self.indexer.db().txs()?;
        // We start from the back and move to the front (rev history)
        'outer: for current_page_num in (0..=db_page_num_start).rev() {
            let db_page_tx_nums = db_outputs.page_txs(current_page_num as u32, prefix, payload)?;
            for inner_idx in (0..=first_inner_idx).rev() {
                let tx_num = db_page_tx_nums[inner_idx];
                let block_tx = tx_reader
                    .by_tx_num(tx_num)?
                    .ok_or(InconsistentNoSuchBlockTxNum(tx_num))?;
                let rich_tx = self.indexer.txs().rich_block_tx(tx_num, &block_tx)?;
                page_txs.push(rich_tx);
                if page_txs.len() == history_page_size {
                    break 'outer;
                }
            }
            first_inner_idx = db_outputs.page_size() - 1;
        }
        // Stable sort, so the block order is retained when timestamps are identical
        page_txs.sort_by_key(|tx| (tx.block.is_some(), -tx.timestamp()));
        Ok(page_txs)
    }

    pub fn rev_history_num_pages(
        &self,
        prefix: PayloadPrefix,
        payload: &[u8],
        page_size: usize,
    ) -> Result<usize> {
        let num_mempool_txs = self.num_mempool_txs(prefix, payload);
        let num_block_txs = self.num_block_txs(prefix, payload)?;
        let total_num_txs = num_mempool_txs + num_block_txs;
        Ok((total_num_txs + page_size - 1) / page_size)
    }

    pub fn num_block_txs(&self, prefix: PayloadPrefix, payload: &[u8]) -> Result<usize> {
        let db_outputs = self.indexer.db().outputs()?;
        let num_pages = db_outputs.num_pages_by_payload(prefix, payload)?;
        if num_pages == 0 {
            return Ok(0);
        }
        let last_page_num = num_pages as u32 - 1;
        let last_page_size = db_outputs.page_txs(last_page_num, prefix, payload)?.len();
        Ok(db_outputs.page_size() * (num_pages - 1) + last_page_size)
    }

    pub fn num_mempool_txs(&self, prefix: PayloadPrefix, payload: &[u8]) -> usize {
        self.indexer
            .db_mempool()
            .outputs(prefix, payload)
            .map(|txs| txs.len())
            .unwrap_or_default()
    }
}

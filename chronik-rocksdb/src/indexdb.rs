use std::sync::{RwLock, RwLockReadGuard};

use bitcoinsuite_core::{Script, Sha256d, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Result};
use rocksdb::WriteBatch;
use thiserror::Error;

use crate::{
    Block, BlockReader, BlockTxs, BlockWriter, Db, OutputsConf, OutputsReader, OutputsWriter,
    OutputsWriterCache, SpendsReader, SpendsWriter, Timings, TxReader, TxWriter, UtxosReader,
    UtxosWriter,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct IndexTimings {
    pub timings: Timings,
    pub outputs_timings: Timings,
    pub utxos_timings: Timings,
}

pub struct IndexDb {
    db: Db,
    timings: RwLock<IndexTimings>,
}

pub struct IndexCache {
    outputs_cache: OutputsWriterCache,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum IndexDbError {
    #[critical()]
    #[error("Unknown block")]
    UnknownBlock(Sha256d),
}

use self::IndexDbError::*;

impl IndexDb {
    pub fn new(db: Db) -> Self {
        IndexDb {
            db,
            timings: Default::default(),
        }
    }

    pub fn blocks(&self) -> Result<BlockReader> {
        BlockReader::new(&self.db)
    }

    pub fn txs(&self) -> Result<TxReader> {
        TxReader::new(&self.db)
    }

    pub fn outputs(&self) -> Result<OutputsReader> {
        OutputsReader::new(&self.db)
    }

    pub fn utxos(&self) -> Result<UtxosReader> {
        UtxosReader::new(&self.db)
    }

    pub fn spends(&self) -> Result<SpendsReader> {
        SpendsReader::new(&self.db)
    }

    pub fn timings(&self) -> RwLockReadGuard<IndexTimings> {
        self.timings.read().unwrap()
    }

    pub fn insert_block<'b>(
        &self,
        block: &Block,
        block_txs: &'b BlockTxs,
        txs: &[UnhashedTx],
        block_spent_scripts: impl IntoIterator<Item = impl IntoIterator<Item = &'b Script>>,
        cache: &mut IndexCache,
    ) -> Result<()> {
        let mut timings = self.timings.write().unwrap();
        let block_writer = BlockWriter::new(&self.db)?;
        let tx_writer = TxWriter::new(&self.db)?;
        let conf = OutputsConf { page_size: 1000 };
        let output_writer = OutputsWriter::new(&self.db, conf)?;
        let utxo_writer = UtxosWriter::new(&self.db)?;
        let spends_writer = SpendsWriter::new(&self.db)?;
        let mut batch = WriteBatch::default();

        timings.timings.start_timer();
        block_writer.insert(&mut batch, block)?;
        timings.timings.stop_timer("blocks");

        timings.timings.start_timer();
        let first_tx_num = tx_writer.insert_block_txs(&mut batch, block_txs)?;
        timings.timings.stop_timer("txs");

        timings.timings.start_timer();
        let outputs_timings = output_writer.insert_block_txs(
            &mut batch,
            first_tx_num,
            txs,
            &mut cache.outputs_cache,
        )?;
        timings.timings.stop_timer("outputs");
        timings.outputs_timings.add(&outputs_timings);

        timings.timings.start_timer();
        let block_txids = block_txs.txs.iter().map(|tx| &tx.txid);
        let utxos_timings = utxo_writer.insert_block_txs(
            &mut batch,
            first_tx_num,
            block_txids.clone(),
            txs,
            block_spent_scripts,
        )?;
        timings.timings.stop_timer("utxos");
        timings.utxos_timings.add(&utxos_timings);

        timings.timings.start_timer();
        spends_writer.insert_block_txs(&mut batch, first_tx_num, block_txids, txs)?;
        timings.timings.stop_timer("spends");

        timings.timings.start_timer();
        self.db.write_batch(batch)?;
        timings.timings.stop_timer("insert");

        Ok(())
    }

    pub fn delete_block<'b>(
        &self,
        block_hash: &Sha256d,
        height: i32,
        block_txids: impl IntoIterator<Item = &'b Sha256d> + Clone,
        txs: &[UnhashedTx],
        block_spent_scripts: impl IntoIterator<Item = impl IntoIterator<Item = &'b Script>>,
        cache: &mut IndexCache,
    ) -> Result<()> {
        let block_writer = BlockWriter::new(&self.db)?;
        let tx_writer = TxWriter::new(&self.db)?;
        let conf = OutputsConf { page_size: 1000 };
        let output_writer = OutputsWriter::new(&self.db, conf)?;
        let utxo_writer = UtxosWriter::new(&self.db)?;
        let spends_writer = SpendsWriter::new(&self.db)?;
        let tx_reader = TxReader::new(&self.db)?;
        let first_tx_num = tx_reader.first_tx_num_by_block(height)?.unwrap();
        let mut batch = WriteBatch::default();
        block_writer.delete_by_hash(&mut batch, block_hash)?;
        let block = self
            .blocks()?
            .by_hash(block_hash)?
            .ok_or_else(|| UnknownBlock(block_hash.clone()))?;
        tx_writer.delete_block_txs(&mut batch, block.height)?;
        output_writer.delete_block_txs(&mut batch, first_tx_num, txs, &mut cache.outputs_cache)?;
        utxo_writer.delete_block_txs(
            &mut batch,
            first_tx_num,
            block_txids.clone(),
            txs,
            block_spent_scripts,
        )?;
        spends_writer.delete_block_txs(&mut batch, first_tx_num, block_txids, txs)?;
        self.db.write_batch(batch)?;
        Ok(())
    }
}

impl IndexCache {
    pub fn new(outputs_capacity: usize) -> Self {
        IndexCache {
            outputs_cache: OutputsWriterCache::with_capacity(outputs_capacity),
        }
    }
}

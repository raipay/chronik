use std::{
    collections::{HashMap, HashSet},
    sync::{RwLock, RwLockReadGuard},
};

use bitcoinsuite_core::{Script, Sha256d, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Result};
use rocksdb::WriteBatch;
use thiserror::Error;

use crate::{
    input_tx_nums::fetch_input_tx_nums, Block, BlockHeight, BlockReader, BlockTxs, BlockWriter, Db,
    MempoolData, MempoolDeleteMode, MempoolSlpData, MempoolWriter, OutputsConf, OutputsReader,
    OutputsWriter, OutputsWriterCache, SlpWriter, SpendsReader, SpendsWriter, Timings, TxReader,
    TxWriter, UtxosReader, UtxosWriter,
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

pub struct IndexMemData {
    outputs_cache: OutputsWriterCache,
    mempool: MempoolData,
    mempool_slp: MempoolSlpData,
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

    pub fn mempool<'a>(&self, data: &'a IndexMemData) -> &'a MempoolData {
        &data.mempool
    }

    pub fn mempool_slp<'a>(&self, data: &'a IndexMemData) -> &'a MempoolSlpData {
        &data.mempool_slp
    }

    pub fn insert_block<'b>(
        &self,
        block: &Block,
        block_txs: &'b BlockTxs,
        txs: &[UnhashedTx],
        block_spent_script_fn: impl Fn(/*tx_num:*/ usize, /*out_idx:*/ usize) -> &'b Script,
        data: &mut IndexMemData,
    ) -> Result<()> {
        let mut timings = self.timings.write().unwrap();
        let block_writer = BlockWriter::new(&self.db)?;
        let tx_writer = TxWriter::new(&self.db)?;
        let conf = OutputsConf { page_size: 1000 };
        let output_writer = OutputsWriter::new(&self.db, conf)?;
        let utxo_writer = UtxosWriter::new(&self.db)?;
        let spends_writer = SpendsWriter::new(&self.db)?;
        let slp_writer = SlpWriter::new(&self.db)?;
        let mut batch = WriteBatch::default();

        let txids_fn = |idx: usize| &block_txs.txs[idx].txid;

        timings.timings.start_timer();
        block_writer.insert(&mut batch, block)?;
        timings.timings.stop_timer("blocks");

        timings.timings.start_timer();
        let first_tx_num = tx_writer.insert_block_txs(&mut batch, block_txs)?;
        timings.timings.stop_timer("txs");

        timings.timings.start_timer();
        let input_tx_nums = fetch_input_tx_nums(&self.db, first_tx_num, txids_fn, txs)?;
        timings.timings.stop_timer("fetch_input_tx_nums");

        timings.timings.start_timer();
        let outputs_timings = output_writer.insert_block_txs(
            &mut batch,
            first_tx_num,
            txs,
            &mut data.outputs_cache,
        )?;
        timings.timings.stop_timer("outputs");
        timings.outputs_timings.add(&outputs_timings);

        timings.timings.start_timer();
        let utxos_timings = utxo_writer.insert_block_txs(
            &mut batch,
            first_tx_num,
            &txids_fn,
            txs,
            &block_spent_script_fn,
            &input_tx_nums,
        )?;
        timings.timings.stop_timer("utxos");
        timings.utxos_timings.add(&utxos_timings);

        timings.timings.start_timer();
        spends_writer.insert_block_txs(&mut batch, first_tx_num, txs, &input_tx_nums)?;
        timings.timings.stop_timer("spends");

        timings.timings.start_timer();
        slp_writer.insert_block_txs(&mut batch, first_tx_num, txs, txids_fn)?;
        timings.timings.stop_timer("slp");

        timings.timings.start_timer();
        self.db.write_batch(batch)?;
        timings.timings.stop_timer("insert");

        let mempool_txids = block_txs
            .txs
            .iter()
            .filter_map(|entry| data.mempool.tx(&entry.txid).map(|_| &entry.txid))
            .collect::<HashSet<_>>();

        let mut mempool_writer = MempoolWriter {
            db: &self.db,
            mempool: &mut data.mempool,
            mempool_slp: &mut data.mempool_slp,
        };
        mempool_writer.delete_mempool_mined_txs(mempool_txids)?;

        Ok(())
    }

    pub fn delete_block<'b>(
        &self,
        block_hash: &Sha256d,
        height: BlockHeight,
        txids_fn: impl Fn(usize) -> &'b Sha256d + Send + Sync,
        txs: &[UnhashedTx],
        block_spent_script_fn: impl Fn(/*tx_num:*/ usize, /*out_idx:*/ usize) -> &'b Script,
        data: &mut IndexMemData,
    ) -> Result<()> {
        let block_writer = BlockWriter::new(&self.db)?;
        let tx_writer = TxWriter::new(&self.db)?;
        let conf = OutputsConf { page_size: 1000 };
        let output_writer = OutputsWriter::new(&self.db, conf)?;
        let utxo_writer = UtxosWriter::new(&self.db)?;
        let spends_writer = SpendsWriter::new(&self.db)?;
        let slp_writer = SlpWriter::new(&self.db)?;
        let tx_reader = TxReader::new(&self.db)?;
        let first_tx_num = tx_reader.first_tx_num_by_block(height)?.unwrap();
        let input_tx_nums = fetch_input_tx_nums(&self.db, first_tx_num, &txids_fn, txs)?;
        let mut batch = WriteBatch::default();
        block_writer.delete_by_hash(&mut batch, block_hash)?;
        let block = self
            .blocks()?
            .by_hash(block_hash)?
            .ok_or_else(|| UnknownBlock(block_hash.clone()))?;
        tx_writer.delete_block_txs(&mut batch, block.height)?;
        output_writer.delete_block_txs(&mut batch, first_tx_num, txs, &mut data.outputs_cache)?;
        utxo_writer.delete_block_txs(
            &mut batch,
            first_tx_num,
            &txids_fn,
            txs,
            block_spent_script_fn,
        )?;
        spends_writer.delete_block_txs(&mut batch, first_tx_num, txs, &input_tx_nums)?;
        slp_writer.delete_block_txs(&mut batch, first_tx_num, txs, &txids_fn)?;
        self.db.write_batch(batch)?;
        Ok(())
    }

    pub fn insert_mempool_tx(
        &self,
        data: &mut IndexMemData,
        txid: Sha256d,
        tx: UnhashedTx,
        spent_scripts: Vec<Script>,
    ) -> Result<()> {
        self.mempool_writer(data)
            .insert_mempool_tx(txid, tx, spent_scripts)?;
        Ok(())
    }

    pub fn insert_mempool_batch_txs(
        &self,
        data: &mut IndexMemData,
        txs: HashMap<Sha256d, (UnhashedTx, Vec<Script>)>,
    ) -> Result<()> {
        self.mempool_writer(data).insert_mempool_batch_txs(txs)?;
        Ok(())
    }

    pub fn remove_mempool_tx(&self, data: &mut IndexMemData, txid: &Sha256d) -> Result<()> {
        self.mempool_writer(data)
            .delete_mempool_tx(txid, MempoolDeleteMode::Remove)?;
        Ok(())
    }

    fn mempool_writer<'a>(&'a self, data: &'a mut IndexMemData) -> MempoolWriter<'a> {
        MempoolWriter {
            db: &self.db,
            mempool: &mut data.mempool,
            mempool_slp: &mut data.mempool_slp,
        }
    }
}

impl IndexMemData {
    pub fn new(outputs_capacity: usize) -> Self {
        IndexMemData {
            outputs_cache: OutputsWriterCache::with_capacity(outputs_capacity),
            mempool: MempoolData::default(),
            mempool_slp: MempoolSlpData::default(),
        }
    }
}

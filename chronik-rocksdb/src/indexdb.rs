use std::{
    collections::{HashMap, HashSet},
    sync::{RwLock, RwLockReadGuard},
};

use bitcoinsuite_core::{Sha256d, TxOutput, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Result};
use bitcoinsuite_slp::{SlpError, SlpValidTxData};
use rocksdb::WriteBatch;
use thiserror::Error;

use crate::{
    input_tx_nums::fetch_input_tx_nums, Block, BlockHeight, BlockReader, BlockStatsReader,
    BlockStatsWriter, BlockTxs, BlockWriter, Db, DbSchema, MempoolData, MempoolDeleteMode,
    MempoolSlpData, MempoolTxEntry, MempoolWriter, ScriptTxsConf, ScriptTxsReader, ScriptTxsWriter,
    ScriptTxsWriterCache, SlpReader, SlpWriter, SpendsReader, SpendsWriter, Timings, TxReader,
    TxWriter, UtxosReader, UtxosWriter,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct IndexTimings {
    pub timings: Timings,
    pub script_txs_timings: Timings,
    pub utxos_timings: Timings,
}

pub struct IndexDb {
    db: Db,
    timings: RwLock<IndexTimings>,
    script_txs_conf: ScriptTxsConf,
}

pub struct IndexMemData {
    script_txs_cache: ScriptTxsWriterCache,
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
    pub fn new(db: Db, script_txs_conf: ScriptTxsConf) -> Self {
        IndexDb {
            db,
            timings: Default::default(),
            script_txs_conf,
        }
    }

    pub fn check_db_version(&self) -> Result<()> {
        DbSchema::new(&self.db)?.check_db_version()
    }

    pub fn blocks(&self) -> Result<BlockReader> {
        BlockReader::new(&self.db)
    }

    pub fn block_stats(&self) -> Result<BlockStatsReader> {
        BlockStatsReader::new(&self.db)
    }

    pub fn txs(&self) -> Result<TxReader> {
        TxReader::new(&self.db)
    }

    pub fn script_txs(&self) -> Result<ScriptTxsReader> {
        ScriptTxsReader::new(&self.db, self.script_txs_conf.clone())
    }

    pub fn utxos(&self) -> Result<UtxosReader> {
        UtxosReader::new(&self.db)
    }

    pub fn spends(&self) -> Result<SpendsReader> {
        SpendsReader::new(&self.db)
    }

    pub fn slp(&self) -> Result<SlpReader> {
        SlpReader::new(&self.db)
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

    pub fn validate_slp_tx<'a>(
        &self,
        data: &'a IndexMemData,
        txid: &Sha256d,
        tx: &UnhashedTx,
    ) -> Result<std::result::Result<SlpValidTxData, SlpError>> {
        data.mempool_slp.validate_slp_tx(&self.db, txid, tx)
    }

    pub fn insert_block<'b>(
        &self,
        block: &Block,
        block_txs: &'b BlockTxs,
        txs: &[UnhashedTx],
        block_spent_output_fn: impl Fn(/*tx_idx:*/ usize, /*out_idx:*/ usize) -> &'b TxOutput,
        data: &mut IndexMemData,
    ) -> Result<()> {
        let mut timings = self.timings.write().unwrap();
        let block_writer = BlockWriter::new(&self.db)?;
        let block_stats_writer = BlockStatsWriter::new(&self.db)?;
        let tx_writer = TxWriter::new(&self.db)?;
        let script_txs_writer = ScriptTxsWriter::new(&self.db, self.script_txs_conf.clone())?;
        let utxo_writer = UtxosWriter::new(&self.db)?;
        let spends_writer = SpendsWriter::new(&self.db)?;
        let slp_writer = SlpWriter::new(&self.db)?;
        let mut batch = WriteBatch::default();

        let txids_fn = |idx: usize| &block_txs.txs[idx].txid;

        timings.timings.start_timer();
        block_writer.insert(&mut batch, block)?;
        timings.timings.stop_timer("blocks");

        timings.timings.start_timer();
        block_stats_writer.insert_block_txs(
            &mut batch,
            block,
            txs,
            block_txs,
            &block_spent_output_fn,
        )?;
        timings.timings.stop_timer("block_stats");

        timings.timings.start_timer();
        let first_tx_num = tx_writer.insert_block_txs(&mut batch, block_txs)?;
        timings.timings.stop_timer("txs");

        timings.timings.start_timer();
        let input_tx_nums = fetch_input_tx_nums(&self.db, first_tx_num, txids_fn, txs)?;
        timings.timings.stop_timer("fetch_input_tx_nums");

        timings.timings.start_timer();
        let script_txs_timings = script_txs_writer.insert_block_txs(
            &mut batch,
            first_tx_num,
            txs,
            &block_spent_output_fn,
            &mut data.script_txs_cache,
        )?;
        timings.timings.stop_timer("outputs");
        timings.script_txs_timings.add(&script_txs_timings);

        timings.timings.start_timer();
        let utxos_timings = utxo_writer.insert_block_txs(
            &mut batch,
            first_tx_num,
            &txids_fn,
            txs,
            &block_spent_output_fn,
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
        block_spent_output_fn: impl Fn(/*tx_idx:*/ usize, /*out_idx:*/ usize) -> &'b TxOutput,
        data: &mut IndexMemData,
    ) -> Result<()> {
        let block_writer = BlockWriter::new(&self.db)?;
        let block_stats_writer = BlockStatsWriter::new(&self.db)?;
        let tx_writer = TxWriter::new(&self.db)?;
        let conf = ScriptTxsConf { page_size: 1000 };
        let script_txs_writer = ScriptTxsWriter::new(&self.db, conf)?;
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
        block_stats_writer.delete_by_height(&mut batch, height)?;
        tx_writer.delete_block_txs(&mut batch, block.height)?;
        script_txs_writer.delete_block_txs(
            &mut batch,
            first_tx_num,
            txs,
            &block_spent_output_fn,
            &mut data.script_txs_cache,
        )?;
        utxo_writer.delete_block_txs(
            &mut batch,
            first_tx_num,
            &txids_fn,
            txs,
            block_spent_output_fn,
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
        entry: MempoolTxEntry,
    ) -> Result<()> {
        self.mempool_writer(data).insert_mempool_tx(txid, entry)?;
        Ok(())
    }

    pub fn insert_mempool_batch_txs(
        &self,
        data: &mut IndexMemData,
        txs: HashMap<Sha256d, MempoolTxEntry>,
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
            script_txs_cache: ScriptTxsWriterCache::with_capacity(outputs_capacity),
            mempool: MempoolData::default(),
            mempool_slp: MempoolSlpData::default(),
        }
    }
}

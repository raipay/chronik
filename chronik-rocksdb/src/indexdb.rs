use bitcoinsuite_core::{Script, Sha256d, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Result};
use rocksdb::WriteBatch;
use thiserror::Error;

use crate::{
    Block, BlockReader, BlockTxs, BlockWriter, Db, OutputsConf, OutputsReader, OutputsWriter,
    TxReader, TxWriter, UtxosReader, UtxosWriter,
};

pub struct IndexDb {
    db: Db,
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
        IndexDb { db }
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

    pub fn insert_block<'b>(
        &self,
        block: &Block,
        block_txs: &'b BlockTxs,
        txs: &[UnhashedTx],
        block_spent_scripts: impl IntoIterator<Item = impl IntoIterator<Item = &'b Script>>,
    ) -> Result<()> {
        let block_writer = BlockWriter::new(&self.db)?;
        let tx_writer = TxWriter::new(&self.db)?;
        let conf = OutputsConf { page_size: 1000 };
        let output_writer = OutputsWriter::new(&self.db, conf)?;
        let utxo_writer = UtxosWriter::new(&self.db)?;
        let mut batch = WriteBatch::default();
        block_writer.insert(&mut batch, block)?;
        let first_tx_num = tx_writer.insert_block_txs(&mut batch, block_txs)?;
        output_writer.insert_block_txs(&mut batch, first_tx_num, txs)?;
        let block_txids = block_txs.txs.iter().map(|tx| &tx.txid);
        utxo_writer.insert_block_txs(
            &mut batch,
            first_tx_num,
            block_txids,
            txs,
            block_spent_scripts,
        )?;
        self.db.write_batch(batch)?;
        Ok(())
    }

    pub fn delete_block<'b>(
        &self,
        block_hash: &Sha256d,
        height: i32,
        block_txids: impl IntoIterator<Item = &'b Sha256d>,
        txs: &[UnhashedTx],
        block_spent_scripts: impl IntoIterator<Item = impl IntoIterator<Item = &'b Script>>,
    ) -> Result<()> {
        let block_writer = BlockWriter::new(&self.db)?;
        let tx_writer = TxWriter::new(&self.db)?;
        let conf = OutputsConf { page_size: 1000 };
        let output_writer = OutputsWriter::new(&self.db, conf)?;
        let utxo_writer = UtxosWriter::new(&self.db)?;
        let tx_reader = TxReader::new(&self.db)?;
        let first_tx_num = tx_reader.first_tx_num_by_block(height)?.unwrap();
        let mut batch = WriteBatch::default();
        block_writer.delete_by_hash(&mut batch, block_hash)?;
        let block = self
            .blocks()?
            .by_hash(block_hash)?
            .ok_or_else(|| UnknownBlock(block_hash.clone()))?;
        tx_writer.delete_block_txs(&mut batch, block.height)?;
        output_writer.delete_block_txs(&mut batch, first_tx_num, txs)?;
        utxo_writer.delete_block_txs(
            &mut batch,
            first_tx_num,
            block_txids,
            txs,
            block_spent_scripts,
        )?;
        self.db.write_batch(batch)?;
        Ok(())
    }
}

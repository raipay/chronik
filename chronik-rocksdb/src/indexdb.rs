use bitcoinsuite_core::Sha256d;
use bitcoinsuite_error::{ErrorMeta, Result};
use rocksdb::WriteBatch;
use thiserror::Error;

use crate::{Block, BlockReader, BlockTxs, BlockWriter, Db, TxReader, TxWriter};

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

    pub fn insert_block(&self, block: &Block, block_txs: &BlockTxs) -> Result<()> {
        let block_writer = BlockWriter::new(&self.db)?;
        let tx_writer = TxWriter::new(&self.db)?;
        let mut batch = WriteBatch::default();
        block_writer.insert(&mut batch, block)?;
        tx_writer.insert_block_txs(&mut batch, block_txs)?;
        self.db.write_batch(batch)?;
        Ok(())
    }

    pub fn delete_block(&self, block_hash: &Sha256d) -> Result<()> {
        let block_writer = BlockWriter::new(&self.db)?;
        let tx_writer = TxWriter::new(&self.db)?;
        let mut batch = WriteBatch::default();
        block_writer.delete_by_hash(&mut batch, block_hash)?;
        let block = self
            .blocks()?
            .by_hash(block_hash)?
            .ok_or_else(|| UnknownBlock(block_hash.clone()))?;
        tx_writer.delete_block_txs(&mut batch, block.height)?;
        self.db.write_batch(batch)?;
        Ok(())
    }
}

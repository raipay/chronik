use bitcoinsuite_core::Sha256d;
use bitcoinsuite_error::Result;
use rocksdb::WriteBatch;

use crate::{Block, BlockReader, BlockWriter, Db};

pub struct IndexDb {
    db: Db,
}

impl IndexDb {
    pub fn new(db: Db) -> Self {
        IndexDb { db }
    }

    pub fn blocks(&self) -> Result<BlockReader> {
        BlockReader::new(&self.db)
    }

    pub fn insert_block(&self, block: &Block) -> Result<()> {
        let block_writer = BlockWriter::new(&self.db)?;
        let mut batch = WriteBatch::default();
        block_writer.insert(&mut batch, block)?;
        self.db.write_batch(batch)?;
        Ok(())
    }

    pub fn delete_block(&self, block_hash: &Sha256d) -> Result<()> {
        let block_writer = BlockWriter::new(&self.db)?;
        let mut batch = WriteBatch::default();
        block_writer.delete_by_hash(&mut batch, block_hash)?;
        self.db.write_batch(batch)?;
        Ok(())
    }
}

use std::borrow::Cow;

use bitcoinsuite_core::{Hashed, Sha256d};
use bitcoinsuite_error::{ErrorMeta, Result};
use byteorder::{BE, LE};
use rocksdb::{ColumnFamilyDescriptor, IteratorMode, Options, WriteBatch};
use thiserror::Error;
use zerocopy::{AsBytes, FromBytes, Unaligned, I32, I64, U32};

use crate::{
    data::interpret,
    index::{Index, Indexable},
    Db, CF,
};

pub const CF_BLOCKS: &str = "blocks";
pub const CF_BLOCKS_INDEX_BY_HASH: &str = "blocks_index_by_hash";
pub const SERIAL_NUM_BLOCKS: &[u8] = b"num_blocks";

// big endian so blocks are sorted ascendingly
type BlockHeightNum = I32<BE>;

pub struct BlockWriter<'a> {
    db: &'a Db,
    cf: &'a CF,
    index: Index<BlockIndexable>,
}

pub struct BlockReader<'a> {
    db: &'a Db,
    cf: &'a CF,
    index: Index<BlockIndexable>,
}

#[derive(Debug, Copy, Clone, FromBytes, AsBytes, Unaligned, PartialEq, Eq)]
#[repr(C)]
pub struct BlockHeight(BlockHeightNum);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub hash: Sha256d,
    pub prev_hash: Sha256d,
    pub height: i32,
    pub n_bits: u32,
    pub timestamp: i64,
    pub file_num: u32,
}

#[derive(Debug, Clone, FromBytes, AsBytes, Unaligned)]
#[repr(C)]
struct BlockData {
    pub hash: [u8; 32],
    pub n_bits: U32<LE>,
    pub timestamp: I64<LE>,
    pub file_num: U32<LE>,
}

struct BlockIndexable;

#[derive(Debug, Error, ErrorMeta)]
pub enum BlocksError {
    #[critical()]
    #[error("Orphan block")]
    OrphanBlock(i32),
}

use self::BlocksError::*;

impl<'a> BlockWriter<'a> {
    pub fn add_cfs(columns: &mut Vec<ColumnFamilyDescriptor>) {
        columns.push(ColumnFamilyDescriptor::new(CF_BLOCKS, Options::default()));
        Index::<BlockIndexable>::add_cfs(columns, CF_BLOCKS_INDEX_BY_HASH);
    }

    pub fn new(db: &'a Db) -> Result<Self> {
        let cf = db.cf(CF_BLOCKS)?;
        let index = block_index();
        Ok(BlockWriter { db, cf, index })
    }

    pub fn insert(&self, batch: &mut WriteBatch, block: &Block) -> Result<()> {
        let block_data = BlockData {
            hash: block.hash.byte_array().array(),
            n_bits: U32::new(block.n_bits),
            timestamp: I64::new(block.timestamp),
            file_num: U32::new(block.file_num),
        };
        let block_height = BlockHeight(BlockHeightNum::new(block.height));
        batch.put_cf(self.cf, block_height.as_bytes(), block_data.as_bytes());
        self.index
            .insert(self.db, batch, &block_height, &block_data)?;
        Ok(())
    }

    pub fn delete_by_height(&self, batch: &mut WriteBatch, height: i32) -> Result<()> {
        let height = BlockHeight(BlockHeightNum::new(height));
        let block_data = self.db.get(self.cf, height.as_bytes())?;
        let block_data = match &block_data {
            Some(block_data) => interpret::<BlockData>(block_data)?,
            None => return Ok(()),
        };
        self.delete_by_height_and_hash(batch, height.0.get(), &Sha256d::new(block_data.hash))?;
        Ok(())
    }

    pub fn delete_by_hash(&self, batch: &mut WriteBatch, block_hash: &Sha256d) -> Result<()> {
        let block_data = self
            .index
            .get(self.db, block_hash.byte_array().as_array())?;
        let (height, _) = match &block_data {
            Some(tuple) => tuple,
            None => return Ok(()),
        };
        self.delete_by_height_and_hash(batch, height.0.get(), block_hash)?;
        Ok(())
    }

    pub fn delete_by_height_and_hash(
        &self,
        batch: &mut WriteBatch,
        height: i32,
        block_hash: &Sha256d,
    ) -> Result<()> {
        let height = BlockHeight(BlockHeightNum::new(height));
        batch.delete_cf(self.cf, height.as_bytes());
        self.index
            .delete(self.db, batch, &height, block_hash.byte_array().as_array())?;
        Ok(())
    }
}

impl<'a> BlockReader<'a> {
    pub fn new(db: &'a Db) -> Result<Self> {
        let cf = db.cf(CF_BLOCKS)?;
        let index = block_index();
        Ok(BlockReader { db, cf, index })
    }

    /// The height of the most-work fully-validated chain. The genesis block has height 0
    pub fn height(&self) -> Result<i32> {
        let mut iter = self.db.rocks().iterator_cf(self.cf, IteratorMode::End);
        match iter.next() {
            Some((height_bytes, _)) => Ok(interpret::<BlockHeightNum>(&height_bytes)?.get()),
            None => Ok(-1),
        }
    }

    pub fn tip(&self) -> Result<Option<Block>> {
        let mut iter = self.db.rocks().iterator_cf(self.cf, IteratorMode::End);
        match iter.next() {
            Some((height_bytes, block_data)) => {
                let height = interpret::<BlockHeightNum>(&height_bytes)?.get();
                let block_data = interpret::<BlockData>(&block_data)?;
                let prev_block_hash = self.get_prev_hash(height)?;
                Ok(Some(Block {
                    hash: Sha256d::new(block_data.hash),
                    prev_hash: Sha256d::new(prev_block_hash),
                    height,
                    n_bits: block_data.n_bits.get(),
                    timestamp: block_data.timestamp.get(),
                    file_num: block_data.file_num.get(),
                }))
            }
            None => Ok(None),
        }
    }

    pub fn by_height(&self, height: i32) -> Result<Option<Block>> {
        let block_data = self
            .db
            .get(self.cf, BlockHeightNum::new(height).as_bytes())?;
        let block_data = match &block_data {
            Some(block_data) => interpret::<BlockData>(block_data)?,
            None => return Ok(None),
        };
        let prev_block_hash = self.get_prev_hash(height)?;
        Ok(Some(Block {
            hash: Sha256d::new(block_data.hash),
            prev_hash: Sha256d::new(prev_block_hash),
            height,
            n_bits: block_data.n_bits.get(),
            timestamp: block_data.timestamp.get(),
            file_num: block_data.file_num.get(),
        }))
    }

    pub fn by_hash(&self, block_hash: &Sha256d) -> Result<Option<Block>> {
        let block_data = self
            .index
            .get(self.db, block_hash.byte_array().as_array())?;
        let (height, block_data) = match &block_data {
            Some(tuple) => tuple,
            None => return Ok(None),
        };
        let height = height.0.get();
        let prev_block_hash = self.get_prev_hash(height)?;
        Ok(Some(Block {
            hash: block_hash.clone(),
            prev_hash: Sha256d::new(prev_block_hash),
            height,
            n_bits: block_data.n_bits.get(),
            timestamp: block_data.timestamp.get(),
            file_num: block_data.file_num.get(),
        }))
    }

    fn get_prev_hash(&self, height: i32) -> Result<[u8; 32]> {
        if height == 0 {
            return Ok([0; 32]);
        }
        let prev_block_data = self
            .db
            .get(self.cf, BlockHeightNum::new(height - 1).as_bytes())?
            .ok_or(OrphanBlock(height))?;
        let prev_block = interpret::<BlockData>(&prev_block_data)?;
        Ok(prev_block.hash)
    }
}

fn block_index() -> Index<BlockIndexable> {
    Index::new(CF_BLOCKS, CF_BLOCKS_INDEX_BY_HASH, BlockIndexable)
}

impl Indexable for BlockIndexable {
    type Hash = U32<LE>;
    type Serial = BlockHeight;
    type Key = [u8; 32];
    type Value = BlockData;

    fn hash(&self, key: &Self::Key) -> Self::Hash {
        U32::new(seahash::hash(key.as_ref()) as u32)
    }

    fn get_value_key<'a>(&self, value: &'a Self::Value) -> Cow<'a, Self::Key> {
        Cow::Borrowed(&value.hash)
    }
}

impl Ord for BlockHeight {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.get().cmp(&other.0.get())
    }
}

impl PartialOrd for BlockHeight {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod test {
    use crate::{Block, BlockReader, BlockWriter, Db};
    use bitcoinsuite_core::Sha256d;
    use bitcoinsuite_error::Result;
    use pretty_assertions::assert_eq;
    use rocksdb::WriteBatch;

    #[test]
    fn test_blocks() -> Result<()> {
        let tempdir = tempdir::TempDir::new("slp-indexer-rocks--blocks")?;
        let db = Db::open(tempdir.path())?;
        let writer = BlockWriter::new(&db)?;
        let reader = BlockReader::new(&db)?;
        let block0 = Block {
            hash: Sha256d::new([44; 32]),
            prev_hash: Sha256d::new([0; 32]),
            height: 0,
            n_bits: 0x1c100000,
            timestamp: 1600000000,
            file_num: 6,
        };
        let block1 = Block {
            hash: Sha256d::new([22; 32]),
            prev_hash: Sha256d::new([44; 32]),
            height: 1,
            n_bits: 0x1c100001,
            timestamp: 1600000001,
            file_num: 7,
        };
        assert_eq!(reader.by_height(0)?, None);
        assert_eq!(reader.height()?, -1);
        assert_eq!(reader.tip()?, None);
        {
            let mut batch = WriteBatch::default();
            writer.insert(&mut batch, &block0)?;
            db.write_batch(batch)?;
            assert_eq!(reader.height()?, 0);
            assert_eq!(reader.tip()?.as_ref(), Some(&block0));
            assert_eq!(reader.by_height(-1)?, None);
            assert_eq!(reader.by_height(0)?.as_ref(), Some(&block0));
            assert_eq!(reader.by_height(1)?, None);
            assert_eq!(reader.by_height(2)?, None);
            assert_eq!(reader.by_hash(&Sha256d::new([0; 32]))?, None);
            assert_eq!(
                reader.by_hash(&Sha256d::new([44; 32]))?.as_ref(),
                Some(&block0)
            );
            assert_eq!(reader.by_hash(&Sha256d::new([22; 32]))?, None);
        }
        {
            let mut batch = WriteBatch::default();
            writer.insert(&mut batch, &block1)?;
            db.write_batch(batch)?;
            assert_eq!(reader.height()?, 1);
            assert_eq!(reader.tip()?.as_ref(), Some(&block1));
            assert_eq!(reader.by_height(-1)?, None);
            assert_eq!(reader.by_height(0)?.as_ref(), Some(&block0));
            assert_eq!(reader.by_height(1)?.as_ref(), Some(&block1));
            assert_eq!(reader.by_height(2)?, None);
            assert_eq!(reader.by_hash(&Sha256d::new([0; 32]))?, None);
            assert_eq!(
                reader.by_hash(&Sha256d::new([44; 32]))?.as_ref(),
                Some(&block0)
            );
            assert_eq!(
                reader.by_hash(&Sha256d::new([22; 32]))?.as_ref(),
                Some(&block1)
            );
        }
        {
            let mut batch = WriteBatch::default();
            writer.delete_by_height(&mut batch, 1)?;
            db.write_batch(batch)?;
            assert_eq!(reader.height()?, 0);
            assert_eq!(reader.tip()?.as_ref(), Some(&block0));
            assert_eq!(reader.by_height(-1)?, None);
            assert_eq!(reader.by_height(0)?.as_ref(), Some(&block0));
            assert_eq!(reader.by_height(1)?, None);
        }
        {
            let mut batch = WriteBatch::default();
            writer.delete_by_hash(&mut batch, &Sha256d::new([44; 32]))?;
            db.write_batch(batch)?;
            assert_eq!(reader.height()?, -1);
            assert_eq!(reader.tip()?, None);
            assert_eq!(reader.by_height(-1)?, None);
            assert_eq!(reader.by_height(0)?, None);
            assert_eq!(reader.by_height(1)?, None);
        }

        Ok(())
    }
}

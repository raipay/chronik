use std::borrow::Cow;

use bitcoinsuite_core::{Hashed, Sha256d};
use bitcoinsuite_error::{ErrorMeta, Result};
use byteorder::{BE, LE};
use rocksdb::{ColumnFamilyDescriptor, Direction, IteratorMode, Options, WriteBatch};
use thiserror::Error;
use zerocopy::{AsBytes, FromBytes, Unaligned, I64, U32, U64};

use crate::{
    data::interpret,
    index::{Index, Indexable},
    BlockHeight, BlockHeightZC, Db, CF,
};

pub const CF_TXS: &str = "txs";
pub const CF_BLOCK_BY_FIRST_TX: &str = "block_by_first_tx";
pub const CF_FIRST_TX_BY_BLOCK: &str = "first_tx_by_block";
pub const CF_TX_INDEX_BY_TXID: &str = "tx_index_by_txid";

pub type TxNum = u64;
// big endian so txs are sorted ascendingly
pub type TxNumZC = U64<BE>;

#[derive(Debug, Copy, Clone, FromBytes, AsBytes, Unaligned, PartialEq, Eq)]
#[repr(C)]
pub struct TxNumOrd(pub TxNumZC);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TxEntry {
    pub txid: Sha256d,
    pub data_pos: u32,
    pub tx_size: u32,
    pub undo_pos: u32,
    pub undo_size: u32,
    pub time_first_seen: i64,
    pub is_coinbase: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockTx {
    pub entry: TxEntry,
    pub block_height: BlockHeight,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockTxs {
    pub txs: Vec<TxEntry>,
    pub block_height: BlockHeight,
}

#[derive(Debug, Clone, FromBytes, AsBytes, Unaligned)]
#[repr(C)]
struct TxData {
    pub txid: [u8; 32],
    pub data_pos: U32<LE>,
    pub tx_size: U32<LE>,
    pub undo_pos: U32<LE>,
    pub undo_size: U32<LE>,
    pub time_first_seen: I64<LE>,
    pub is_coinbase: u8,
}

pub struct TxWriter<'a> {
    db: &'a Db,
    cf_txs: &'a CF,
    cf_block_by_first_tx: &'a CF,
    cf_first_tx_by_block: &'a CF,
    txid_index: Index<TxIndexable>,
}

pub struct TxReader<'a> {
    db: &'a Db,
    txid_index: Index<TxIndexable>,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum TxsError {
    #[critical()]
    #[error("Inconsistent tx index")]
    InconsistentTxIndex,

    #[critical()]
    #[error("Txs for block not found")]
    NoSuchBlock,
}

use self::TxsError::*;

struct TxIndexable;

fn _assert_txs_reader_send_sync() {
    _assert_send_sync(|_: TxReader| ());
}
fn _assert_send_sync<T: Send + Sync>(_: impl Fn(T)) {}

impl<'a> TxWriter<'a> {
    pub fn add_cfs(columns: &mut Vec<ColumnFamilyDescriptor>) {
        columns.push(ColumnFamilyDescriptor::new(CF_TXS, Options::default()));
        columns.push(ColumnFamilyDescriptor::new(
            CF_BLOCK_BY_FIRST_TX,
            Options::default(),
        ));
        columns.push(ColumnFamilyDescriptor::new(
            CF_FIRST_TX_BY_BLOCK,
            Options::default(),
        ));
        Index::<TxIndexable>::add_cfs(columns, CF_TX_INDEX_BY_TXID);
    }

    pub fn new(db: &'a Db) -> Result<Self> {
        let cf_txs = db.cf(CF_TXS)?;
        let cf_block_by_first_tx = db.cf(CF_BLOCK_BY_FIRST_TX)?;
        let cf_first_tx_by_block = db.cf(CF_FIRST_TX_BY_BLOCK)?;
        Ok(TxWriter {
            db,
            cf_txs,
            cf_block_by_first_tx,
            cf_first_tx_by_block,
            txid_index: txid_index(),
        })
    }

    pub fn insert_block_txs(&self, batch: &mut WriteBatch, block_txs: &BlockTxs) -> Result<TxNum> {
        let mut last_tx_num_iterator = self.db.rocks().iterator_cf(self.cf_txs, IteratorMode::End);
        let mut next_tx_num = match last_tx_num_iterator.next() {
            Some((tx_num, _)) => interpret::<TxNumZC>(&tx_num)?.get() + 1,
            None => 0,
        };
        let first_new_tx = next_tx_num;
        batch.put_cf(
            self.cf_block_by_first_tx,
            TxNumZC::new(first_new_tx).as_bytes(),
            BlockHeightZC::new(block_txs.block_height).as_bytes(),
        );
        batch.put_cf(
            self.cf_first_tx_by_block,
            BlockHeightZC::new(block_txs.block_height).as_bytes(),
            TxNumZC::new(first_new_tx).as_bytes(),
        );
        for tx in &block_txs.txs {
            let tx_data = TxData {
                txid: tx.txid.byte_array().array(),
                data_pos: U32::new(tx.data_pos),
                tx_size: U32::new(tx.tx_size),
                undo_pos: U32::new(tx.undo_pos),
                undo_size: U32::new(tx.undo_size),
                time_first_seen: I64::new(tx.time_first_seen),
                is_coinbase: tx.is_coinbase as u8,
            };
            let tx_num = TxNumOrd(TxNumZC::new(next_tx_num));
            batch.put_cf(self.cf_txs, tx_num.as_bytes(), tx_data.as_bytes());
            self.txid_index.insert(self.db, batch, &tx_num, &tx_data)?;
            next_tx_num += 1;
        }
        Ok(first_new_tx)
    }

    pub fn delete_block_txs(
        &self,
        batch: &mut WriteBatch,
        block_height: BlockHeight,
    ) -> Result<()> {
        let block_height_inner = BlockHeightZC::new(block_height);
        let first_tx_num = self
            .db
            .get(self.cf_first_tx_by_block, block_height_inner.as_bytes())?
            .ok_or(NoSuchBlock)?;
        let next_block_height_inner = BlockHeightZC::new(block_height + 1);
        let end_tx_num = self
            .db
            .get(
                self.cf_first_tx_by_block,
                next_block_height_inner.as_bytes(),
            )?
            .map(|end_tx_num| -> Result<_> { Ok(interpret::<TxNumZC>(&end_tx_num)?.get()) })
            .transpose()?;
        let iterator = self.db.rocks().iterator_cf(
            self.cf_txs,
            IteratorMode::From(&first_tx_num, Direction::Forward),
        );
        for (tx_num, tx_data) in iterator {
            let tx_num = interpret::<TxNumOrd>(&tx_num)?;
            let tx_data = interpret::<TxData>(&tx_data)?;
            if let Some(end_tx_num) = end_tx_num {
                if tx_num.0.get() >= end_tx_num {
                    break;
                }
            }
            batch.delete_cf(self.cf_txs, tx_num.as_bytes());
            self.txid_index
                .delete(self.db, batch, tx_num, &tx_data.txid)?;
        }
        batch.delete_cf(self.cf_block_by_first_tx, &first_tx_num);
        batch.delete_cf(self.cf_first_tx_by_block, block_height_inner.as_bytes());
        Ok(())
    }
}

impl<'a> TxReader<'a> {
    pub fn new(db: &'a Db) -> Result<Self> {
        let _ = db.cf(CF_TXS)?;
        let _ = db.cf(CF_BLOCK_BY_FIRST_TX)?;
        let _ = db.cf(CF_FIRST_TX_BY_BLOCK)?;
        Ok(TxReader {
            db,
            txid_index: txid_index(),
        })
    }

    pub fn by_txid(&self, txid: &Sha256d) -> Result<Option<BlockTx>> {
        match self.tx_and_num_by_txid(txid)? {
            Some((_, block_tx)) => Ok(Some(block_tx)),
            None => Ok(None),
        }
    }

    pub fn tx_and_num_by_txid(&self, txid: &Sha256d) -> Result<Option<(TxNum, BlockTx)>> {
        let (tx_num, tx_data) = match self.txid_index.get(self.db, txid.byte_array().as_array())? {
            Some(tuple) => tuple,
            None => return Ok(None),
        };
        let block_height = self.block_height_by_tx_num(tx_num.0)?;
        Ok(Some((
            tx_num.0.get(),
            BlockTx {
                entry: TxEntry {
                    txid: Sha256d::new(tx_data.txid),
                    data_pos: tx_data.data_pos.get(),
                    tx_size: tx_data.tx_size.get(),
                    undo_pos: tx_data.undo_pos.get(),
                    undo_size: tx_data.undo_size.get(),
                    time_first_seen: tx_data.time_first_seen.get(),
                    is_coinbase: tx_data.is_coinbase != 0,
                },
                block_height,
            },
        )))
    }

    fn block_height_by_tx_num(&self, tx_num: TxNumZC) -> Result<BlockHeight> {
        let mut tx_block = self.db.rocks().iterator_cf(
            self.cf_tx_block(),
            IteratorMode::From(tx_num.as_bytes(), Direction::Reverse),
        );
        let block_height = match tx_block.next() {
            Some((_, block_height)) => interpret::<BlockHeightZC>(&block_height)?.get(),
            None => return Err(InconsistentTxIndex.into()),
        };
        Ok(block_height)
    }

    pub fn tx_num_by_txid(&self, txid: &Sha256d) -> Result<Option<TxNum>> {
        match self.txid_index.get(self.db, txid.byte_array().as_array())? {
            Some((tx_num, _)) => Ok(Some(interpret::<TxNumZC>(tx_num.as_bytes())?.get())),
            None => Ok(None),
        }
    }

    pub fn by_tx_num(&self, tx_num: TxNum) -> Result<Option<BlockTx>> {
        let tx_num = TxNumZC::new(tx_num);
        let tx_entry = match self.db.get(self.cf_txs(), tx_num.as_bytes())? {
            Some(entry) => entry,
            None => return Ok(None),
        };
        let block_height = self.block_height_by_tx_num(tx_num)?;
        let tx_data = interpret::<TxData>(&tx_entry)?;
        Ok(Some(BlockTx {
            entry: TxEntry {
                txid: Sha256d::new(tx_data.txid),
                data_pos: tx_data.data_pos.get(),
                tx_size: tx_data.tx_size.get(),
                undo_pos: tx_data.undo_pos.get(),
                undo_size: tx_data.undo_size.get(),
                time_first_seen: tx_data.time_first_seen.get(),
                is_coinbase: tx_data.is_coinbase != 0,
            },
            block_height,
        }))
    }

    pub fn txid_by_tx_num(&self, tx_num: TxNum) -> Result<Option<Sha256d>> {
        let tx_num = TxNumZC::new(tx_num);
        let tx_entry = match self.db.get(self.cf_txs(), tx_num.as_bytes())? {
            Some(entry) => entry,
            None => return Ok(None),
        };
        let tx_data = interpret::<TxData>(&tx_entry)?;
        Ok(Some(Sha256d::new(tx_data.txid)))
    }

    pub fn first_tx_num_by_block(&self, block_height: BlockHeight) -> Result<Option<TxNum>> {
        let block_height_inner = BlockHeightZC::new(block_height);
        let first_tx_num = match self
            .db
            .get(self.cf_first_tx_by_block(), block_height_inner.as_bytes())?
        {
            Some(first_tx_num) => first_tx_num,
            None => return Ok(None),
        };
        let tx_num = interpret::<TxNumZC>(&first_tx_num)?;
        Ok(Some(tx_num.get()))
    }

    pub fn last_tx_num(&self) -> Result<Option<TxNum>> {
        let mut iter = self
            .db
            .rocks()
            .iterator_cf(self.cf_txs(), IteratorMode::End);
        match iter.next() {
            Some((key, _)) => {
                let tx_num = interpret::<TxNumZC>(&key)?;
                Ok(Some(tx_num.get()))
            }
            None => Ok(None),
        }
    }

    fn cf_txs(&self) -> &CF {
        self.db.cf(CF_TXS).unwrap()
    }

    fn cf_tx_block(&self) -> &CF {
        self.db.cf(CF_BLOCK_BY_FIRST_TX).unwrap()
    }

    fn cf_first_tx_by_block(&self) -> &CF {
        self.db.cf(CF_FIRST_TX_BY_BLOCK).unwrap()
    }
}

fn txid_index() -> Index<TxIndexable> {
    Index::new(CF_TXS, CF_TX_INDEX_BY_TXID, TxIndexable)
}

impl Ord for TxNumOrd {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.get().cmp(&other.0.get())
    }
}

impl PartialOrd for TxNumOrd {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Indexable for TxIndexable {
    type Hash = U32<LE>;
    type Serial = TxNumOrd;
    type Key = [u8; 32];
    type Value = TxData;
    fn hash(&self, key: &Self::Key) -> Self::Hash {
        U32::new(seahash::hash(key) as u32)
    }
    fn get_value_key<'a>(&self, value: &'a Self::Value) -> Cow<'a, Self::Key> {
        Cow::Borrowed(&value.txid)
    }
}

#[cfg(test)]
mod test {
    use crate::{BlockTx, BlockTxs, Db, TxEntry, TxReader, TxWriter};
    use bitcoinsuite_core::Sha256d;
    use bitcoinsuite_error::Result;
    use pretty_assertions::assert_eq;
    use rocksdb::WriteBatch;

    #[test]
    fn test_txs() -> Result<()> {
        bitcoinsuite_error::install()?;
        let tempdir = tempdir::TempDir::new("slp-indexer-rocks--blocks")?;
        let db = Db::open(tempdir.path())?;
        let tx_writer = TxWriter::new(&db)?;
        let tx_reader = TxReader::new(&db)?;
        let tx1 = TxEntry {
            txid: Sha256d::new([1; 32]),
            data_pos: 100,
            tx_size: 1000,
            undo_pos: 10,
            undo_size: 1,
            time_first_seen: 123456,
            is_coinbase: true,
        };
        let block_tx1 = BlockTx {
            entry: tx1.clone(),
            block_height: 0,
        };
        assert_eq!(tx_reader.last_tx_num()?, None);
        {
            // insert genesis tx
            let block_txs = BlockTxs {
                block_height: 0,
                txs: vec![tx1],
            };
            let mut batch = WriteBatch::default();
            tx_writer.insert_block_txs(&mut batch, &block_txs)?;
            db.write_batch(batch)?;
            let tx_reader = TxReader::new(&db)?;
            assert_eq!(tx_reader.first_tx_num_by_block(0)?, Some(0));
            assert_eq!(tx_reader.first_tx_num_by_block(1)?, None);
            assert_eq!(tx_reader.last_tx_num()?, Some(0));
            assert_eq!(tx_reader.by_txid(&Sha256d::new([0; 32]))?, None);
            assert_eq!(tx_reader.tx_num_by_txid(&Sha256d::new([0; 32]))?, None);
            assert_eq!(
                tx_reader.by_txid(&Sha256d::new([1; 32]))?,
                Some(block_tx1.clone())
            );
            assert_eq!(tx_reader.by_tx_num(0)?, Some(block_tx1.clone()));
            assert_eq!(tx_reader.tx_num_by_txid(&Sha256d::new([1; 32]))?, Some(0));
        }
        let tx2 = TxEntry {
            txid: Sha256d::new([2; 32]),
            data_pos: 200,
            tx_size: 2000,
            undo_pos: 20,
            undo_size: 2,
            time_first_seen: 234567,
            is_coinbase: false,
        };
        let block_tx2 = BlockTx {
            entry: tx2.clone(),
            block_height: 1,
        };
        let tx3 = TxEntry {
            txid: Sha256d::new([3; 32]),
            data_pos: 300,
            tx_size: 3000,
            undo_pos: 30,
            undo_size: 3,
            time_first_seen: 345678,
            is_coinbase: false,
        };
        let block_tx3 = BlockTx {
            entry: tx3.clone(),
            block_height: 1,
        };
        {
            // insert 2 more txs
            let block_txs = BlockTxs {
                block_height: 1,
                txs: vec![tx2, tx3],
            };
            let mut batch = WriteBatch::default();
            tx_writer.insert_block_txs(&mut batch, &block_txs)?;
            db.write_batch(batch)?;
            assert_eq!(tx_reader.first_tx_num_by_block(0)?, Some(0));
            assert_eq!(tx_reader.first_tx_num_by_block(1)?, Some(1));
            assert_eq!(tx_reader.first_tx_num_by_block(2)?, None);
            assert_eq!(tx_reader.last_tx_num()?, Some(2));
            assert_eq!(tx_reader.by_txid(&Sha256d::new([0; 32]))?, None);
            assert_eq!(tx_reader.tx_num_by_txid(&Sha256d::new([0; 32]))?, None);
            assert_eq!(
                tx_reader.by_txid(&Sha256d::new([1; 32]))?,
                Some(block_tx1.clone()),
            );
            assert_eq!(tx_reader.tx_num_by_txid(&Sha256d::new([1; 32]))?, Some(0));
            assert_eq!(tx_reader.by_tx_num(0)?, Some(block_tx1.clone()));
            assert_eq!(
                tx_reader.by_txid(&Sha256d::new([2; 32]))?,
                Some(block_tx2.clone()),
            );
            assert_eq!(tx_reader.tx_num_by_txid(&Sha256d::new([2; 32]))?, Some(1));
            assert_eq!(tx_reader.by_tx_num(1)?, Some(block_tx2));
            assert_eq!(
                tx_reader.by_txid(&Sha256d::new([3; 32]))?,
                Some(block_tx3.clone()),
            );
            assert_eq!(tx_reader.tx_num_by_txid(&Sha256d::new([3; 32]))?, Some(2));
            assert_eq!(tx_reader.by_tx_num(2)?, Some(block_tx3));
        }
        {
            // delete latest block
            let mut batch = WriteBatch::default();
            tx_writer.delete_block_txs(&mut batch, 1)?;
            db.write_batch(batch)?;
            assert_eq!(tx_reader.first_tx_num_by_block(0)?, Some(0));
            assert_eq!(tx_reader.first_tx_num_by_block(1)?, None);
            assert_eq!(tx_reader.last_tx_num()?, Some(0));
            assert_eq!(tx_reader.by_txid(&Sha256d::new([0; 32]))?, None);
            assert_eq!(
                tx_reader.by_txid(&Sha256d::new([1; 32]))?,
                Some(block_tx1.clone())
            );
            assert_eq!(tx_reader.by_tx_num(0)?, Some(block_tx1));
            assert_eq!(tx_reader.by_txid(&Sha256d::new([2; 32]))?, None);
            assert_eq!(tx_reader.by_tx_num(1)?, None);
            assert_eq!(tx_reader.by_txid(&Sha256d::new([3; 32]))?, None);
            assert_eq!(tx_reader.by_tx_num(2)?, None);
        }
        let tx2 = TxEntry {
            txid: Sha256d::new([102; 32]),
            data_pos: 200,
            tx_size: 2000,
            undo_pos: 20,
            undo_size: 2,
            time_first_seen: 234567,
            is_coinbase: false,
        };
        let block_tx2 = BlockTx {
            entry: tx2.clone(),
            block_height: 1,
        };
        let tx3 = TxEntry {
            txid: Sha256d::new([103; 32]),
            data_pos: 300,
            tx_size: 3000,
            undo_pos: 30,
            undo_size: 3,
            time_first_seen: 345678,
            is_coinbase: false,
        };
        let block_tx3 = BlockTx {
            entry: tx3.clone(),
            block_height: 1,
        };
        {
            // Add new latest block and then delete genesis block
            // This should never happen in practice, but we test for it so we have consistent
            // behavior in this case.
            let block_txs = BlockTxs {
                block_height: 1,
                txs: vec![tx2, tx3],
            };
            let mut batch = WriteBatch::default();
            tx_writer.insert_block_txs(&mut batch, &block_txs)?;
            db.write_batch(batch)?;

            let mut batch = WriteBatch::default();
            tx_writer.delete_block_txs(&mut batch, 0)?;
            db.write_batch(batch)?;

            assert_eq!(tx_reader.first_tx_num_by_block(0)?, None);
            assert_eq!(tx_reader.first_tx_num_by_block(1)?, Some(1));
            assert_eq!(tx_reader.first_tx_num_by_block(2)?, None);
            assert_eq!(tx_reader.by_txid(&Sha256d::new([0; 32]))?, None);
            assert_eq!(tx_reader.by_txid(&Sha256d::new([1; 32]))?, None);
            assert_eq!(
                tx_reader.by_txid(&Sha256d::new([102; 32]))?,
                Some(block_tx2.clone()),
            );
            assert_eq!(tx_reader.tx_num_by_txid(&Sha256d::new([102; 32]))?, Some(1));
            assert_eq!(tx_reader.by_tx_num(1)?, Some(block_tx2));
            assert_eq!(
                tx_reader.by_txid(&Sha256d::new([103; 32]))?,
                Some(block_tx3.clone()),
            );
            assert_eq!(tx_reader.tx_num_by_txid(&Sha256d::new([103; 32]))?, Some(2));
            assert_eq!(tx_reader.by_tx_num(2)?, Some(block_tx3));
        }
        Ok(())
    }
}

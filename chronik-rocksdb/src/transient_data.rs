use std::path::Path;

use bitcoinsuite_core::Hashed;
use bitcoinsuite_error::{ErrorMeta, Result, WrapErr};
use prost::Message;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rocksdb::{ColumnFamilyDescriptor, IteratorMode, Options};
use thiserror::Error;
use zerocopy::AsBytes;

use crate::{data::interpret, proto, BlockHeight, BlockHeightZC, Db, TxNum, TxReader, CF};

pub const CF_TRANSIENT_BLOCK_DATA: &str = "transient_block_data";

pub struct TransientData {
    db: rocksdb::DB,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum TransientDataError {
    #[critical()]
    #[error("RocksDB error")]
    RocksDb,

    #[critical()]
    #[error("Inconsistent db, invalid protobuf")]
    InvalidProtobuf,

    #[critical()]
    #[error("Inconsistent db, block height doesn't exist: {0}")]
    NoSuchBlock(BlockHeight),

    #[critical()]
    #[error("Inconsistent db, tx_num doesn't exist: {0}")]
    NoSuchTxNum(TxNum),
}

use self::TransientDataError::*;

impl TransientData {
    pub fn open(db_path: &Path) -> Result<Self> {
        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);
        let cfs = vec![ColumnFamilyDescriptor::new(
            CF_TRANSIENT_BLOCK_DATA,
            Options::default(),
        )];
        let db = rocksdb::DB::open_cf_descriptors(&db_options, db_path, cfs).wrap_err(RocksDb)?;
        Ok(TransientData { db })
    }

    pub fn update_block(&self, db: &Db, block_height: BlockHeight) -> Result<()> {
        let tx_reader = TxReader::new(db)?;
        let first_tx_num = tx_reader
            .first_tx_num_by_block(block_height)?
            .ok_or(NoSuchBlock(block_height))?;
        let last_tx_num = match tx_reader.first_tx_num_by_block(block_height + 1)? {
            Some(last_tx_num) => last_tx_num,
            None => tx_reader.last_tx_num()?.unwrap_or(0) + 1,
        };
        let tx_data = (first_tx_num..last_tx_num)
            .into_par_iter()
            .map(|tx_num| {
                let tx = tx_reader.by_tx_num(tx_num)?.ok_or(NoSuchTxNum(tx_num))?;
                if tx.entry.time_first_seen == 0 {
                    return Ok(None);
                }
                let txid_hash = seahash::hash(tx.entry.txid.as_slice());
                Ok(Some(proto::TransientTxData {
                    txid_hash,
                    time_first_seen: tx.entry.time_first_seen,
                }))
            })
            .filter_map(|tx_data| tx_data.transpose())
            .collect::<Result<Vec<_>>>()?;
        let block_data = proto::TransientBlockData { tx_data };
        self.db
            .put_cf(
                self.cf_transient_block_data(),
                BlockHeightZC::new(block_height).as_bytes(),
                &block_data.encode_to_vec(),
            )
            .wrap_err(RocksDb)?;
        Ok(())
    }

    pub fn delete_block(&self, block_height: BlockHeight) -> Result<()> {
        self.db
            .delete_cf(
                self.cf_transient_block_data(),
                BlockHeightZC::new(block_height).as_bytes(),
            )
            .wrap_err(RocksDb)?;
        Ok(())
    }

    pub fn read_block(
        &self,
        block_height: BlockHeight,
    ) -> Result<Option<proto::TransientBlockData>> {
        let block_data = self
            .db
            .get_pinned_cf(
                self.cf_transient_block_data(),
                BlockHeightZC::new(block_height).as_bytes(),
            )
            .wrap_err(RocksDb)?;
        let block_data = match block_data {
            Some(block_data) => block_data,
            None => return Ok(None),
        };
        let block_data =
            proto::TransientBlockData::decode(block_data.as_ref()).wrap_err(InvalidProtobuf)?;
        Ok(Some(block_data))
    }

    pub fn next_block_height(&self) -> Result<BlockHeight> {
        let mut iter = self
            .db
            .iterator_cf(self.cf_transient_block_data(), IteratorMode::End);
        match iter.next() {
            Some((key, _)) => Ok(interpret::<BlockHeightZC>(&key)?.get()),
            None => Ok(0),
        }
    }

    fn cf_transient_block_data(&self) -> &CF {
        self.db
            .cf_handle(CF_TRANSIENT_BLOCK_DATA)
            .expect("Missing column family 'cf_transient_block_data'")
    }
}

#[cfg(test)]
mod test {
    use bitcoinsuite_core::Sha256d;
    use bitcoinsuite_error::Result;
    use pretty_assertions::assert_eq;
    use rocksdb::WriteBatch;

    use crate::{proto, BlockTxs, Db, TransientData, TxEntry, TxWriter};

    #[test]
    fn test_txs() -> Result<()> {
        bitcoinsuite_error::install()?;
        let tempdir = tempdir::TempDir::new("slp-indexer-rocks--transient-data")?;
        let db = Db::open(tempdir.path().join("data"))?;
        let transient_db = TransientData::open(&tempdir.path().join("transient-data"))?;
        let tx_writer = TxWriter::new(&db)?;
        let tx1 = TxEntry {
            txid: Sha256d::new([1; 32]),
            time_first_seen: 123456,
            ..Default::default()
        };
        assert_eq!(transient_db.read_block(0)?, None);
        {
            // insert genesis tx
            let block_txs = BlockTxs {
                block_height: 0,
                txs: vec![tx1],
            };
            let mut batch = WriteBatch::default();
            tx_writer.insert_block_txs(&mut batch, &block_txs)?;
            db.write_batch(batch)?;
            transient_db.update_block(&db, 0)?;
            assert_eq!(
                transient_db.read_block(0)?,
                Some(proto::TransientBlockData {
                    tx_data: vec![proto::TransientTxData {
                        txid_hash: seahash::hash(&[1; 32]),
                        time_first_seen: 123456,
                    }],
                }),
            );
        }
        let tx2 = TxEntry {
            txid: Sha256d::new([2; 32]),
            time_first_seen: 0,
            ..Default::default()
        };
        let tx3 = TxEntry {
            txid: Sha256d::new([3; 32]),
            time_first_seen: 345678,
            ..Default::default()
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
            transient_db.update_block(&db, 1)?;
            assert_eq!(
                transient_db.read_block(1)?,
                Some(proto::TransientBlockData {
                    tx_data: vec![proto::TransientTxData {
                        txid_hash: seahash::hash(&[3; 32]),
                        time_first_seen: 345678,
                    }],
                }),
            );
        }
        {
            // delete latest block
            let mut batch = WriteBatch::default();
            tx_writer.delete_block_txs(&mut batch, 1)?;
            db.write_batch(batch)?;
            transient_db.delete_block(1)?;
            assert_eq!(transient_db.read_block(1)?, None);
        }
        let tx2 = TxEntry {
            txid: Sha256d::new([102; 32]),
            time_first_seen: 234567,
            ..Default::default()
        };
        let tx3 = TxEntry {
            txid: Sha256d::new([103; 32]),
            time_first_seen: 0,
            ..Default::default()
        };
        {
            // add 2 txs back in
            let block_txs = BlockTxs {
                block_height: 1,
                txs: vec![tx2, tx3],
            };
            let mut batch = WriteBatch::default();
            tx_writer.insert_block_txs(&mut batch, &block_txs)?;
            db.write_batch(batch)?;
            transient_db.update_block(&db, 1)?;
            assert_eq!(
                transient_db.read_block(1)?,
                Some(proto::TransientBlockData {
                    tx_data: vec![proto::TransientTxData {
                        txid_hash: seahash::hash(&[102; 32]),
                        time_first_seen: 234567,
                    }],
                }),
            );
        }
        Ok(())
    }
}

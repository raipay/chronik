use std::path::Path;

use rocksdb::{ColumnFamily, ColumnFamilyDescriptor, Options, WriteBatch};

use crate::{BlockWriter, OutputsWriter, TxWriter, UtxosWriter};
use bitcoinsuite_error::{ErrorMeta, Result, WrapErr};
use thiserror::Error;

pub type CF = ColumnFamily;

pub struct Db {
    db: rocksdb::DB,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum DbError {
    #[critical()]
    #[error("Column family {0} doesn't exist")]
    NoSuchColumnFamily(String),

    #[critical()]
    #[error("RocksDB error")]
    RocksDb,
}

use self::DbError::*;

impl Db {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut cfs = Vec::new();
        BlockWriter::add_cfs(&mut cfs);
        TxWriter::add_cfs(&mut cfs);
        OutputsWriter::add_cfs(&mut cfs);
        UtxosWriter::add_cfs(&mut cfs);
        Self::open_with_cfs(path, cfs)
    }

    pub fn open_with_cfs(path: impl AsRef<Path>, cfs: Vec<ColumnFamilyDescriptor>) -> Result<Self> {
        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);
        let db = rocksdb::DB::open_cf_descriptors(&db_options, path, cfs).wrap_err(RocksDb)?;
        Ok(Db { db })
    }

    pub fn rocks(&self) -> &rocksdb::DB {
        &self.db
    }

    pub fn cf(&self, name: &str) -> Result<&CF> {
        Ok(self
            .db
            .cf_handle(name)
            .ok_or_else(|| NoSuchColumnFamily(name.to_string()))?)
    }

    pub fn get(&self, cf: &CF, key: impl AsRef<[u8]>) -> Result<Option<rocksdb::DBPinnableSlice>> {
        self.db.get_pinned_cf(cf, key).wrap_err(RocksDb)
    }

    pub fn write_batch(&self, batch: WriteBatch) -> Result<()> {
        self.db.write(batch).wrap_err(RocksDb)
    }
}

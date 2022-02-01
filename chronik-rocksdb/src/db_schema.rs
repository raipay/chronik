use bitcoinsuite_error::{ErrorMeta, Result, WrapErr};
use byteorder::BE;
use rocksdb::ColumnFamilyDescriptor;
use thiserror::Error;
use zerocopy::{AsBytes, U64};

use crate::{data::interpret, Db, DbError, CF};

pub const CF_SCHEMA: &str = "schema";

pub const DB_SCHEMA_VERSION: DbVersionNum = 100;

const FIELD_VERSION: &[u8] = b"version";

pub type DbVersionNum = u64;
pub type DbVersionNumZC = U64<BE>;

pub struct DbSchema<'a> {
    db: &'a Db,
    cf_schema: &'a CF,
}

#[derive(Debug, Error, ErrorMeta, PartialEq, Eq)]
pub enum DbSchemaError {
    #[critical()]
    #[error(
        "Database version too old. The database is on version {actual}, but this indexer is on \
         version {actual}. Consider re-indexing (will lose precise first-seen timestamps) or \
         upgrading the database."
    )]
    DbTooOld {
        expected: DbVersionNum,
        actual: DbVersionNum,
    },

    #[critical()]
    #[error(
        "Database version too new. The database is on version {actual}, but this indexer is on \
         version {actual}. Consider upgrading the indexer."
    )]
    DbTooNew {
        expected: DbVersionNum,
        actual: DbVersionNum,
    },
}

use self::DbSchemaError::*;

impl<'a> DbSchema<'a> {
    pub fn add_cfs(columns: &mut Vec<ColumnFamilyDescriptor>) {
        let options = rocksdb::Options::default();
        columns.push(ColumnFamilyDescriptor::new(CF_SCHEMA, options));
    }

    pub fn new(db: &'a Db) -> Result<Self> {
        let cf_schema = db.cf(CF_SCHEMA)?;
        Ok(DbSchema { db, cf_schema })
    }

    pub fn check_db_version(&self) -> Result<()> {
        let version_slice = self.db.get(self.cf_schema, FIELD_VERSION)?;
        match version_slice {
            Some(version_slice) => {
                let version = interpret::<DbVersionNumZC>(&version_slice)?.get();
                if version < DB_SCHEMA_VERSION {
                    return Err(DbTooOld {
                        actual: version,
                        expected: DB_SCHEMA_VERSION,
                    }
                    .into());
                }
                if version > DB_SCHEMA_VERSION {
                    return Err(DbTooNew {
                        actual: version,
                        expected: DB_SCHEMA_VERSION,
                    }
                    .into());
                }
            }
            None => {
                let version = DbVersionNumZC::new(DB_SCHEMA_VERSION);
                self.db
                    .rocks()
                    .put_cf(self.cf_schema, FIELD_VERSION, version.as_bytes())
                    .wrap_err(DbError::RocksDb)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::{Db, DbSchema, DbSchemaError, DbVersionNumZC, CF_SCHEMA, DB_SCHEMA_VERSION};
    use bitcoinsuite_error::Result;
    use pretty_assertions::assert_eq;
    use zerocopy::AsBytes;

    use super::FIELD_VERSION;

    #[test]
    fn test_schema() -> Result<()> {
        bitcoinsuite_error::install()?;
        let tempdir = tempdir::TempDir::new("slp-indexer-rocks--schema")?;
        let db = Db::open(tempdir.path())?;
        let db_schema = DbSchema::new(&db)?;
        let cf_schema = db.cf(CF_SCHEMA)?;

        // Empty DB sets schema to DB_SCHEMA_VERSION
        db_schema.check_db_version()?;
        let version_slice = db.get(cf_schema, FIELD_VERSION)?;
        assert_eq!(
            version_slice.as_deref(),
            Some(DbVersionNumZC::new(DB_SCHEMA_VERSION).as_bytes()),
        );

        // Manually downgrade to version 99, will err with "db too old".
        db.rocks()
            .put_cf(cf_schema, FIELD_VERSION, 99_u64.to_be_bytes())?;
        let err = db_schema
            .check_db_version()
            .unwrap_err()
            .downcast::<DbSchemaError>()?;
        assert_eq!(
            err,
            DbSchemaError::DbTooOld {
                expected: DB_SCHEMA_VERSION,
                actual: 99,
            },
        );

        // Manually upgrade to version 1234567890, will err with "db too new".
        db.rocks()
            .put_cf(cf_schema, FIELD_VERSION, 1234567890_u64.to_be_bytes())?;
        let err = db_schema
            .check_db_version()
            .unwrap_err()
            .downcast::<DbSchemaError>()?;
        assert_eq!(
            err,
            DbSchemaError::DbTooNew {
                expected: DB_SCHEMA_VERSION,
                actual: 1234567890,
            },
        );

        Ok(())
    }
}

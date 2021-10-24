use std::{borrow::Cow, fmt::Debug};

use bitcoinsuite_error::{ErrorMeta, Result};
use rocksdb::{ColumnFamilyDescriptor, Options, WriteBatch};
use thiserror::Error;
use zerocopy::{AsBytes, FromBytes, Unaligned};

use crate::{
    data::{interpret, interpret_slice},
    merge_ops::merge_op_ordered_list,
    Db,
};

const FLAG_INSERT: u8 = b'I';
const FLAG_DELETE: u8 = b'D';

pub struct Index<I: Indexable> {
    lookup_cf_name: &'static str,
    index_cf_name: &'static str,
    indexable: I,
}

pub trait Indexable: 'static {
    type Hash: FromBytes + AsBytes + Unaligned + Debug;
    type Serial: FromBytes + AsBytes + Unaligned + Clone + Ord + Debug;
    type Key: FromBytes + Unaligned + Clone + Eq + Debug;
    type Value: FromBytes + Unaligned + Clone;
    fn hash(&self, key: &Self::Key) -> Self::Hash;
    fn get_value_key<'a>(&self, value: &'a Self::Value) -> Cow<'a, Self::Key>;
}

#[derive(Debug, Error, ErrorMeta)]
pub enum IndexError {
    #[critical()]
    #[error("Inconsistent database: {0}")]
    InconsistentDatabase(Cow<'static, str>),
}

impl<I: Indexable> Index<I> {
    pub fn add_cfs(columns: &mut Vec<ColumnFamilyDescriptor>, index_cf_name: &'static str) {
        let mut options = Options::default();
        options.set_merge_operator(
            "slp-indexer-rocks.MergeIndex",
            merge_op_ordered_list::<I::Serial>,
            merge_op_ordered_list::<I::Serial>,
        );
        columns.push(ColumnFamilyDescriptor::new(index_cf_name, options));
    }

    pub fn new(lookup_cf_name: &'static str, index_cf_name: &'static str, indexable: I) -> Self {
        Index {
            lookup_cf_name,
            index_cf_name,
            indexable,
        }
    }

    pub fn get(&self, db: &Db, key: &I::Key) -> Result<Option<(I::Serial, I::Value)>> {
        let index_cf = db.cf(self.index_cf_name)?;
        let lookup_cf = db.cf(self.lookup_cf_name)?;
        let hash = self.indexable.hash(key);
        let hash_items = db.get(index_cf, hash.as_bytes())?;
        let hash_items = match hash_items {
            Some(hash_items) => hash_items,
            None => return Ok(None),
        };
        let serials: &[I::Serial] = interpret_slice(hash_items.as_ref())?;
        for serial in serials {
            let value = match db.get(lookup_cf, serial.as_bytes())? {
                Some(value) => value,
                None => return Err(self._inconsistent_error().into()),
            };
            let value = interpret(value.as_ref())?;
            if self.indexable.get_value_key(value).as_ref() == key {
                return Ok(Some((serial.clone(), value.clone())));
            }
        }
        Ok(None)
    }

    pub fn insert(
        &self,
        db: &Db,
        batch: &mut WriteBatch,
        serial: &I::Serial,
        value: &I::Value,
    ) -> Result<()> {
        let key = self.indexable.get_value_key(value);
        self.merge_value(db, batch, serial, &key, FLAG_INSERT)
    }

    pub fn delete(
        &self,
        db: &Db,
        batch: &mut WriteBatch,
        serial: &I::Serial,
        key: &I::Key,
    ) -> Result<()> {
        self.merge_value(db, batch, serial, key, FLAG_DELETE)
    }

    fn merge_value(
        &self,
        db: &Db,
        batch: &mut WriteBatch,
        serial: &I::Serial,
        key: &I::Key,
        flag: u8,
    ) -> Result<()> {
        let index_cf = db.cf(self.index_cf_name)?;
        let hash = self.indexable.hash(key);
        let mut serial_bytes = serial.as_bytes().to_vec();
        serial_bytes.insert(0, flag);
        batch.merge_cf(&index_cf, hash.as_bytes(), &serial_bytes);
        Ok(())
    }

    fn _inconsistent_error(&self) -> IndexError {
        IndexError::InconsistentDatabase(
            format!(
                "Lookup in {} for item indexed in {} doesn't exist",
                self.lookup_cf_name, self.index_cf_name
            )
            .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use bitcoinsuite_error::Result;
    use byteorder::LE;
    use pretty_assertions::assert_eq;
    use rocksdb::{ColumnFamilyDescriptor, Options, WriteBatch};
    use zerocopy::{AsBytes, FromBytes, Unaligned, I32, U16, U32};

    use crate::Db;

    use super::{Index, Indexable};

    #[derive(Debug, Clone, Copy, FromBytes, AsBytes, Unaligned, PartialEq, Eq)]
    #[repr(C)]
    struct TestData {
        pub key: U16<LE>,
        pub payload: U32<LE>,
    }

    #[derive(Debug, Copy, Clone, FromBytes, AsBytes, Unaligned, PartialEq, Eq)]
    #[repr(C)]
    pub struct TestSerial(I32<LE>);

    impl Ord for TestSerial {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.0.get().cmp(&other.0.get())
        }
    }

    impl PartialOrd for TestSerial {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    struct ModIndexable;

    impl Indexable for ModIndexable {
        type Hash = u8;
        type Serial = TestSerial;
        type Key = U16<LE>;
        type Value = TestData;

        fn hash(&self, key: &Self::Key) -> Self::Hash {
            (key.get() % 10) as u8
        }

        fn get_value_key<'a>(&self, value: &'a Self::Value) -> Cow<'a, Self::Key> {
            Cow::Borrowed(&value.key)
        }
    }

    pub const CF_TEST: &str = "test";
    pub const CF_TEST_INDEX: &str = "test_index";

    #[test]
    fn test_index() -> Result<()> {
        bitcoinsuite_error::install()?;
        let tempdir = tempdir::TempDir::new("slp-indexer-rocks--blocks")?;
        let mut cfs = vec![ColumnFamilyDescriptor::new(CF_TEST, Options::default())];
        Index::<ModIndexable>::add_cfs(&mut cfs, CF_TEST_INDEX);
        let db = Db::open_with_cfs(tempdir.path(), cfs)?;
        let index = Index::new(CF_TEST, CF_TEST_INDEX, ModIndexable);
        // First insert
        let serial0 = TestSerial(I32::new(1));
        let key0 = U16::new(10_007);
        let data0 = TestData {
            key: key0,
            payload: U32::new(0xdeadbeef),
        };
        {
            let mut batch = WriteBatch::default();
            batch.put_cf(db.cf(CF_TEST)?, serial0.as_bytes(), data0.as_bytes());
            index.insert(&db, &mut batch, &serial0, &data0)?;
            db.write_batch(batch)?;
            assert_eq!(index.get(&db, &key0)?, Some((serial0, data0)));
            assert_eq!(
                db.get(db.cf(CF_TEST_INDEX)?, &[7])?.as_deref(),
                Some(serial0.as_bytes())
            );
        }
        // Second, non-colliding insert
        let serial1 = TestSerial(I32::new(2));
        let key1 = U16::new(10_008);
        let data1 = TestData {
            key: key1,
            payload: U32::new(0xcafe),
        };
        {
            let mut batch = WriteBatch::default();
            batch.put_cf(db.cf(CF_TEST)?, serial1.as_bytes(), data1.as_bytes());
            index.insert(&db, &mut batch, &serial1, &data1)?;
            db.write_batch(batch)?;
            assert_eq!(index.get(&db, &key0)?, Some((serial0, data0)));
            assert_eq!(index.get(&db, &key1)?, Some((serial1, data1)));
            assert_eq!(
                db.get(db.cf(CF_TEST_INDEX)?, &[7])?.as_deref(),
                Some(serial0.as_bytes())
            );
            assert_eq!(
                db.get(db.cf(CF_TEST_INDEX)?, &[8])?.as_deref(),
                Some(serial1.as_bytes())
            );
        }
        // 1st colliding insert
        let serial2 = TestSerial(I32::new(4));
        let key2 = U16::new(10_047);
        let data2 = TestData {
            key: key2,
            payload: U32::new(0xabcd),
        };
        {
            let mut batch = WriteBatch::default();
            batch.put_cf(db.cf(CF_TEST)?, serial2.as_bytes(), data2.as_bytes());
            index.insert(&db, &mut batch, &serial2, &data2)?;
            db.write_batch(batch)?;
            assert_eq!(index.get(&db, &key0)?, Some((serial0, data0)));
            assert_eq!(index.get(&db, &key1)?, Some((serial1, data1)));
            assert_eq!(index.get(&db, &key2)?, Some((serial2, data2)));
            // inserted in 7 to the right
            let index_value = [serial0.as_bytes(), serial2.as_bytes()].concat();
            assert_eq!(
                db.get(db.cf(CF_TEST_INDEX)?, &[7])?.as_deref(),
                Some(index_value.as_slice())
            );
            assert_eq!(
                db.get(db.cf(CF_TEST_INDEX)?, &[8])?.as_deref(),
                Some(serial1.as_bytes())
            );
        }
        // 2nd colliding insert
        let serial3 = TestSerial(I32::new(3));
        let key3 = U16::new(10_037);
        let data3 = TestData {
            key: key3,
            payload: U32::new(0xfedc),
        };
        {
            let mut batch = WriteBatch::default();
            batch.put_cf(db.cf(CF_TEST)?, serial3.as_bytes(), data3.as_bytes());
            index.insert(&db, &mut batch, &serial3, &data3)?;
            db.write_batch(batch)?;
            assert_eq!(index.get(&db, &key0)?, Some((serial0, data0)));
            assert_eq!(index.get(&db, &key1)?, Some((serial1, data1)));
            assert_eq!(index.get(&db, &key2)?, Some((serial2, data2)));
            assert_eq!(index.get(&db, &key3)?, Some((serial3, data3)));
            // inserted in 7 in the middle
            let index_value = [serial0.as_bytes(), serial3.as_bytes(), serial2.as_bytes()].concat();
            assert_eq!(
                db.get(db.cf(CF_TEST_INDEX)?, &[7])?.as_deref(),
                Some(index_value.as_slice())
            );
            assert_eq!(
                db.get(db.cf(CF_TEST_INDEX)?, &[8])?.as_deref(),
                Some(serial1.as_bytes())
            );
        }
        // Delete key0
        {
            let mut batch = WriteBatch::default();
            index.delete(&db, &mut batch, &serial0, &key0)?;
            db.write_batch(batch)?;
            assert_eq!(index.get(&db, &key0)?, None);
            let index_value = [serial3.as_bytes(), serial2.as_bytes()].concat();
            assert_eq!(
                db.get(db.cf(CF_TEST_INDEX)?, &[7])?.as_deref(),
                Some(index_value.as_slice())
            );
            assert_eq!(
                db.get(db.cf(CF_TEST_INDEX)?, &[8])?.as_deref(),
                Some(serial1.as_bytes())
            );
        }
        // Delete key1
        {
            let mut batch = WriteBatch::default();
            index.delete(&db, &mut batch, &serial1, &key1)?;
            db.write_batch(batch)?;
            assert_eq!(index.get(&db, &key1)?, None);
            let index_value = [serial3.as_bytes(), serial2.as_bytes()].concat();
            assert_eq!(
                db.get(db.cf(CF_TEST_INDEX)?, &[7])?.as_deref(),
                Some(index_value.as_slice())
            );
            assert_eq!(
                db.get(db.cf(CF_TEST_INDEX)?, &[8])?.as_deref(),
                Some([].as_ref())
            );
        }
        // Delete key2
        {
            let mut batch = WriteBatch::default();
            index.delete(&db, &mut batch, &serial2, &key2)?;
            db.write_batch(batch)?;
            assert_eq!(index.get(&db, &key2)?, None);
            assert_eq!(
                db.get(db.cf(CF_TEST_INDEX)?, &[7])?.as_deref(),
                Some(serial3.as_bytes())
            );
            assert_eq!(
                db.get(db.cf(CF_TEST_INDEX)?, &[8])?.as_deref(),
                Some([].as_ref())
            );
        }
        Ok(())
    }
}

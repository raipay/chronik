use std::cmp::Ordering;

use bitcoinsuite_core::UnhashedTx;
use bitcoinsuite_error::Result;
use byteorder::BE;
use rocksdb::{ColumnFamilyDescriptor, Options, WriteBatch};
use zerocopy::{AsBytes, FromBytes, Unaligned, U32};

use crate::{
    data::interpret_slice,
    merge_ops::{
        full_merge_ordered_list, partial_merge_ordered_list, PREFIX_DELETE, PREFIX_INSERT,
    },
    Db, TxNum, TxNumZC, CF,
};

pub const CF_SPENDS: &str = "spends";

/*
spends:
tx_num -> [(out_idx, tx_num, input_idx)]
*/

#[derive(Debug, Clone, FromBytes, AsBytes, Unaligned, PartialEq, Eq)]
#[repr(C)]
struct SpendData {
    out_idx: U32<BE>,
    tx_num: TxNumZC,
    input_idx: U32<BE>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpendEntry {
    pub out_idx: u32,
    pub tx_num: TxNum,
    pub input_idx: u32,
}

pub struct SpendsWriter<'a> {
    cf_spends: &'a CF,
}

pub struct SpendsReader<'a> {
    db: &'a Db,
    cf_spends: &'a CF,
}

impl<'a> SpendsWriter<'a> {
    pub fn add_cfs(columns: &mut Vec<ColumnFamilyDescriptor>) {
        let mut options = Options::default();
        options.set_merge_operator(
            "slp-indexer-rocks.MergeSpends",
            full_merge_ordered_list::<SpendData>,
            partial_merge_ordered_list::<SpendData>,
        );
        columns.push(ColumnFamilyDescriptor::new(CF_SPENDS, options));
    }

    pub fn new(db: &'a Db) -> Result<Self> {
        let cf_spends = db.cf(CF_SPENDS)?;
        Ok(SpendsWriter { cf_spends })
    }

    pub fn insert_block_txs(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: TxNum,
        txs: &[UnhashedTx],
        input_tx_nums: &[Vec<u64>],
    ) -> Result<()> {
        self.update_block_txs(batch, first_tx_num, txs, input_tx_nums, PREFIX_INSERT)
    }

    pub fn delete_block_txs(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: TxNum,
        txs: &[UnhashedTx],
        input_tx_nums: &[Vec<u64>],
    ) -> Result<()> {
        self.update_block_txs(batch, first_tx_num, txs, input_tx_nums, PREFIX_DELETE)
    }

    fn update_block_txs(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: TxNum,
        txs: &[UnhashedTx],
        input_tx_nums: &[Vec<u64>],
        prefix: u8,
    ) -> Result<()> {
        for ((tx_idx, tx), tx_input_nums) in txs.iter().enumerate().skip(1).zip(input_tx_nums) {
            let tx_num = first_tx_num + tx_idx as TxNum;
            for (input_idx, (input, input_tx_num)) in tx
                .inputs
                .iter()
                .zip(tx_input_nums.iter().cloned())
                .enumerate()
            {
                let spend = SpendData {
                    out_idx: input.prev_out.out_idx.into(),
                    tx_num: tx_num.into(),
                    input_idx: (input_idx as u32).into(),
                };
                let mut value = spend.as_bytes().to_vec();
                value.insert(0, prefix);
                batch.merge_cf(self.cf_spends, TxNumZC::new(input_tx_num).as_bytes(), value);
            }
        }
        Ok(())
    }
}

impl<'a> SpendsReader<'a> {
    pub fn new(db: &'a Db) -> Result<Self> {
        let cf_spends = db.cf(CF_SPENDS)?;
        Ok(SpendsReader { db, cf_spends })
    }

    pub fn spends_by_tx_num(&self, tx_num: TxNum) -> Result<Vec<SpendEntry>> {
        let tx_num = TxNumZC::new(tx_num);
        let value = match self.db.get(self.cf_spends, tx_num.as_bytes())? {
            Some(value) => value,
            None => return Ok(vec![]),
        };
        let entries = interpret_slice::<SpendData>(&value)?
            .iter()
            .map(|entry| SpendEntry {
                out_idx: entry.out_idx.get(),
                tx_num: entry.tx_num.get(),
                input_idx: entry.input_idx.get(),
            })
            .collect();
        Ok(entries)
    }
}

impl Ord for SpendData {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.out_idx.get().cmp(&other.out_idx.get()) {
            Ordering::Equal => match self.tx_num.get().cmp(&other.tx_num.get()) {
                Ordering::Equal => self.input_idx.get().cmp(&other.input_idx.get()),
                ordering => ordering,
            },
            ordering => ordering,
        }
    }
}

impl PartialOrd for SpendData {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod test {
    use crate::{
        input_tx_nums::fetch_input_tx_nums, spends::SpendData, BlockHeight, BlockTxs, Db,
        SpendEntry, SpendsReader, SpendsWriter, TxEntry, TxNum, TxNumZC, TxWriter,
    };
    use bitcoinsuite_core::{OutPoint, Sha256d, TxInput, UnhashedTx};
    use bitcoinsuite_error::Result;
    use pretty_assertions::assert_eq;
    use rocksdb::WriteBatch;
    use zerocopy::AsBytes;

    #[test]
    fn test_spends() -> Result<()> {
        bitcoinsuite_error::install()?;
        let tempdir = tempdir::TempDir::new("slp-indexer-rocks--spends")?;
        let db = Db::open(tempdir.path())?;
        let tx_writer = TxWriter::new(&db)?;
        let spends_writer = SpendsWriter::new(&db)?;
        let spends_reader = SpendsReader::new(&db)?;
        let txs_block1: &[&[_]] = &[&[]];
        let txs_block2: &[&[_]] = &[&[], &[(0, 0)], &[(2, 1)], &[(3, 0)]];
        let txs_block3: &[&[_]] = &[&[], &[(3, 1), (0, 1)]];
        let txs_blocks = &[txs_block1, txs_block2, txs_block3];
        let mut blocks = Vec::new();
        let mut num_txs: TxNum = 0;
        for &txs_block in txs_blocks {
            let mut block_txids = Vec::new();
            let mut block_txs = Vec::new();
            let mut txs = Vec::new();
            let first_tx_num = num_txs;
            for inputs in txs_block {
                let txid = Sha256d::new([num_txs as u8; 32]);
                block_txids.push(txid.clone());
                block_txs.push(TxEntry {
                    txid,
                    data_pos: 0,
                    tx_size: 0,
                    undo_pos: 0,
                    undo_size: 0,
                    time_first_seen: 0,
                    is_coinbase: false,
                });
                num_txs += 1;
                txs.push(UnhashedTx {
                    version: 1,
                    inputs: inputs
                        .iter()
                        .map(|&(tx_num, out_idx)| TxInput {
                            prev_out: OutPoint {
                                txid: Sha256d::new([tx_num as u8; 32]),
                                out_idx,
                            },
                            ..Default::default()
                        })
                        .collect(),
                    outputs: vec![],
                    lock_time: 0,
                });
            }
            blocks.push((first_tx_num, block_txids, txs, block_txs));
        }
        let connect_block = |block_height: usize| -> Result<()> {
            let mut batch = WriteBatch::default();
            let input_tx_nums = fetch_input_tx_nums(
                &db,
                blocks[block_height].0,
                |idx| &blocks[block_height].1[idx],
                &blocks[block_height].2,
            )?;
            spends_writer.insert_block_txs(
                &mut batch,
                blocks[block_height].0,
                &blocks[block_height].2,
                &input_tx_nums,
            )?;
            tx_writer.insert_block_txs(
                &mut batch,
                &BlockTxs {
                    txs: blocks[block_height].3.clone(),
                    block_height: block_height as BlockHeight,
                },
            )?;
            db.write_batch(batch)?;
            Ok(())
        };
        let disconnect_block = |block_height: usize| -> Result<()> {
            let mut batch = WriteBatch::default();
            let input_tx_nums = fetch_input_tx_nums(
                &db,
                blocks[block_height].0,
                |idx| &blocks[block_height].1[idx],
                &blocks[block_height].2,
            )?;
            spends_writer.delete_block_txs(
                &mut batch,
                blocks[block_height].0,
                &blocks[block_height].2,
                &input_tx_nums,
            )?;
            tx_writer.delete_block_txs(&mut batch, block_height as BlockHeight)?;
            db.write_batch(batch)?;
            Ok(())
        };
        {
            check_spends(&spends_reader, 0, [])?;
            check_spends(&spends_reader, 1, [])?;
        }
        {
            // Connect block 0
            connect_block(0)?;
            check_spends(&spends_reader, 0, [])?;
            check_spends(&spends_reader, 1, [])?;
        }
        {
            // Connect block 1
            connect_block(1)?;
            check_spends(&spends_reader, 0, [(0, 2, 0)])?;
            check_spends(&spends_reader, 1, [])?;
            check_spends(&spends_reader, 2, [(1, 3, 0)])?;
            check_spends(&spends_reader, 3, [(0, 4, 0)])?;
            check_spends(&spends_reader, 4, [])?;
        }
        {
            // Disconnect block 1
            disconnect_block(1)?;
            check_spends(&spends_reader, 0, [])?;
            check_spends(&spends_reader, 1, [])?;
        }
        {
            // Disconnect block 0
            disconnect_block(0)?;
            check_spends(&spends_reader, 0, [])?;
            check_spends(&spends_reader, 1, [])?;
        }
        {
            // Connect block 0, 1, 2
            connect_block(0)?;
            connect_block(1)?;
            connect_block(2)?;
            check_spends(&spends_reader, 0, [(0, 2, 0), (1, 6, 1)])?;
            check_spends(&spends_reader, 1, [])?;
            check_spends(&spends_reader, 2, [(1, 3, 0)])?;
            check_spends(&spends_reader, 3, [(0, 4, 0), (1, 6, 0)])?;
            check_spends(&spends_reader, 4, [])?;
            check_spends(&spends_reader, 5, [])?;
            check_spends(&spends_reader, 6, [])?;
        }
        {
            // Disconnect block 2
            disconnect_block(2)?;
            check_spends(&spends_reader, 0, [(0, 2, 0)])?;
            check_spends(&spends_reader, 1, [])?;
            check_spends(&spends_reader, 2, [(1, 3, 0)])?;
            check_spends(&spends_reader, 3, [(0, 4, 0)])?;
            check_spends(&spends_reader, 4, [])?;
            check_spends(&spends_reader, 5, [])?;
            check_spends(&spends_reader, 6, [])?;
        }
        Ok(())
    }

    fn check_spends<const N: usize>(
        spends_reader: &SpendsReader,
        tx_num: TxNum,
        expected_txs: [(u32, TxNum, u32); N],
    ) -> Result<()> {
        assert_eq!(
            spends_reader.spends_by_tx_num(tx_num)?,
            expected_txs
                .into_iter()
                .map(|(out_idx, tx_num, input_idx)| SpendEntry {
                    out_idx,
                    tx_num,
                    input_idx
                })
                .collect::<Vec<_>>(),
        );
        let tx_num = TxNumZC::new(tx_num);
        let value = match spends_reader
            .db
            .get(spends_reader.cf_spends, tx_num.as_bytes())?
        {
            Some(value) => value,
            None => {
                assert_eq!(N, 0);
                return Ok(());
            }
        };
        let entry_data = expected_txs
            .into_iter()
            .map(|(out_idx, tx_num, input_idx)| SpendData {
                out_idx: out_idx.into(),
                tx_num: tx_num.into(),
                input_idx: input_idx.into(),
            })
            .collect::<Vec<_>>();
        assert_eq!(value.as_ref(), entry_data.as_bytes());
        Ok(())
    }
}

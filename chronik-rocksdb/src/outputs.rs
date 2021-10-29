use std::collections::HashMap;

use bitcoinsuite_core::UnhashedTx;
use bitcoinsuite_error::Result;
use rocksdb::{ColumnFamilyDescriptor, Direction, IteratorMode, Options, WriteBatch};
use zerocopy::{AsBytes, U32};

use crate::{
    data::interpret_slice,
    merge_ops::{
        full_merge_ordered_list, partial_merge_ordered_list, PREFIX_DELETE, PREFIX_INSERT,
    },
    outpoint_data::{OutpointData, OutpointEntry},
    script_payload::{script_payloads, PayloadPrefix},
    Db, Timings, TxNum, CF,
};

pub const CF_OUTPUTS: &str = "outputs";

type ScriptPageNum = u32;
const PAGE_NUM_SIZE: usize = std::mem::size_of::<ScriptPageNum>();

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OutputsConf {
    pub page_size: usize,
}

pub struct OutputsWriter<'a> {
    db: &'a Db,
    cf_outputs: &'a CF,
    conf: OutputsConf,
}

pub struct OutputsReader<'a> {
    db: &'a Db,
    cf_outputs: &'a CF,
}

impl<'a> OutputsWriter<'a> {
    pub fn add_cfs(columns: &mut Vec<ColumnFamilyDescriptor>) {
        let mut options = Options::default();
        options.set_merge_operator(
            "slp-indexer-rocks.MergeOutputs",
            full_merge_ordered_list::<OutpointData>,
            partial_merge_ordered_list::<OutpointData>,
        );
        columns.push(ColumnFamilyDescriptor::new(CF_OUTPUTS, options));
    }

    pub fn new(db: &'a Db, conf: OutputsConf) -> Result<Self> {
        let cf_outputs = db.cf(CF_OUTPUTS)?;
        Ok(OutputsWriter {
            db,
            cf_outputs,
            conf,
        })
    }

    pub fn insert_block_txs(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: u64,
        txs: &[UnhashedTx],
    ) -> Result<Timings> {
        let mut tx_num = first_tx_num;
        let mut num_outputs_by_payload = HashMap::new();
        let mut timings = Timings::default();
        for tx in txs {
            for (out_idx, output) in tx.outputs.iter().enumerate() {
                for (payload_prefix, mut script_payload) in script_payloads(&output.script) {
                    timings.start_timer();
                    script_payload.insert(0, payload_prefix as u8);
                    let num_txs_db = self.get_num_outputs_for_payload(&script_payload)?;
                    timings.stop_timer("get_num_outputs");

                    timings.start_timer();
                    let num_txs_map = num_outputs_by_payload
                        .entry(script_payload.clone())
                        .or_insert(0);
                    let num_txs = num_txs_db + *num_txs_map;
                    let page_num = num_txs / self.conf.page_size as u32;
                    let key = key_for_script_payload(&script_payload, page_num);
                    let script_entry = OutpointData {
                        tx_num: TxNum(tx_num.into()),
                        out_idx: U32::new(out_idx as u32),
                    };
                    let mut value = script_entry.as_bytes().to_vec();
                    value.insert(0, PREFIX_INSERT);
                    timings.stop_timer("prepare_value");

                    timings.start_timer();
                    batch.merge_cf(self.cf_outputs, key, value);
                    timings.stop_timer("merge_into_batch");

                    *num_txs_map += 1;
                }
            }
            tx_num += 1;
        }
        Ok(timings)
    }

    pub fn delete_block_txs(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: u64,
        txs: &[UnhashedTx],
    ) -> Result<()> {
        let mut num_outputs_by_payload = HashMap::new();
        for (tx_idx, tx) in txs.iter().enumerate().rev() {
            let tx_num = first_tx_num + tx_idx as u64;
            for (out_idx, output) in tx.outputs.iter().enumerate().rev() {
                for (payload_prefix, mut script_payload) in script_payloads(&output.script) {
                    script_payload.insert(0, payload_prefix as u8);
                    let num_txs_db = self.get_num_outputs_for_payload(&script_payload)?;
                    let num_txs_map = num_outputs_by_payload
                        .entry(script_payload.clone())
                        .or_insert(0);
                    let num_txs = num_txs_db - *num_txs_map - 1;
                    let page_num = num_txs / self.conf.page_size as u32;
                    let key = key_for_script_payload(&script_payload, page_num);
                    let script_entry = OutpointData {
                        tx_num: TxNum(tx_num.into()),
                        out_idx: U32::new(out_idx as u32),
                    };
                    let mut value = script_entry.as_bytes().to_vec();
                    value.insert(0, PREFIX_DELETE);
                    batch.merge_cf(self.cf_outputs, key, value);
                    *num_txs_map += 1;
                }
            }
        }
        Ok(())
    }

    fn get_num_outputs_for_payload(&self, payload: &[u8]) -> Result<u32> {
        let last_key = key_for_script_payload(payload, std::u32::MAX);
        let mut iterator = self.db.rocks().iterator_cf(
            self.cf_outputs,
            IteratorMode::From(&last_key, Direction::Reverse),
        );
        let (key, value) = loop {
            match iterator.next() {
                Some((key, value)) => {
                    if !value.is_empty() {
                        break (key, value);
                    }
                }
                None => return Ok(0),
            };
        };
        let entries = interpret_slice::<OutpointData>(&value)?;
        let page_num =
            ScriptPageNum::from_be_bytes(key[key.len() - PAGE_NUM_SIZE..].try_into().unwrap());
        Ok((page_num as usize * self.conf.page_size + entries.len()) as u32)
    }
}

fn key_for_script_payload(script_payload: &[u8], page_num: u32) -> Vec<u8> {
    [script_payload, page_num.to_be_bytes().as_ref()].concat()
}

impl<'a> OutputsReader<'a> {
    pub fn new(db: &'a Db) -> Result<Self> {
        let cf_outputs = db.cf(CF_OUTPUTS)?;
        Ok(OutputsReader { db, cf_outputs })
    }

    pub fn num_pages_by_payload(
        &self,
        prefix: PayloadPrefix,
        payload_data: &[u8],
    ) -> Result<usize> {
        let script_payload = [[prefix as u8].as_ref(), payload_data].concat();
        let iterator = self.db.rocks().iterator_cf(
            self.cf_outputs,
            IteratorMode::From(&script_payload, Direction::Forward),
        );
        let num_pages = iterator
            .take_while(|(key, _)| {
                key.get(..script_payload.len()) == Some(script_payload.as_slice())
            })
            .filter(|(_, value)| !value.is_empty())
            .count();
        Ok(num_pages)
    }

    pub fn page_txs(
        &self,
        page_num: ScriptPageNum,
        prefix: PayloadPrefix,
        payload_data: &[u8],
    ) -> Result<Vec<OutpointEntry>> {
        let script_payload = [[prefix as u8].as_ref(), payload_data].concat();
        let key = key_for_script_payload(&script_payload, page_num);
        let value = match self.db.get(self.cf_outputs, &key)? {
            Some(value) => value,
            None => return Ok(vec![]),
        };
        let entries = interpret_slice::<OutpointData>(&value)?
            .iter()
            .map(|entry| OutpointEntry {
                tx_num: entry.tx_num.0.get(),
                out_idx: entry.out_idx.get(),
            })
            .collect();
        Ok(entries)
    }
}

#[cfg(test)]
mod test {
    use crate::{
        outpoint_data::OutpointData, outputs::key_for_script_payload, Db, OutpointEntry,
        OutputsConf, OutputsReader, OutputsWriter, PayloadPrefix, TxNum,
    };
    use bitcoinsuite_core::{ecc::PubKey, Script, ShaRmd160, TxOutput, UnhashedTx};
    use bitcoinsuite_error::Result;
    use pretty_assertions::assert_eq;
    use rocksdb::WriteBatch;
    use zerocopy::AsBytes;

    #[test]
    fn test_scripts() -> Result<()> {
        use PayloadPrefix::*;
        bitcoinsuite_error::install()?;
        let tempdir = tempdir::TempDir::new("slp-indexer-rocks--scripts")?;
        let db = Db::open(tempdir.path())?;
        let conf = OutputsConf { page_size: 5 };
        let outputs_writer = OutputsWriter::new(&db, conf)?;
        let outputs_reader = OutputsReader::new(&db)?;
        let r = &outputs_reader;
        let (script1, payload1) = (Script::p2pkh(&ShaRmd160::new([1; 20])), [1; 20]);
        let (script2, payload2) = (Script::p2pkh(&ShaRmd160::new([2; 20])), [2; 20]);
        let (script3, payload3) = (Script::p2sh(&ShaRmd160::new([3; 20])), [3; 20]);
        let (script4, payload4) = (Script::p2sh(&ShaRmd160::new([4; 20])), [4; 20]);
        let (script5, payload5) = (Script::p2pk(&PubKey::new_unchecked([5; 33])), [5; 33]);
        let (script6, payload6) = (Script::p2tr(&PubKey::new_unchecked([6; 33]), None), [6; 33]);
        let (script7, payload7, payload8) = (
            Script::p2tr(&PubKey::new_unchecked([7; 33]), Some([8; 32])),
            [7; 33],
            [8; 32],
        );
        let tx_scripts_block1 = vec![
            vec![&script1, &script2],
            vec![&script1, &script2, &script1, &script1],
        ];
        let tx_scripts_block2 = vec![
            vec![&script4, &script1],
            vec![&script5, &script1],
            vec![&script1, &script3, &script1, &script1],
        ];
        let tx_scripts_block3 = vec![vec![&script6, &script1], vec![&script7, &script1]];
        let mut blocks = Vec::new();
        let mut num_txs = 0;
        for tx_scripts in [&tx_scripts_block1, &tx_scripts_block2, &tx_scripts_block3] {
            let txs = tx_scripts
                .iter()
                .map(|scripts| UnhashedTx {
                    version: 1,
                    inputs: vec![],
                    outputs: scripts
                        .iter()
                        .map(|&script| TxOutput {
                            value: 0,
                            script: script.clone(),
                        })
                        .collect(),
                    lock_time: 0,
                })
                .collect::<Vec<_>>();
            blocks.push((num_txs as u64, txs));
            num_txs += tx_scripts.len();
        }
        {
            check_pages(r, P2PKH, &payload1, [])?;
        }
        {
            let mut batch = WriteBatch::default();
            outputs_writer.insert_block_txs(&mut batch, blocks[0].0, &blocks[0].1)?;
            db.write_batch(batch)?;
            check_pages(r, P2PKH, &payload1, [&[(0, 0), (1, 0), (1, 2), (1, 3)]])?;
            check_pages(r, P2PKH, &payload2, [&[(0, 1), (1, 1)]])?;
            check_pages(r, P2PK, &payload2, [])?;
        }
        {
            let mut batch = WriteBatch::default();
            outputs_writer.delete_block_txs(&mut batch, blocks[0].0, &blocks[0].1)?;
            db.write_batch(batch)?;
            check_pages(r, P2PKH, &payload1, [])?;
            check_pages(r, P2PKH, &payload2, [])?;
        }
        {
            let mut batch = WriteBatch::default();
            outputs_writer.insert_block_txs(&mut batch, blocks[0].0, &blocks[0].1)?;
            db.write_batch(batch)?;
            let mut batch = WriteBatch::default();
            outputs_writer.insert_block_txs(&mut batch, blocks[1].0, &blocks[1].1)?;
            db.write_batch(batch)?;
            check_pages(
                r,
                P2PKH,
                &payload1,
                [
                    &[(0, 0), (1, 0), (1, 2), (1, 3), (2, 1)],
                    &[(3, 1), (4, 0), (4, 2), (4, 3)],
                ],
            )?;
            check_pages(r, P2PKH, &payload2, [&[(0, 1), (1, 1)]])?;
            check_pages(r, P2SH, &payload3, [&[(4, 1)]])?;
            check_pages(r, P2SH, &payload4, [&[(2, 0)]])?;
            check_pages(r, P2PK, &payload5, [&[(3, 0)]])?;
        }
        {
            let mut batch = WriteBatch::default();
            outputs_writer.insert_block_txs(&mut batch, blocks[2].0, &blocks[2].1)?;
            db.write_batch(batch)?;
            check_pages(
                r,
                P2PKH,
                &payload1,
                [
                    &[(0, 0), (1, 0), (1, 2), (1, 3), (2, 1)],
                    &[(3, 1), (4, 0), (4, 2), (4, 3), (5, 1)],
                    &[(6, 1)],
                ],
            )?;
            check_pages(r, P2TRCommitment, &payload6, [&[(5, 0)]])?;
            check_pages(r, P2TRCommitment, &payload7, [&[(6, 0)]])?;
            check_pages(r, P2TRState, &payload8, [&[(6, 0)]])?;
        }
        {
            let mut batch = WriteBatch::default();
            outputs_writer.delete_block_txs(&mut batch, blocks[2].0, &blocks[2].1)?;
            db.write_batch(batch)?;
            check_pages(
                r,
                P2PKH,
                &payload1,
                [
                    &[(0, 0), (1, 0), (1, 2), (1, 3), (2, 1)],
                    &[(3, 1), (4, 0), (4, 2), (4, 3)],
                ],
            )?;
            check_pages(r, P2PK, &payload5, [&[(3, 0)]])?;
            check_pages(r, P2TRCommitment, &payload6, [])?;
            check_pages(r, P2TRCommitment, &payload7, [])?;
            check_pages(r, P2TRState, &payload8, [])?;
        }
        {
            let mut batch = WriteBatch::default();
            outputs_writer.delete_block_txs(&mut batch, blocks[1].0, &blocks[1].1)?;
            db.write_batch(batch)?;
            check_pages(r, P2PKH, &payload1, [&[(0, 0), (1, 0), (1, 2), (1, 3)]])?;
            check_pages(r, P2PKH, &payload2, [&[(0, 1), (1, 1)]])?;
            check_pages(r, P2SH, &payload3, [])?;
            check_pages(r, P2SH, &payload4, [])?;
            check_pages(r, P2PK, &payload5, [])?;
        }
        {
            let mut batch = WriteBatch::default();
            outputs_writer.delete_block_txs(&mut batch, blocks[0].0, &blocks[0].1)?;
            db.write_batch(batch)?;
            check_pages(r, P2PKH, &payload1, [])?;
            check_pages(r, P2PKH, &payload2, [])?;
            check_pages(r, P2SH, &payload3, [])?;
            check_pages(r, P2SH, &payload4, [])?;
            check_pages(r, P2PK, &payload5, [])?;
        }
        Ok(())
    }

    fn check_pages<const N: usize>(
        outputs_reader: &OutputsReader,
        prefix: PayloadPrefix,
        payload_body: &[u8],
        expected_txs: [&[(u64, u32)]; N],
    ) -> Result<()> {
        assert_eq!(
            outputs_reader.num_pages_by_payload(prefix, payload_body)?,
            N,
        );
        for (page_num, txs) in expected_txs.into_iter().enumerate() {
            assert_eq!(
                outputs_reader.page_txs(page_num as u32, prefix, payload_body)?,
                txs.iter()
                    .cloned()
                    .map(|(tx_num, out_idx)| OutpointEntry { tx_num, out_idx })
                    .collect::<Vec<_>>(),
            );
            let script_payload = [[prefix as u8].as_ref(), payload_body].concat();
            let key = key_for_script_payload(&script_payload, page_num as u32);
            let value = outputs_reader
                .db
                .get(outputs_reader.cf_outputs, &key)?
                .unwrap();
            let entry_data = txs
                .iter()
                .cloned()
                .map(|(tx_num, out_idx)| OutpointData {
                    tx_num: TxNum(tx_num.into()),
                    out_idx: out_idx.into(),
                })
                .collect::<Vec<_>>();
            assert_eq!(value.as_ref(), entry_data.as_bytes());
        }
        Ok(())
    }
}

use std::collections::{BTreeSet, HashMap};

use bitcoinsuite_core::{Script, UnhashedTx};
use bitcoinsuite_error::Result;
use lru::LruCache;
use rocksdb::{ColumnFamilyDescriptor, Direction, IteratorMode, Options, WriteBatch};
use zerocopy::AsBytes;

use crate::{
    data::interpret_slice,
    merge_ops::{
        full_merge_ordered_list, partial_merge_ordered_list, PREFIX_DELETE, PREFIX_INSERT,
    },
    script_payload::{script_payloads, PayloadPrefix},
    Db, Timings, TxNum, TxNumOrd, TxNumZC, CF,
};

pub const CF_SCRIPT_TXS: &str = "script_txs";

type ScriptPageNum = u32;
const PAGE_NUM_SIZE: usize = std::mem::size_of::<ScriptPageNum>();

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScriptTxsConf {
    pub page_size: usize,
}

pub struct ScriptTxsWriter<'a> {
    db: &'a Db,
    cf_script_txs: &'a CF,
    conf: ScriptTxsConf,
}

pub struct ScriptTxsReader<'a> {
    db: &'a Db,
    cf_script_txs: &'a CF,
    conf: ScriptTxsConf,
}

pub struct ScriptTxsWriterCache {
    capacity: usize,
    num_txs_by_script: LruCache<Vec<u8>, u32>,
}

impl<'a> ScriptTxsWriter<'a> {
    pub fn add_cfs(columns: &mut Vec<ColumnFamilyDescriptor>) {
        let mut options = Options::default();
        options.set_merge_operator(
            "slp-indexer-rocks.MergeScriptTxs",
            full_merge_ordered_list::<TxNumOrd>,
            partial_merge_ordered_list::<TxNumOrd>,
        );
        columns.push(ColumnFamilyDescriptor::new(CF_SCRIPT_TXS, options));
    }

    pub fn new(db: &'a Db, conf: ScriptTxsConf) -> Result<Self> {
        let cf_script_txs = db.cf(CF_SCRIPT_TXS)?;
        Ok(ScriptTxsWriter {
            db,
            cf_script_txs,
            conf,
        })
    }

    pub fn insert_block_txs<'b>(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: TxNum,
        txs: &[UnhashedTx],
        block_spent_script_fn: impl Fn(/*tx_idx:*/ usize, /*out_idx:*/ usize) -> &'b Script,
        script_txs_cache: &mut ScriptTxsWriterCache,
    ) -> Result<Timings> {
        let timings = Timings::default();
        let payload_tx_nums = prepare_tx_nums_by_payload(first_tx_num, txs, block_spent_script_fn);
        for (script_payload, tx_nums) in payload_tx_nums {
            let start_num_txs = script_txs_cache.get_num_txs_by_payload(
                self.db,
                self.cf_script_txs,
                &self.conf,
                &script_payload,
            )?;
            for (new_tx_idx, tx_num) in tx_nums.iter().cloned().enumerate() {
                let num_txs = start_num_txs + new_tx_idx as u32;
                let page_num = num_txs / self.conf.page_size as u32;
                let key = key_for_script_payload(&script_payload, page_num);
                let mut value = TxNumZC::new(tx_num).as_bytes().to_vec();
                value.insert(0, PREFIX_INSERT);
                batch.merge_cf(self.cf_script_txs, key, value);
            }
            script_txs_cache.increment_num_txs(&script_payload, tx_nums.len() as u32);
        }
        Ok(timings)
    }

    pub fn delete_block_txs<'b>(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: TxNum,
        txs: &[UnhashedTx],
        block_spent_script_fn: impl Fn(/*tx_idx:*/ usize, /*out_idx:*/ usize) -> &'b Script,
        script_txs_cache: &mut ScriptTxsWriterCache,
    ) -> Result<()> {
        let payload_tx_nums = prepare_tx_nums_by_payload(first_tx_num, txs, block_spent_script_fn);
        for (script_payload, tx_nums) in payload_tx_nums {
            let start_num_txs = script_txs_cache.get_num_txs_by_payload(
                self.db,
                self.cf_script_txs,
                &self.conf,
                &script_payload,
            )?;
            let start_num_txs = start_num_txs - tx_nums.len() as u32;
            for (new_tx_idx, tx_num) in tx_nums.iter().cloned().enumerate() {
                let num_txs = start_num_txs + new_tx_idx as u32;
                let page_num = num_txs / self.conf.page_size as u32;
                let key = key_for_script_payload(&script_payload, page_num);
                let mut value = TxNumZC::new(tx_num).as_bytes().to_vec();
                value.insert(0, PREFIX_DELETE);
                batch.merge_cf(self.cf_script_txs, key, value);
            }
            script_txs_cache.decrement_num_txs(&script_payload, tx_nums.len() as u32);
        }
        Ok(())
    }
}

fn key_for_script_payload(script_payload: &[u8], page_num: u32) -> Vec<u8> {
    [script_payload, page_num.to_be_bytes().as_ref()].concat()
}

fn prepare_tx_nums_by_payload<'b>(
    first_tx_num: TxNum,
    txs: &[UnhashedTx],
    block_spent_script_fn: impl Fn(/*tx_idx:*/ usize, /*out_idx:*/ usize) -> &'b Script,
) -> HashMap<Vec<u8>, BTreeSet<TxNum>> {
    let mut payload_tx_nums = HashMap::<_, BTreeSet<TxNum>>::new();
    for (tx_idx, tx) in txs.iter().enumerate() {
        let tx_num = first_tx_num + tx_idx as u64;
        for output in &tx.outputs {
            for (payload_prefix, mut script_payload) in script_payloads(&output.script) {
                script_payload.insert(0, payload_prefix as u8);
                let tx_nums = payload_tx_nums.entry(script_payload).or_default();
                tx_nums.insert(tx_num);
            }
        }
        if tx_idx == 0 {
            // skip coinbase
            continue;
        }
        let tx_pos = tx_idx - 1;
        for input_idx in 0..tx.inputs.len() {
            let spent_script = block_spent_script_fn(tx_pos, input_idx);
            for (payload_prefix, mut script_payload) in script_payloads(spent_script) {
                script_payload.insert(0, payload_prefix as u8);
                let tx_nums = payload_tx_nums.entry(script_payload).or_default();
                tx_nums.insert(tx_num);
            }
        }
    }
    payload_tx_nums
}

impl<'a> ScriptTxsReader<'a> {
    pub fn new(db: &'a Db, conf: ScriptTxsConf) -> Result<Self> {
        let cf_script_txs = db.cf(CF_SCRIPT_TXS)?;
        Ok(ScriptTxsReader {
            db,
            cf_script_txs,
            conf,
        })
    }

    pub fn page_size(&self) -> usize {
        self.conf.page_size
    }

    pub fn num_pages_by_payload(
        &self,
        prefix: PayloadPrefix,
        payload_data: &[u8],
    ) -> Result<usize> {
        let script_payload = [[prefix as u8].as_ref(), payload_data].concat();
        let iterator = self.db.rocks().iterator_cf(
            self.cf_script_txs,
            IteratorMode::From(&script_payload, Direction::Forward),
        );
        let num_pages = iterator
            .take_while(|(key, _)| {
                key.get(..key.len() - PAGE_NUM_SIZE) == Some(script_payload.as_slice())
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
    ) -> Result<Vec<TxNum>> {
        let script_payload = [[prefix as u8].as_ref(), payload_data].concat();
        let key = key_for_script_payload(&script_payload, page_num);
        let value = match self.db.get(self.cf_script_txs, &key)? {
            Some(value) => value,
            None => return Ok(vec![]),
        };
        let entries = interpret_slice::<TxNumZC>(&value)?
            .iter()
            .map(|tx_num| tx_num.get())
            .collect();
        Ok(entries)
    }
}

impl ScriptTxsWriterCache {
    pub fn with_capacity(capacity: usize) -> Self {
        ScriptTxsWriterCache {
            capacity,
            num_txs_by_script: LruCache::new(capacity),
        }
    }

    fn get_num_txs_by_payload(
        &mut self,
        db: &Db,
        cf: &CF,
        conf: &ScriptTxsConf,
        payload: &[u8],
    ) -> Result<u32> {
        if self.capacity > 0 {
            if let Some(&num_txs) = self.num_txs_by_script.get(payload) {
                return Ok(num_txs);
            }
        }
        let last_key = key_for_script_payload(payload, std::u32::MAX);
        let mut iterator = db
            .rocks()
            .iterator_cf(cf, IteratorMode::From(&last_key, Direction::Reverse));
        let (key, value) = loop {
            match iterator.next() {
                Some((key, value)) if &key[..key.len() - PAGE_NUM_SIZE] == payload => {
                    if !value.is_empty() {
                        break (key, value);
                    }
                }
                _ => {
                    if self.capacity > 0 {
                        self.num_txs_by_script.put(payload.to_vec(), 0);
                    }
                    return Ok(0);
                }
            };
        };
        let tx_nums = interpret_slice::<TxNumZC>(&value)?;
        let page_num =
            ScriptPageNum::from_be_bytes(key[key.len() - PAGE_NUM_SIZE..].try_into().unwrap());
        let num_txs = (page_num * conf.page_size as u32) + tx_nums.len() as u32;
        if self.capacity > 0 {
            self.num_txs_by_script.put(payload.to_vec(), num_txs);
        }
        Ok(num_txs)
    }

    fn increment_num_txs(&mut self, payload: &[u8], delta: u32) {
        if let Some(num_txs) = self.num_txs_by_script.get_mut(payload) {
            *num_txs += delta;
        }
    }

    fn decrement_num_txs(&mut self, payload: &[u8], delta: u32) {
        if let Some(num_txs) = self.num_txs_by_script.get_mut(payload) {
            *num_txs -= delta;
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{
        script_txs::key_for_script_payload, Db, PayloadPrefix, ScriptTxsConf, ScriptTxsReader,
        ScriptTxsWriter, ScriptTxsWriterCache, TxNum, TxNumZC,
    };
    use bitcoinsuite_core::{
        ecc::PubKey, OutPoint, Script, Sha256d, ShaRmd160, TxInput, TxOutput, UnhashedTx,
    };
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
        let mut cache = ScriptTxsWriterCache::with_capacity(4);
        let conf = ScriptTxsConf { page_size: 4 };
        let script_txs_writer = ScriptTxsWriter::new(&db, conf.clone())?;
        let script_txs_reader = ScriptTxsReader::new(&db, conf)?;
        let r = &script_txs_reader;
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
        let (script9, payload9) = (Script::p2sh(&ShaRmd160::new([9; 20])), [9; 20]);
        let (script10, payload10) = (Script::p2sh(&ShaRmd160::new([10; 20])), [10; 20]);
        let txs_block1: &[(&[_], &[_])] = &[(&[], &[&script1, &script2])];
        let txs_block2: &[(&[_], &[_])] = &[
            (&[], &[&script1, &script2, &script1, &script1]),
            (&[(0, 0)], &[&script4, &script1]),
            (&[(2, 1)], &[&script5, &script1]),
            (&[(3, 0)], &[&script1, &script3, &script1, &script1]),
        ];
        let txs_block3: &[(&[_], &[_])] = &[
            (&[], &[&script6, &script1]),
            (&[(3, 1), (0, 1)], &[&script7, &script1]),
        ];
        let txs_block4: &[(&[(i32, u32)], &[_])] = &[
            (&[], &[&script10]),
            (&[], &[&script10]),
            (&[], &[&script10]),
            (&[], &[&script10]),
            (&[], &[&script10]),
            (&[], &[&script9]),
        ];
        let txs_blocks = &[txs_block1, txs_block2, txs_block3, txs_block4];
        let mut blocks = Vec::new();
        let mut num_txs: TxNum = 0;
        for &txs_block in txs_blocks {
            let mut block_txids = Vec::new();
            let mut txs = Vec::new();
            let mut block_spent_scripts = Vec::new();
            let first_tx_num = num_txs;
            for (inputs, output_scripts) in txs_block {
                let txid = Sha256d::new([num_txs as u8; 32]);
                num_txs += 1;
                block_txids.push(txid.clone());
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
                    outputs: output_scripts
                        .iter()
                        .map(|&script| TxOutput {
                            value: 0,
                            script: script.clone(),
                        })
                        .collect(),
                    lock_time: 0,
                });
                let mut spent_scripts: Vec<&Script> = Vec::new();
                for &(tx_num, out_idx) in inputs.iter() {
                    let output_scripts = txs_blocks
                        .iter()
                        .flat_map(|txs_block| {
                            txs_block.iter().map(|&(_, output_scripts)| output_scripts)
                        })
                        .nth(tx_num as usize)
                        .unwrap();
                    spent_scripts.push(output_scripts[out_idx as usize]);
                }
                block_spent_scripts.push(spent_scripts);
            }
            block_spent_scripts.remove(0);
            blocks.push((first_tx_num, block_txids, txs, block_spent_scripts));
        }
        let connect_block = |block_height: usize, cache: &mut ScriptTxsWriterCache| -> Result<()> {
            let mut batch = WriteBatch::default();
            script_txs_writer.insert_block_txs(
                &mut batch,
                blocks[block_height].0,
                &blocks[block_height].2,
                |tx_pos, input_idx| blocks[block_height].3[tx_pos][input_idx],
                cache,
            )?;
            db.write_batch(batch)?;
            Ok(())
        };
        let disconnect_block =
            |block_height: usize, cache: &mut ScriptTxsWriterCache| -> Result<()> {
                let mut batch = WriteBatch::default();
                script_txs_writer.delete_block_txs(
                    &mut batch,
                    blocks[block_height].0,
                    &blocks[block_height].2,
                    |tx_pos, input_idx| blocks[block_height].3[tx_pos][input_idx],
                    cache,
                )?;
                db.write_batch(batch)?;
                Ok(())
            };
        {
            check_pages(r, P2PKH, &payload1, [])?;
        }
        {
            connect_block(0, &mut cache)?;
            check_pages(r, P2PKH, &payload1, [&[0]])?;
            check_pages(r, P2PKH, &payload2, [&[0]])?;
            check_pages(r, P2PK, &payload2, [])?;
        }
        {
            disconnect_block(0, &mut cache)?;
            check_pages(r, P2PKH, &payload1, [])?;
            check_pages(r, P2PKH, &payload2, [])?;
        }
        {
            connect_block(0, &mut cache)?;
            connect_block(1, &mut cache)?;
            check_pages(r, P2PKH, &payload1, [&[0, 1, 2, 3], &[4]])?;
            check_pages(r, P2PKH, &payload2, [&[0, 1]])?;
            check_pages(r, P2SH, &payload3, [&[4]])?;
            check_pages(r, P2SH, &payload4, [&[2]])?;
            check_pages(r, P2PK, &payload5, [&[3, 4]])?;
        }
        {
            connect_block(2, &mut cache)?;
            check_pages(r, P2PKH, &payload1, [&[0, 1, 2, 3], &[4, 5, 6]])?;
            check_pages(r, P2PKH, &payload2, [&[0, 1, 6]])?;
            check_pages(r, P2SH, &payload3, [&[4]])?;
            check_pages(r, P2SH, &payload4, [&[2]])?;
            check_pages(r, P2PK, &payload5, [&[3, 4]])?;
            check_pages(r, P2TRCommitment, &payload6, [&[5]])?;
            check_pages(r, P2TRCommitment, &payload7, [&[6]])?;
            check_pages(r, P2TRState, &payload8, [&[6]])?;
        }
        {
            disconnect_block(2, &mut cache)?;
            check_pages(r, P2PKH, &payload1, [&[0, 1, 2, 3], &[4]])?;
            check_pages(r, P2PKH, &payload2, [&[0, 1]])?;
            check_pages(r, P2SH, &payload3, [&[4]])?;
            check_pages(r, P2SH, &payload4, [&[2]])?;
            check_pages(r, P2PK, &payload5, [&[3, 4]])?;
            check_pages(r, P2TRCommitment, &payload6, [])?;
            check_pages(r, P2TRCommitment, &payload7, [])?;
            check_pages(r, P2TRState, &payload8, [])?;
        }
        {
            disconnect_block(1, &mut cache)?;
            check_pages(r, P2PKH, &payload1, [&[0]])?;
            check_pages(r, P2PKH, &payload2, [&[0]])?;
            check_pages(r, P2SH, &payload3, [])?;
            check_pages(r, P2SH, &payload4, [])?;
            check_pages(r, P2PK, &payload5, [])?;
        }
        {
            disconnect_block(0, &mut cache)?;
            check_pages(r, P2PKH, &payload1, [])?;
            check_pages(r, P2PKH, &payload2, [])?;
            check_pages(r, P2SH, &payload3, [])?;
            check_pages(r, P2SH, &payload4, [])?;
            check_pages(r, P2PK, &payload5, [])?;
        }
        {
            // Test with disabled cache
            connect_block(0, &mut ScriptTxsWriterCache::with_capacity(0))?;
            connect_block(1, &mut ScriptTxsWriterCache::with_capacity(0))?;
            connect_block(2, &mut ScriptTxsWriterCache::with_capacity(0))?;
            connect_block(3, &mut ScriptTxsWriterCache::with_capacity(0))?;
            check_pages(r, P2SH, &payload9, [&[12]])?;
            check_pages(r, P2SH, &payload10, [&[7, 8, 9, 10], &[11]])?;
        }
        Ok(())
    }

    fn check_pages<const N: usize>(
        script_txs_reader: &ScriptTxsReader,
        prefix: PayloadPrefix,
        payload_body: &[u8],
        expected_txs: [&[TxNum]; N],
    ) -> Result<()> {
        assert_eq!(
            script_txs_reader.num_pages_by_payload(prefix, payload_body)?,
            N,
        );
        for (page_num, txs) in expected_txs.into_iter().enumerate() {
            assert_eq!(
                script_txs_reader.page_txs(page_num as u32, prefix, payload_body)?,
                txs.to_vec(),
            );
            let script_payload = [[prefix as u8].as_ref(), payload_body].concat();
            let key = key_for_script_payload(&script_payload, page_num as u32);
            let value = script_txs_reader
                .db
                .get(script_txs_reader.cf_script_txs, &key)?
                .unwrap();
            let entry_data = txs.iter().cloned().map(TxNumZC::new).collect::<Vec<_>>();
            assert_eq!(value.as_ref(), entry_data.as_bytes());
        }
        Ok(())
    }
}

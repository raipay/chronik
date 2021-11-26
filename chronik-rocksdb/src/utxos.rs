use std::collections::{hash_map::Entry, HashMap};

use bitcoinsuite_core::{OutPoint, Script, Sha256d, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Result};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rocksdb::{ColumnFamilyDescriptor, Options, WriteBatch};
use thiserror::Error;
use zerocopy::{AsBytes, U32};

use crate::{
    data::interpret_slice, outpoint_data::OutpointData, script_payload::script_payloads, Db,
    OutpointEntry, PayloadPrefix, Timings, TxNum, TxReader, CF,
};

pub const CF_UTXOS: &str = "utxos";

/*
utxos:
script -> [(tx_num, out_idx)]
*/

pub struct UtxosWriter<'a> {
    db: &'a Db,
}

pub struct UtxosReader<'a> {
    db: &'a Db,
    cf_utxos: &'a CF,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum UtxosError {
    #[critical()]
    #[error("Unknown input spent: {0:?}")]
    UnknownInputSpent(OutPoint),

    #[critical()]
    #[error("Inconsistent DB state, UTXO already exists: {0:?}")]
    InconsistentDbUtxoAlreadyExists(OutpointData),

    #[critical()]
    #[error("Inconsistent DB state, UTXO doesn't exists: {0:?}")]
    InconsistentDbUtxoDoesntExists(OutpointData),
}

use self::UtxosError::*;

fn _assert_utxos_writer_send_sync() {
    _assert_send_sync(|_: UtxosWriter| ());
}
fn _assert_send_sync<T: Send + Sync>(_: impl Fn(T)) {}

impl<'a> UtxosWriter<'a> {
    pub fn add_cfs(columns: &mut Vec<ColumnFamilyDescriptor>) {
        let options = Options::default();
        columns.push(ColumnFamilyDescriptor::new(CF_UTXOS, options));
    }

    pub fn new(db: &'a Db) -> Result<Self> {
        let _ = db.cf(CF_UTXOS)?;
        Ok(UtxosWriter { db })
    }

    pub fn insert_block_txs<'b>(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: TxNum,
        txids_fn: impl Fn(usize) -> &'b Sha256d,
        txs: &[UnhashedTx],
        block_spent_script_fn: impl Fn(/*tx_idx:*/ usize, /*out_idx:*/ usize) -> &'b Script,
        input_tx_nums: &[Vec<u64>],
    ) -> Result<Timings> {
        let mut tx_num = first_tx_num;
        let mut new_tx_nums = HashMap::new();
        let mut timings = Timings::default();
        timings.start_timer();
        // All new outpoints (tx_num, out_idx) from outputs by script
        let mut output_outpoints = HashMap::new();
        for (tx_idx, tx) in txs.iter().enumerate() {
            let txid = txids_fn(tx_idx);
            new_tx_nums.insert(txid.clone(), tx_num);
            for (out_idx, output) in tx.outputs.iter().enumerate() {
                for script_payload in script_payloads(&output.script) {
                    let script_payload = script_payload.payload.into_vec();
                    let outpoints = output_outpoints.entry(script_payload).or_insert(vec![]);
                    outpoints.push((tx_num, out_idx as u32));
                }
            }
            tx_num += 1;
        }
        timings.stop_timer("prepare_insert");
        timings.start_timer();
        // Updated UTXOs by script, with new outpoints inserted
        let new_insert_utxos = output_outpoints
            .into_par_iter()
            .map(|(script_payload, outpoints)| {
                let value = self.db.get(self.cf_utxos(), &script_payload)?;
                let mut db_outpoints = match &value {
                    Some(value) => interpret_slice::<OutpointData>(value)?.to_vec(),
                    None => vec![],
                };
                for (tx_num, out_idx) in outpoints {
                    let script_entry = OutpointData {
                        tx_num: tx_num.into(),
                        out_idx: U32::new(out_idx),
                    };
                    match db_outpoints.binary_search(&script_entry) {
                        Err(idx) => db_outpoints.insert(idx, script_entry),
                        Ok(_) => return Err(InconsistentDbUtxoAlreadyExists(script_entry).into()),
                    }
                }
                Ok((script_payload, db_outpoints))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        timings.stop_timer("insert");
        timings.start_timer();
        // All destroyed outpoints (tx_num, out_idx) by script
        let mut input_outpoints = HashMap::new();
        for (tx_pos, (tx, input_tx_nums)) in txs.iter().skip(1).zip(input_tx_nums).enumerate() {
            for (input_idx, (input, spent_tx_num)) in tx
                .inputs
                .iter()
                .zip(input_tx_nums.iter().cloned())
                .enumerate()
            {
                let spent_script = block_spent_script_fn(tx_pos, input_idx);
                for script_payload in script_payloads(spent_script) {
                    let script_payload = script_payload.payload.into_vec();
                    let outpoints = input_outpoints.entry(script_payload).or_insert(vec![]);
                    outpoints.push((spent_tx_num, input.prev_out.out_idx));
                }
            }
        }
        timings.stop_timer("prepare_delete");
        timings.start_timer();
        // Updated UTXOs by script, with destroyed outpoints deleted.
        // Overrides entries which are also present in new_insert_utxos.
        let new_delete_utxos = input_outpoints
            .into_par_iter()
            .map(|(script_payload, spent_outpoints)| {
                let mut outpoints = match new_insert_utxos.get(&script_payload) {
                    Some(outpoints) => outpoints.clone(),
                    None => match self.db.get(self.cf_utxos(), &script_payload)? {
                        Some(value) => interpret_slice::<OutpointData>(&value)?.to_vec(),
                        None => vec![],
                    },
                };
                for (tx_num, out_idx) in spent_outpoints {
                    let script_entry = OutpointData {
                        tx_num: tx_num.into(),
                        out_idx: U32::new(out_idx),
                    };
                    match outpoints.binary_search(&script_entry) {
                        Ok(idx) => {
                            outpoints.remove(idx);
                        }
                        Err(_) => return Err(InconsistentDbUtxoDoesntExists(script_entry).into()),
                    }
                }
                Ok((script_payload, outpoints))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        timings.stop_timer("delete");
        timings.start_timer();
        for (key, value) in &new_delete_utxos {
            match value.is_empty() {
                true => batch.delete_cf(self.cf_utxos(), key),
                false => batch.put_cf(self.cf_utxos(), key, value.as_bytes()),
            }
        }
        for (key, value) in new_insert_utxos {
            if new_delete_utxos.contains_key(&key) {
                // new_delete_utxos overrides new_insert_utxos, so no update
                continue;
            }
            match value.is_empty() {
                true => batch.delete_cf(self.cf_utxos(), key),
                false => batch.put_cf(self.cf_utxos(), key, value.as_bytes()),
            }
        }
        timings.stop_timer("update_batch");
        Ok(timings)
    }

    pub fn delete_block_txs<'b>(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: TxNum,
        txids_fn: impl Fn(usize) -> &'b Sha256d,
        txs: &[UnhashedTx],
        block_spent_script_fn: impl Fn(/*tx_idx:*/ usize, /*out_idx:*/ usize) -> &'b Script,
    ) -> Result<()> {
        let mut new_tx_nums = HashMap::new();
        for tx_idx in 0..txs.len() {
            let txid = txids_fn(tx_idx);
            new_tx_nums.insert(txid.clone(), first_tx_num + tx_idx as TxNum);
        }
        let tx_reader = TxReader::new(self.db)?;
        let mut new_utxos = HashMap::<Vec<u8>, Vec<OutpointData>>::new();
        for (tx_pos, tx) in txs.iter().skip(1).enumerate() {
            for (input_idx, input) in tx.inputs.iter().enumerate() {
                let spent_script = block_spent_script_fn(tx_pos, input_idx);
                let spent_tx_num = match new_tx_nums.get(&input.prev_out.txid) {
                    Some(&tx_num) => tx_num,
                    None => tx_reader
                        .tx_num_by_txid(&input.prev_out.txid)?
                        .ok_or_else(|| UnknownInputSpent(input.prev_out.clone()))?,
                };
                for script_payload in script_payloads(spent_script) {
                    let script_payload = script_payload.payload.into_vec();
                    update_map_or_db_entry(
                        self.db,
                        self.cf_utxos(),
                        &mut new_utxos,
                        script_payload,
                        |outpoints| {
                            let script_entry = OutpointData {
                                tx_num: spent_tx_num.into(),
                                out_idx: U32::new(input.prev_out.out_idx),
                            };
                            if let Err(idx) = outpoints.binary_search(&script_entry) {
                                outpoints.insert(idx, script_entry);
                            }
                        },
                    )?;
                }
            }
        }
        let mut tx_num = first_tx_num;
        for tx in txs {
            for (out_idx, output) in tx.outputs.iter().enumerate() {
                for script_payload in script_payloads(&output.script) {
                    let script_payload = script_payload.payload.into_vec();
                    update_map_or_db_entry(
                        self.db,
                        self.cf_utxos(),
                        &mut new_utxos,
                        script_payload,
                        |outpoints| {
                            let script_entry = OutpointData {
                                tx_num: tx_num.into(),
                                out_idx: U32::new(out_idx as u32),
                            };
                            if let Ok(idx) = outpoints.binary_search(&script_entry) {
                                outpoints.remove(idx);
                            }
                        },
                    )?;
                }
            }
            tx_num += 1;
        }
        for (key, value) in new_utxos {
            match value.is_empty() {
                true => batch.delete_cf(self.cf_utxos(), key),
                false => batch.put_cf(self.cf_utxos(), key, value.as_bytes()),
            }
        }
        Ok(())
    }

    fn cf_utxos(&self) -> &CF {
        self.db.cf(CF_UTXOS).unwrap()
    }
}

impl<'a> UtxosReader<'a> {
    pub fn new(db: &'a Db) -> Result<Self> {
        let cf_utxos = db.cf(CF_UTXOS)?;
        Ok(UtxosReader { db, cf_utxos })
    }

    pub fn utxos(&self, prefix: PayloadPrefix, payload_data: &[u8]) -> Result<Vec<OutpointEntry>> {
        let script_payload = [[prefix as u8].as_ref(), payload_data].concat();
        let value = match self.db.get(self.cf_utxos, &script_payload)? {
            Some(value) => value,
            None => return Ok(vec![]),
        };
        let entries = interpret_slice::<OutpointData>(&value)?
            .iter()
            .map(|entry| OutpointEntry {
                tx_num: entry.tx_num.get(),
                out_idx: entry.out_idx.get(),
            })
            .collect();
        Ok(entries)
    }
}

fn update_map_or_db_entry<'a>(
    db: &Db,
    cf: &CF,
    map: &'a mut HashMap<Vec<u8>, Vec<OutpointData>>,
    key: Vec<u8>,
    f: impl Fn(&mut Vec<OutpointData>),
) -> Result<()> {
    let mut utxo_entry;
    let value = match map.entry(key) {
        Entry::Occupied(entry) => {
            utxo_entry = entry;
            utxo_entry.get_mut()
        }
        Entry::Vacant(vacant) => match db.get(cf, vacant.key())? {
            Some(value) => vacant.insert(interpret_slice::<OutpointData>(&value)?.to_vec()),
            None => vacant.insert(vec![]),
        },
    };
    f(value);
    Ok(())
}

#[cfg(test)]
mod test {
    use crate::{
        input_tx_nums::fetch_input_tx_nums, outpoint_data::OutpointData, BlockHeight, BlockTxs, Db,
        OutpointEntry, PayloadPrefix, TxEntry, TxNum, TxWriter, UtxosReader, UtxosWriter,
    };
    use bitcoinsuite_core::{
        ecc::PubKey, OutPoint, Script, Sha256d, ShaRmd160, TxInput, TxOutput, UnhashedTx,
    };
    use bitcoinsuite_error::Result;
    use pretty_assertions::{assert_eq, assert_ne};
    use rocksdb::WriteBatch;
    use zerocopy::AsBytes;

    #[test]
    fn test_scripts() -> Result<()> {
        use PayloadPrefix::*;
        bitcoinsuite_error::install()?;
        let tempdir = tempdir::TempDir::new("slp-indexer-rocks--utxos")?;
        let db = Db::open(tempdir.path())?;
        let tx_writer = TxWriter::new(&db)?;
        let utxo_writer = UtxosWriter::new(&db)?;
        let utxo_reader = UtxosReader::new(&db)?;
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
        let txs_blocks = &[txs_block1, txs_block2, txs_block3];
        let mut blocks = Vec::new();
        let mut num_txs: TxNum = 0;
        for &txs_block in txs_blocks {
            let mut block_txids = Vec::new();
            let mut block_txs = Vec::new();
            let mut txs = Vec::new();
            let mut block_spent_scripts = Vec::new();
            let first_tx_num = num_txs;
            for (inputs, output_scripts) in txs_block {
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
            blocks.push((
                first_tx_num,
                block_txids,
                txs,
                block_spent_scripts,
                block_txs,
            ));
        }
        let connect_block = |block_height: usize| -> Result<()> {
            let mut batch = WriteBatch::default();
            let input_tx_nums = fetch_input_tx_nums(
                &db,
                blocks[block_height].0,
                |idx| &blocks[block_height].1[idx],
                &blocks[block_height].2,
            )?;
            utxo_writer.insert_block_txs(
                &mut batch,
                blocks[block_height].0,
                |idx| &blocks[block_height].1[idx],
                &blocks[block_height].2,
                |tx_pos, input_idx| blocks[block_height].3[tx_pos][input_idx],
                &input_tx_nums,
            )?;
            tx_writer.insert_block_txs(
                &mut batch,
                &BlockTxs {
                    txs: blocks[block_height].4.clone(),
                    block_height: block_height as BlockHeight,
                },
            )?;
            db.write_batch(batch)?;
            Ok(())
        };
        let disconnect_block = |block_height: usize| -> Result<()> {
            let mut batch = WriteBatch::default();
            utxo_writer.delete_block_txs(
                &mut batch,
                blocks[block_height].0,
                |idx| &blocks[block_height].1[idx],
                &blocks[block_height].2,
                |tx_pos, input_idx| blocks[block_height].3[tx_pos][input_idx],
            )?;
            tx_writer.delete_block_txs(&mut batch, block_height as BlockHeight)?;
            db.write_batch(batch)?;
            Ok(())
        };
        {
            check_utxos(&utxo_reader, P2PKH, &payload1, [])?;
            check_utxos(&utxo_reader, P2PKH, &payload2, [])?;
        }
        {
            // Connect block 0
            connect_block(0)?;
            check_utxos(&utxo_reader, P2PKH, &payload1, [(0, 0)])?;
            check_utxos(&utxo_reader, P2PKH, &payload2, [(0, 1)])?;
        }
        {
            // Connect block 1
            connect_block(1)?;
            check_utxos(
                &utxo_reader,
                P2PKH,
                &payload1,
                [(1, 0), (1, 2), (1, 3), (3, 1), (4, 0), (4, 2), (4, 3)],
            )?;
            check_utxos(&utxo_reader, P2PKH, &payload2, [(0, 1), (1, 1)])?;
            check_utxos(&utxo_reader, P2SH, &payload3, [(4, 1)])?;
            check_utxos(&utxo_reader, P2SH, &payload4, [(2, 0)])?;
            check_utxos(&utxo_reader, P2PK, &payload5, [])?;
        }
        {
            // Disconnect block 1
            disconnect_block(1)?;
            check_utxos(&utxo_reader, P2PKH, &payload1, [(0, 0)])?;
            check_utxos(&utxo_reader, P2PKH, &payload2, [(0, 1)])?;
            check_utxos(&utxo_reader, P2SH, &payload3, [])?;
            check_utxos(&utxo_reader, P2SH, &payload4, [])?;
            check_utxos(&utxo_reader, P2PK, &payload5, [])?;
        }
        {
            // Disconnect block 0
            disconnect_block(0)?;
            check_utxos(&utxo_reader, P2PKH, &payload1, [])?;
            check_utxos(&utxo_reader, P2PKH, &payload2, [])?;
        }
        {
            // Connect block 0, 1, 2
            connect_block(0)?;
            connect_block(1)?;
            connect_block(2)?;
            check_utxos(
                &utxo_reader,
                P2PKH,
                &payload1,
                [
                    (1, 0),
                    (1, 2),
                    (1, 3),
                    (4, 0),
                    (4, 2),
                    (4, 3),
                    (5, 1),
                    (6, 1),
                ],
            )?;
            check_utxos(&utxo_reader, P2PKH, &payload2, [(1, 1)])?;
            check_utxos(&utxo_reader, P2SH, &payload3, [(4, 1)])?;
            check_utxos(&utxo_reader, P2SH, &payload4, [(2, 0)])?;
            check_utxos(&utxo_reader, P2PK, &payload5, [])?;
            check_utxos(&utxo_reader, P2TRCommitment, &payload6, [(5, 0)])?;
            check_utxos(&utxo_reader, P2TRCommitment, &payload7, [(6, 0)])?;
            check_utxos(&utxo_reader, P2TRState, &payload8, [(6, 0)])?;
        }
        {
            // Disconnect block 2
            disconnect_block(2)?;
            check_utxos(&utxo_reader, P2PKH, &payload2, [(0, 1), (1, 1)])?;
            check_utxos(&utxo_reader, P2SH, &payload3, [(4, 1)])?;
            check_utxos(&utxo_reader, P2SH, &payload4, [(2, 0)])?;
            check_utxos(&utxo_reader, P2PK, &payload5, [])?;
            check_utxos(&utxo_reader, P2TRCommitment, &payload6, [])?;
            check_utxos(&utxo_reader, P2TRCommitment, &payload7, [])?;
            check_utxos(&utxo_reader, P2TRState, &payload8, [])?;
        }
        Ok(())
    }

    fn check_utxos<const N: usize>(
        utxo_reader: &UtxosReader,
        prefix: PayloadPrefix,
        payload_body: &[u8],
        expected_txs: [(TxNum, u32); N],
    ) -> Result<()> {
        assert_eq!(
            utxo_reader.utxos(prefix, payload_body)?,
            expected_txs
                .into_iter()
                .map(|(tx_num, out_idx)| OutpointEntry { tx_num, out_idx })
                .collect::<Vec<_>>(),
        );
        let script_payload = [[prefix as u8].as_ref(), payload_body].concat();
        let value = match utxo_reader.db.get(utxo_reader.cf_utxos, &script_payload)? {
            Some(value) => value,
            None => {
                assert_eq!(N, 0);
                return Ok(());
            }
        };
        let entry_data = expected_txs
            .into_iter()
            .map(|(tx_num, out_idx)| OutpointData {
                tx_num: tx_num.into(),
                out_idx: out_idx.into(),
            })
            .collect::<Vec<_>>();
        assert_eq!(value.as_ref(), entry_data.as_bytes());
        assert_ne!(value.as_ref(), &[]);
        Ok(())
    }
}

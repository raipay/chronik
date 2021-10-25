use std::collections::{hash_map::Entry, HashMap};

use bitcoinsuite_core::{OutPoint, Script, Sha256d, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Result};
use rocksdb::{ColumnFamilyDescriptor, Options, WriteBatch};
use thiserror::Error;
use zerocopy::{AsBytes, U32};

use crate::{
    data::interpret_slice, outpoint_data::OutpointData, script_payload::script_payloads, Db,
    OutpointEntry, PayloadPrefix, TxNum, TxReader, CF,
};

pub const CF_UTXOS: &str = "utxos";

/*
utxos:
script -> [(tx_num, out_idx)]
*/

pub struct UtxosWriter<'a> {
    db: &'a Db,
    cf_utxos: &'a CF,
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
}

use self::UtxosError::*;

impl<'a> UtxosWriter<'a> {
    pub fn add_cfs(columns: &mut Vec<ColumnFamilyDescriptor>) {
        let options = Options::default();
        columns.push(ColumnFamilyDescriptor::new(CF_UTXOS, options));
    }

    pub fn new(db: &'a Db) -> Result<Self> {
        let cf_utxos = db.cf(CF_UTXOS)?;
        Ok(UtxosWriter { db, cf_utxos })
    }

    pub fn insert_block_txs<'b>(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: u64,
        block_txids: impl IntoIterator<Item = &'b Sha256d>,
        txs: &[UnhashedTx],
        block_spent_scripts: impl IntoIterator<Item = impl IntoIterator<Item = &'b Script>>,
    ) -> Result<()> {
        let mut tx_num = first_tx_num;
        let mut new_tx_nums = HashMap::new();
        let mut new_utxos = HashMap::<Vec<u8>, Vec<OutpointData>>::new();
        for (tx, txid) in txs.iter().zip(block_txids) {
            new_tx_nums.insert(txid, tx_num);
            for (out_idx, output) in tx.outputs.iter().enumerate() {
                for (payload_prefix, mut script_payload) in script_payloads(&output.script) {
                    script_payload.insert(0, payload_prefix as u8);
                    update_map_or_db_entry(
                        self.db,
                        self.cf_utxos,
                        &mut new_utxos,
                        script_payload,
                        |outpoints| {
                            let script_entry = OutpointData {
                                tx_num: TxNum(tx_num.into()),
                                out_idx: U32::new(out_idx as u32),
                            };
                            if let Err(idx) = outpoints.binary_search(&script_entry) {
                                outpoints.insert(idx, script_entry);
                            }
                        },
                    )?;
                }
            }
            tx_num += 1;
        }
        let tx_reader = TxReader::new(self.db)?;
        for (tx, spent_scripts) in txs.iter().skip(1).zip(block_spent_scripts) {
            for (input, spent_script) in tx.inputs.iter().zip(spent_scripts) {
                let spent_tx_num = match new_tx_nums.get(&input.prev_out.txid) {
                    Some(&tx_num) => tx_num,
                    None => tx_reader
                        .tx_num_by_txid(&input.prev_out.txid)?
                        .ok_or_else(|| UnknownInputSpent(input.prev_out.clone()))?,
                };
                for (payload_prefix, mut script_payload) in script_payloads(spent_script) {
                    script_payload.insert(0, payload_prefix as u8);
                    update_map_or_db_entry(
                        self.db,
                        self.cf_utxos,
                        &mut new_utxos,
                        script_payload,
                        |outpoints| {
                            let script_entry = OutpointData {
                                tx_num: TxNum(spent_tx_num.into()),
                                out_idx: U32::new(input.prev_out.out_idx),
                            };
                            if let Ok(idx) = outpoints.binary_search(&script_entry) {
                                outpoints.remove(idx);
                            }
                        },
                    )?;
                }
            }
        }
        for (key, value) in new_utxos {
            match value.is_empty() {
                true => batch.delete_cf(self.cf_utxos, key),
                false => batch.put_cf(self.cf_utxos, key, value.as_bytes()),
            }
        }
        Ok(())
    }

    pub fn delete_block_txs<'b>(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: u64,
        block_txids: impl IntoIterator<Item = &'b Sha256d>,
        txs: &[UnhashedTx],
        block_spent_scripts: impl IntoIterator<Item = impl IntoIterator<Item = &'b Script>>,
    ) -> Result<()> {
        let mut new_tx_nums = HashMap::new();
        for (tx_idx, txid) in block_txids.into_iter().enumerate() {
            new_tx_nums.insert(txid, first_tx_num + tx_idx as u64);
        }
        let tx_reader = TxReader::new(self.db)?;
        let mut new_utxos = HashMap::<Vec<u8>, Vec<OutpointData>>::new();
        for (tx, spent_scripts) in txs.iter().skip(1).zip(block_spent_scripts) {
            for (input, spent_script) in tx.inputs.iter().zip(spent_scripts) {
                let spent_tx_num = match new_tx_nums.get(&input.prev_out.txid) {
                    Some(&tx_num) => tx_num,
                    None => tx_reader
                        .tx_num_by_txid(&input.prev_out.txid)?
                        .ok_or_else(|| UnknownInputSpent(input.prev_out.clone()))?,
                };
                for (payload_prefix, mut script_payload) in script_payloads(spent_script) {
                    script_payload.insert(0, payload_prefix as u8);
                    update_map_or_db_entry(
                        self.db,
                        self.cf_utxos,
                        &mut new_utxos,
                        script_payload,
                        |outpoints| {
                            let script_entry = OutpointData {
                                tx_num: TxNum(spent_tx_num.into()),
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
                for (payload_prefix, mut script_payload) in script_payloads(&output.script) {
                    script_payload.insert(0, payload_prefix as u8);
                    update_map_or_db_entry(
                        self.db,
                        self.cf_utxos,
                        &mut new_utxos,
                        script_payload,
                        |outpoints| {
                            let script_entry = OutpointData {
                                tx_num: TxNum(tx_num.into()),
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
                true => batch.delete_cf(self.cf_utxos, key),
                false => batch.put_cf(self.cf_utxos, key, value.as_bytes()),
            }
        }
        Ok(())
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
                tx_num: entry.tx_num.0.get(),
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
        outpoint_data::OutpointData, BlockTxs, Db, OutpointEntry, PayloadPrefix, TxEntry, TxNum,
        TxWriter, UtxosReader, UtxosWriter,
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
        let mut num_txs = 0u64;
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
        fn iter_scripts<'b>(
            scripts: &'b [Vec<&'b Script>],
        ) -> impl IntoIterator<Item = impl IntoIterator<Item = &'b Script>> {
            scripts.iter().map(|scripts| scripts.iter().copied())
        }
        let connect_block = |block_height: usize| -> Result<()> {
            let mut batch = WriteBatch::default();
            utxo_writer.insert_block_txs(
                &mut batch,
                blocks[block_height].0,
                &blocks[block_height].1,
                &blocks[block_height].2,
                iter_scripts(&blocks[block_height].3),
            )?;
            tx_writer.insert_block_txs(
                &mut batch,
                &BlockTxs {
                    txs: blocks[block_height].4.clone(),
                    block_height: block_height as i32,
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
                &blocks[block_height].1,
                &blocks[block_height].2,
                iter_scripts(&blocks[block_height].3),
            )?;
            tx_writer.delete_block_txs(&mut batch, block_height as i32)?;
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
        expected_txs: [(u64, u32); N],
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
                tx_num: TxNum(tx_num.into()),
                out_idx: out_idx.into(),
            })
            .collect::<Vec<_>>();
        assert_eq!(value.as_ref(), entry_data.as_bytes());
        assert_ne!(value.as_ref(), &[]);
        Ok(())
    }
}

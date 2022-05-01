use std::collections::{HashMap, HashSet};

use bitcoinsuite_core::Sha256d;
use bitcoinsuite_error::{ErrorMeta, Result};
use thiserror::Error;

use crate::{Db, MempoolData, MempoolDeleteMode, MempoolSlpData, MempoolTxEntry};

pub struct MempoolWriter<'a> {
    pub db: &'a Db,
    pub mempool: &'a mut MempoolData,
    pub mempool_slp: &'a mut MempoolSlpData,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum MempoolError {
    #[critical()]
    #[error("No such mempool tx: {0}")]
    NoSuchTx(Sha256d),

    #[critical()]
    #[error("Cycle in mempool: {0:?}")]
    MempoolCycle(HashSet<Sha256d>),
}

use self::MempoolError::*;

impl<'a> MempoolWriter<'a> {
    pub fn insert_mempool_tx(&mut self, txid: Sha256d, entry: MempoolTxEntry) -> Result<()> {
        self.mempool_slp
            .insert_mempool_tx(self.db, &txid, &entry.tx)?;
        self.mempool
            .insert_mempool_tx(txid, entry.tx, entry.spent_coins, entry.time_first_seen)?;
        Ok(())
    }

    pub fn delete_mempool_tx(&mut self, txid: &Sha256d, mode: MempoolDeleteMode) -> Result<()> {
        self.mempool_slp.delete_mempool_tx(txid);
        self.mempool.delete_mempool_tx(txid, mode)?;
        Ok(())
    }

    pub fn insert_mempool_batch_txs(
        &mut self,
        mut txs: HashMap<Sha256d, MempoolTxEntry>,
    ) -> Result<()> {
        let mut next_round = HashMap::new();
        loop {
            let txids = txs.keys().cloned().collect::<HashSet<_>>();
            let mut is_only_orphans = true;
            'tx_loop: for (txid, entry) in txs {
                for input in &entry.tx.inputs {
                    if txids.contains(&input.prev_out.txid) {
                        next_round.insert(txid, entry);
                        continue 'tx_loop;
                    }
                }
                is_only_orphans = false;
                self.insert_mempool_tx(txid, entry)?;
            }
            if next_round.is_empty() {
                return Ok(());
            }
            if is_only_orphans {
                return Err(MempoolCycle(next_round.keys().cloned().collect()).into());
            }
            txs = next_round;
            next_round = HashMap::new();
        }
    }

    pub fn delete_mempool_mined_txs(&mut self, mut txids: HashSet<&Sha256d>) -> Result<()> {
        let mut next_round = HashSet::new();
        loop {
            let mut is_only_parents = true;
            'tx_loop: for &txid in &txids {
                let entry = self
                    .mempool
                    .tx(txid)
                    .ok_or_else(|| NoSuchTx(txid.clone()))?;
                for input in &entry.tx.inputs {
                    if txids.contains(&input.prev_out.txid) {
                        next_round.insert(txid);
                        continue 'tx_loop;
                    }
                }
                is_only_parents = false;
                self.delete_mempool_tx(txid, MempoolDeleteMode::Mined)?;
            }
            if next_round.is_empty() {
                return Ok(());
            }
            if is_only_parents {
                return Err(MempoolCycle(next_round.into_iter().cloned().collect()).into());
            }
            txids = next_round;
            next_round = HashSet::new();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use bitcoinsuite_core::{Coin, OutPoint, Script, Sha256d, TxInput, TxOutput, UnhashedTx};
    use bitcoinsuite_error::Result;
    use bitcoinsuite_slp::{
        genesis_opreturn, send_opreturn, SlpAmount, SlpGenesisInfo, SlpToken, SlpTokenType,
        SlpTxData, SlpTxType, SlpValidTxData, TokenId,
    };
    use pretty_assertions::assert_eq;
    use rocksdb::WriteBatch;

    use crate::{
        input_tx_nums::fetch_input_tx_nums, BlockTxs, Db, MempoolData, MempoolSlpData,
        MempoolTxEntry, MempoolWriter, SlpWriter, TxEntry, TxWriter,
    };

    #[test]
    fn test_mempool_batch() -> Result<()> {
        bitcoinsuite_error::install()?;
        let tempdir = tempdir::TempDir::new("slp-indexer-rocks--mempool")?;
        let db = Db::open(tempdir.path())?;
        let tx_writer = TxWriter::new(&db)?;
        let slp_writer = SlpWriter::new(&db)?;
        let token_id = TokenId::new(make_hash(4));
        let (block_txids, block_txs) = make_block([
            make_tx((1, [(0, 0xffff_ffff)], 5), Script::default()),
            make_tx((2, [(1, 1)], 2), Script::default()),
            make_tx((3, [(1, 2)], 3), Script::default()),
            make_tx(
                (4, [(1, 3)], 3),
                genesis_opreturn(&SlpGenesisInfo::default(), SlpTokenType::Fungible, None, 10),
            ),
            make_tx(
                (5, [(4, 1)], 3),
                send_opreturn(
                    &token_id,
                    SlpTokenType::Fungible,
                    &[SlpAmount::new(3), SlpAmount::new(7)],
                ),
            ),
        ]);
        {
            // Validate initial block
            let mut batch = WriteBatch::default();
            let input_tx_nums = fetch_input_tx_nums(&db, 0, |idx| &block_txids[idx], &block_txs)?;
            slp_writer.insert_block_txs(
                &mut batch,
                0,
                &block_txs,
                |idx| &block_txids[idx],
                &input_tx_nums,
            )?;
            let block_txs = block_txids
                .iter()
                .cloned()
                .map(|txid| TxEntry {
                    txid,
                    data_pos: 0,
                    tx_size: 0,
                    undo_pos: 0,
                    undo_size: 0,
                    time_first_seen: 0,
                    is_coinbase: false,
                })
                .collect::<Vec<_>>();
            tx_writer.insert_block_txs(
                &mut batch,
                &BlockTxs {
                    txs: block_txs,
                    block_height: 0,
                },
            )?;
            db.write_batch(batch)?;
        }
        let mut mempool = MempoolData::default();
        let mut mempool_slp = MempoolSlpData::default();
        let mut mempool_writer = MempoolWriter {
            db: &db,
            mempool: &mut mempool,
            mempool_slp: &mut mempool_slp,
        };
        let mempool_batch = [
            make_tx((10, [(2, 0)], 3), Script::default()),
            make_tx((11, [(10, 2), (2, 1)], 3), Script::default()),
            make_tx((12, [(11, 0), (3, 0), (13, 0)], 3), Script::default()),
            make_tx((13, [(3, 1)], 4), Script::default()),
            make_tx((14, [(13, 1)], 3), Script::default()),
            make_tx((15, [(14, 0), (13, 3)], 3), Script::default()),
            make_tx((16, [(15, 0), (13, 2), (3, 2)], 3), Script::default()),
            make_tx(
                (17, [(5, 1)], 3),
                send_opreturn(
                    &token_id,
                    SlpTokenType::Fungible,
                    &[SlpAmount::new(1), SlpAmount::new(2)],
                ),
            ),
            make_tx(
                (18, [(5, 2), (17, 1)], 3),
                send_opreturn(&token_id, SlpTokenType::Fungible, &[SlpAmount::new(8)]),
            ),
        ];
        // Drop txs out of mempool (due to mining) in this order:
        let mine_blocks: &[&[u8]] = &[&[10, 11, 17], &[13, 12], &[14, 15, 16], &[18]];
        // Run multiple times to cover different orders of the HashMap
        for _ in 0..100 {
            let txs = mempool_batch
                .iter()
                .map(|(txid, tx)| {
                    (
                        txid.clone(),
                        MempoolTxEntry {
                            tx: tx.clone(),
                            spent_coins: vec![Coin::default(); tx.inputs.len()],
                            time_first_seen: 0,
                        },
                    )
                })
                .collect::<HashMap<_, _>>();
            mempool_writer.insert_mempool_batch_txs(txs)?;
            for hash_byte in 10..=18 {
                assert!(
                    mempool_writer.mempool.tx(&make_hash(hash_byte)).is_some(),
                    "Tx {} not in mempool",
                    hash_byte,
                );
            }
            assert_eq!(
                mempool_writer.mempool_slp.slp_tx_error(&make_hash(17)),
                None
            );
            assert_eq!(
                mempool_writer.mempool_slp.slp_tx_data(&make_hash(17)),
                Some(&SlpValidTxData {
                    slp_tx_data: SlpTxData {
                        input_tokens: vec![SlpToken::amount(3)],
                        output_tokens: vec![
                            SlpToken::EMPTY,
                            SlpToken::amount(1),
                            SlpToken::amount(2)
                        ],
                        slp_token_type: SlpTokenType::Fungible,
                        slp_tx_type: SlpTxType::Send,
                        token_id: token_id.clone(),
                        group_token_id: None,
                    },
                    slp_burns: vec![None],
                }),
            );
            assert_eq!(
                mempool_writer.mempool_slp.slp_tx_error(&make_hash(18)),
                None
            );
            assert_eq!(
                mempool_writer.mempool_slp.slp_tx_data(&make_hash(18)),
                Some(&SlpValidTxData {
                    slp_tx_data: SlpTxData {
                        input_tokens: vec![SlpToken::amount(7), SlpToken::amount(1)],
                        output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(8)],
                        slp_token_type: SlpTokenType::Fungible,
                        slp_tx_type: SlpTxType::Send,
                        token_id: token_id.clone(),
                        group_token_id: None,
                    },
                    slp_burns: vec![None, None],
                }),
            );
            for &mine_block in mine_blocks {
                let txids = mine_block
                    .iter()
                    .map(|&hash_byte| make_hash(hash_byte))
                    .collect::<Vec<_>>();
                let txids = txids.iter().collect::<HashSet<_>>();
                mempool_writer.delete_mempool_mined_txs(txids)?;
            }
            for hash_byte in 10..=18 {
                let txid = make_hash(hash_byte);
                assert!(
                    mempool_writer.mempool.tx(&txid).is_none(),
                    "Tx {} in mempool",
                    hash_byte,
                );
                assert_eq!(mempool_writer.mempool_slp.slp_tx_data(&txid), None);
                assert_eq!(mempool_writer.mempool_slp.slp_tx_error(&txid), None);
            }
        }
        Ok(())
    }

    fn make_block<const N: usize>(
        txs: [(Sha256d, UnhashedTx); N],
    ) -> (Vec<Sha256d>, Vec<UnhashedTx>) {
        let (txids, txs): (Vec<_>, Vec<_>) = txs.into_iter().map(|(txid, tx)| (txid, tx)).unzip();
        (txids, txs)
    }

    fn make_tx<const N: usize>(
        shape: (u8, [(u8, u32); N], usize),
        slp_script: Script,
    ) -> (Sha256d, UnhashedTx) {
        let (txid_byte, inputs, num_outputs) = shape;
        (
            make_hash(txid_byte),
            UnhashedTx {
                version: 1,
                inputs: inputs
                    .iter()
                    .map(|&(input_byte, out_idx)| TxInput {
                        prev_out: OutPoint {
                            txid: make_hash(input_byte),
                            out_idx,
                        },
                        ..Default::default()
                    })
                    .collect(),
                outputs: std::iter::once(TxOutput {
                    value: 0,
                    script: slp_script,
                })
                .chain(vec![TxOutput::default(); num_outputs - 1])
                .into_iter()
                .collect(),
                lock_time: 0,
            },
        )
    }

    fn make_hash(byte: u8) -> Sha256d {
        let mut hash = [0; 32];
        hash[31] = byte;
        Sha256d::new(hash)
    }
}

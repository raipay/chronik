use std::collections::HashMap;

use bitcoinsuite_core::{Sha256d, UnhashedTx};
use bitcoinsuite_error::Result;
use bitcoinsuite_slp::{
    parse_slp_tx, validate_slp_tx, SlpError, SlpSpentOutput, SlpTxType, SlpValidTxData, TokenId,
};

use crate::{is_ignored_error, Db, SlpReader, TokenStats, TxReader};

#[derive(Debug, Default)]
pub struct MempoolSlpData {
    valid_slp_txs: HashMap<Sha256d, SlpValidTxData>,
    invalid_slp_txs: HashMap<Sha256d, SlpError>,
    token_stats_delta: HashMap<[u8; 32], TokenStats>,
}

impl MempoolSlpData {
    pub fn insert_mempool_tx(&mut self, db: &Db, txid: &Sha256d, tx: &UnhashedTx) -> Result<()> {
        let spent_outputs = self.collect_spent_outputs(db, tx)?;
        let adder = |a: &mut i128, b: i128| *a += b;
        match self.validate_slp_tx(txid, tx, &spent_outputs)? {
            Ok(valid_tx_data) => {
                self.update_token_stats(Some(&valid_tx_data), &spent_outputs, adder);
                self.valid_slp_txs.insert(txid.clone(), valid_tx_data);
            }
            Err(slp_error) => {
                self.update_token_stats(None, &spent_outputs, adder);
                if !is_ignored_error(&slp_error) {
                    self.invalid_slp_txs.insert(txid.clone(), slp_error);
                }
            }
        }
        Ok(())
    }

    pub fn delete_mempool_tx(&mut self, db: &Db, txid: &Sha256d, tx: &UnhashedTx) -> Result<()> {
        let spent_outputs = self.collect_spent_outputs(db, tx)?;
        let subber = |a: &mut i128, b: i128| *a -= b;
        match self.valid_slp_txs.remove(txid) {
            Some(valid_tx_data) => {
                self.update_token_stats(Some(&valid_tx_data), &spent_outputs, subber)
            }
            None => self.update_token_stats(None, &spent_outputs, subber),
        }
        self.invalid_slp_txs.remove(txid);
        Ok(())
    }

    pub fn validate_slp_tx(
        &self,
        txid: &Sha256d,
        tx: &UnhashedTx,
        spent_outputs: &[Option<SlpSpentOutput>],
    ) -> Result<std::result::Result<SlpValidTxData, SlpError>> {
        let parse_data = match parse_slp_tx(txid, tx) {
            Ok(parse_data) => parse_data,
            Err(slp_error) => return Ok(Err(slp_error)),
        };
        match validate_slp_tx(
            parse_data,
            &spent_outputs
                .iter()
                .map(|spent_output| spent_output.as_ref())
                .collect::<Vec<_>>(),
        ) {
            Ok(valid_tx_data) => Ok(Ok(valid_tx_data)),
            Err(slp_error) => Ok(Err(slp_error)),
        }
    }

    pub fn collect_spent_outputs(
        &self,
        db: &Db,
        tx: &UnhashedTx,
    ) -> Result<Vec<Option<SlpSpentOutput>>> {
        let tx_reader = TxReader::new(db)?;
        let slp_reader = SlpReader::new(db)?;
        let mut spent_outputs = Vec::with_capacity(tx.inputs.len());
        for input in &tx.inputs {
            let out_idx = input.prev_out.out_idx as usize;
            spent_outputs.push(match self.valid_slp_txs.get(&input.prev_out.txid) {
                Some(tx_data) => {
                    let slp = &tx_data.slp_tx_data;
                    slp.output_tokens.get(out_idx).map(|&token| SlpSpentOutput {
                        token_id: slp.token_id.clone(),
                        token_type: slp.slp_token_type,
                        token,
                        group_token_id: slp.group_token_id.clone(),
                    })
                }
                None => match self.invalid_slp_txs.get(&input.prev_out.txid) {
                    Some(_) => None,
                    None => tx_reader
                        .tx_num_by_txid(&input.prev_out.txid)?
                        .and_then(|tx_num| slp_reader.slp_data_by_tx_num(tx_num).transpose())
                        .transpose()?
                        .and_then(|slp| {
                            let token = *slp.slp_tx_data.output_tokens.get(out_idx)?;
                            Some(SlpSpentOutput {
                                token_id: slp.slp_tx_data.token_id,
                                token_type: slp.slp_tx_data.slp_token_type,
                                token,
                                group_token_id: slp.slp_tx_data.group_token_id,
                            })
                        }),
                },
            });
        }
        Ok(spent_outputs)
    }

    fn update_token_stats(
        &mut self,
        valid_tx_data: Option<&SlpValidTxData>,
        spent_outputs: &[Option<SlpSpentOutput>],
        op: impl Fn(&mut i128, i128),
    ) {
        fn update_token(
            this: &mut MempoolSlpData,
            token_id: &TokenId,
            f: impl Fn(&mut TokenStats),
        ) {
            let token_stats = this
                .token_stats_delta
                .entry(token_id.token_id_be())
                .or_default();
            f(token_stats);
            if token_stats == &TokenStats::default() {
                this.token_stats_delta.remove(token_id.as_slice_be());
            }
        }
        match valid_tx_data {
            // SEND has the correct burns computed
            Some(slp) if slp.slp_tx_data.slp_tx_type == SlpTxType::Send => {
                for burn in slp.slp_burns.iter().flatten() {
                    update_token(self, &burn.token_id, |token_stats| {
                        op(
                            &mut token_stats.total_burned,
                            burn.token.amount.base_amount(),
                        );
                    });
                }
                return; // SEND doesn't mint
            }
            // Others burn all inputs (see SlpWriter::update_token_stats for details)
            _ => {
                for spent_output in spent_outputs.iter().flatten() {
                    update_token(self, &spent_output.token_id, |token_stats| {
                        op(
                            &mut token_stats.total_burned,
                            spent_output.token.amount.base_amount(),
                        );
                    });
                }
            }
        }
        let slp_tx_data = match valid_tx_data {
            Some(valid_tx_data) => &valid_tx_data.slp_tx_data,
            None => return,
        };
        // GENESIS and MINT can mint
        if let SlpTxType::Genesis(_) | SlpTxType::Mint = &slp_tx_data.slp_tx_type {
            for token in &slp_tx_data.output_tokens {
                update_token(self, &slp_tx_data.token_id, |token_stats| {
                    op(&mut token_stats.total_minted, token.amount.base_amount());
                });
            }
        }
    }

    pub fn slp_tx_data(&self, txid: &Sha256d) -> Option<&SlpValidTxData> {
        self.valid_slp_txs.get(txid)
    }

    pub fn slp_tx_error(&self, txid: &Sha256d) -> Option<&SlpError> {
        self.invalid_slp_txs.get(txid)
    }

    pub fn token_stats_delta(&self, token_id: &TokenId) -> Option<&TokenStats> {
        self.token_stats_delta.get(token_id.as_slice_be())
    }
}

#[cfg(test)]
mod tests {
    use bitcoinsuite_core::{OutPoint, Script, Sha256d, TxInput, TxOutput, UnhashedTx};
    use bitcoinsuite_error::Result;
    use bitcoinsuite_slp::{
        genesis_opreturn, mint_opreturn, send_opreturn, SlpAmount, SlpBurn, SlpError,
        SlpGenesisInfo, SlpToken, SlpTokenType, SlpTxData, SlpTxType, SlpValidTxData, TokenId,
    };
    use pretty_assertions::assert_eq;
    use rocksdb::WriteBatch;

    use crate::{
        input_tx_nums::fetch_input_tx_nums, BlockTxs, Db, MempoolSlpData, SlpReader, SlpWriter,
        TokenStats, TxEntry, TxWriter,
    };

    #[test]
    fn test_slp_mempool() -> Result<()> {
        let tempdir = tempdir::TempDir::new("slp-indexer-rocks--utxos")?;
        let db = Db::open(tempdir.path())?;
        let tx_writer = TxWriter::new(&db)?;
        let slp_writer = SlpWriter::new(&db)?;
        let slp_reader = SlpReader::new(&db)?;
        let token_id = TokenId::new(make_hash(2));
        let (block_txids, block_txs) = make_block([
            make_tx((1, [(0, 0xffff_ffff)], 3), Script::opreturn(&[])),
            make_tx(
                (2, [(1, 1)], 3),
                genesis_opreturn(&SlpGenesisInfo::default(), SlpTokenType::Fungible, None, 13),
            ),
            make_tx(
                (3, [(2, 1)], 4),
                send_opreturn(
                    &token_id,
                    SlpTokenType::Fungible,
                    &[SlpAmount::new(3), SlpAmount::new(7), SlpAmount::new(3)],
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
        let token_id = TokenId::new(make_hash(2));
        let token_num = slp_reader.token_num_by_id(&token_id)?.unwrap();
        assert_eq!(
            slp_reader.token_stats_by_token_num(token_num)?,
            Some(TokenStats {
                total_minted: 13,
                total_burned: 0,
            }),
        );

        let mut slp_mempool = MempoolSlpData::default();
        let (txid0, tx0) = make_tx(
            (10, [(3, 1)], 2),
            send_opreturn(
                &token_id,
                SlpTokenType::Fungible,
                &[SlpAmount::new(1), SlpAmount::new(2)],
            ),
        );
        slp_mempool.insert_mempool_tx(&db, &txid0, &tx0)?;
        assert_eq!(slp_mempool.slp_tx_error(&txid0), None);
        assert_eq!(
            slp_mempool.slp_tx_data(&txid0),
            Some(&SlpValidTxData {
                slp_tx_data: SlpTxData {
                    input_tokens: vec![SlpToken::amount(3)],
                    output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(1), SlpToken::amount(2)],
                    slp_token_type: SlpTokenType::Fungible,
                    slp_tx_type: SlpTxType::Send,
                    token_id: token_id.clone(),
                    group_token_id: None,
                },
                slp_burns: vec![None],
            })
        );

        let (txid1, tx1) = make_tx(
            (11, [(10, 1), (3, 2)], 2),
            send_opreturn(&token_id, SlpTokenType::Fungible, &[SlpAmount::new(9)]),
        );
        slp_mempool.insert_mempool_tx(&db, &txid1, &tx1)?;
        assert_eq!(slp_mempool.slp_tx_data(&txid1), None);
        assert_eq!(
            slp_mempool.slp_tx_error(&txid1),
            Some(&SlpError::OutputSumExceedInputSum {
                input_sum: SlpAmount::new(8),
                output_sum: SlpAmount::new(9),
            })
        );
        assert_eq!(
            slp_mempool.token_stats_delta(&token_id),
            Some(&TokenStats {
                total_minted: 0,
                total_burned: 8,
            }),
        );

        slp_mempool.delete_mempool_tx(&db, &txid1, &tx1)?;
        assert_eq!(slp_mempool.slp_tx_data(&txid1), None);
        assert_eq!(slp_mempool.slp_tx_error(&txid1), None);
        assert_eq!(slp_mempool.token_stats_delta(&token_id), None,);

        slp_mempool.delete_mempool_tx(&db, &txid0, &tx0)?;
        assert_eq!(slp_mempool.slp_tx_data(&txid0), None);
        assert_eq!(slp_mempool.slp_tx_error(&txid0), None);

        let (txid0, tx0) = make_tx((10, [(3, 1)], 2), Script::opreturn(&[b"SLP\0"]));
        slp_mempool.insert_mempool_tx(&db, &txid0, &tx0)?;
        assert_eq!(slp_mempool.slp_tx_data(&txid0), None);
        assert_eq!(
            slp_mempool.slp_tx_error(&txid0),
            Some(&SlpError::TooFewPushes {
                expected: 3,
                actual: 1,
            })
        );
        assert_eq!(
            slp_mempool.token_stats_delta(&token_id),
            Some(&TokenStats {
                total_minted: 0,
                total_burned: 3,
            }),
        );

        let (txid1, tx1) = make_tx((11, [(3, 2)], 2), Script::from_slice(b"\x04SLP\0\x01"));
        slp_mempool.insert_mempool_tx(&db, &txid1, &tx1)?;
        assert_eq!(slp_mempool.slp_tx_data(&txid1), None);
        assert_eq!(slp_mempool.slp_tx_error(&txid1), None);
        assert_eq!(
            slp_mempool.token_stats_delta(&token_id),
            Some(&TokenStats {
                total_minted: 0,
                total_burned: 10,
            }),
        );

        let (txid2, tx2) = make_tx(
            (12, [(10, 1)], 3),
            genesis_opreturn(
                &SlpGenesisInfo::default(),
                SlpTokenType::Fungible,
                Some(2),
                1000,
            ),
        );
        slp_mempool.insert_mempool_tx(&db, &txid2, &tx2)?;
        let token_id2 = TokenId::new(txid2.clone());
        assert_eq!(slp_mempool.slp_tx_error(&txid2), None);
        assert_eq!(
            slp_mempool.slp_tx_data(&txid2),
            Some(&SlpValidTxData {
                slp_tx_data: SlpTxData {
                    input_tokens: vec![SlpToken::EMPTY],
                    output_tokens: vec![
                        SlpToken::EMPTY,
                        SlpToken::amount(1000),
                        SlpToken::MINT_BATON
                    ],
                    slp_token_type: SlpTokenType::Fungible,
                    slp_tx_type: SlpTxType::Genesis(Default::default()),
                    token_id: token_id2.clone(),
                    group_token_id: None,
                },
                slp_burns: vec![None],
            })
        );
        assert_eq!(
            slp_mempool.token_stats_delta(&token_id2),
            Some(&TokenStats {
                total_minted: 1000,
                total_burned: 0,
            }),
        );

        let (txid3, tx3) = make_tx(
            (13, [(12, 2)], 3),
            mint_opreturn(&token_id2, SlpTokenType::Fungible, Some(2), 400),
        );
        slp_mempool.insert_mempool_tx(&db, &txid3, &tx3)?;
        assert_eq!(slp_mempool.slp_tx_error(&txid3), None);
        assert_eq!(
            slp_mempool.slp_tx_data(&txid3),
            Some(&SlpValidTxData {
                slp_tx_data: SlpTxData {
                    input_tokens: vec![SlpToken::MINT_BATON],
                    output_tokens: vec![
                        SlpToken::EMPTY,
                        SlpToken::amount(400),
                        SlpToken::MINT_BATON
                    ],
                    slp_token_type: SlpTokenType::Fungible,
                    slp_tx_type: SlpTxType::Mint,
                    token_id: token_id2.clone(),
                    group_token_id: None,
                },
                slp_burns: vec![None],
            })
        );
        assert_eq!(
            slp_mempool.token_stats_delta(&token_id2),
            Some(&TokenStats {
                total_minted: 1400,
                total_burned: 0,
            }),
        );

        let (txid4, tx4) = make_tx(
            (14, [(3, 3), (10, 1), (12, 1)], 2),
            send_opreturn(
                &token_id,
                SlpTokenType::Fungible,
                &[SlpAmount::new(1), SlpAmount::new(2)],
            ),
        );
        slp_mempool.insert_mempool_tx(&db, &txid4, &tx4)?;
        assert_eq!(slp_mempool.slp_tx_error(&txid4), None);
        assert_eq!(
            slp_mempool.slp_tx_data(&txid4),
            Some(&SlpValidTxData {
                slp_tx_data: SlpTxData {
                    input_tokens: vec![SlpToken::amount(3), SlpToken::EMPTY, SlpToken::EMPTY],
                    output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(1), SlpToken::amount(2)],
                    slp_token_type: SlpTokenType::Fungible,
                    slp_tx_type: SlpTxType::Send,
                    token_id: token_id.clone(),
                    group_token_id: None,
                },
                slp_burns: vec![
                    None,
                    None,
                    Some(Box::new(SlpBurn {
                        token: SlpToken::amount(1000),
                        token_id: token_id2.clone(),
                    }))
                ],
            })
        );
        assert_eq!(
            slp_mempool.token_stats_delta(&token_id2),
            Some(&TokenStats {
                total_minted: 1400,
                total_burned: 1000,
            }),
        );

        slp_mempool.delete_mempool_tx(&db, &txid1, &tx1)?;
        assert_eq!(slp_mempool.slp_tx_data(&txid1), None);
        assert_eq!(slp_mempool.slp_tx_error(&txid1), None);
        assert_eq!(
            slp_mempool.token_stats_delta(&token_id),
            Some(&TokenStats {
                total_minted: 0,
                total_burned: 3,
            })
        );

        slp_mempool.delete_mempool_tx(&db, &txid0, &tx0)?;
        assert_eq!(slp_mempool.slp_tx_data(&txid0), None);
        assert_eq!(slp_mempool.slp_tx_error(&txid0), None);
        assert_eq!(slp_mempool.token_stats_delta(&token_id), None);

        slp_mempool.delete_mempool_tx(&db, &txid2, &tx2)?;
        assert_eq!(slp_mempool.slp_tx_data(&txid2), None);
        assert_eq!(slp_mempool.slp_tx_error(&txid2), None);
        assert_eq!(
            slp_mempool.token_stats_delta(&token_id2),
            Some(&TokenStats {
                total_minted: 400,
                total_burned: 1000,
            })
        );

        slp_mempool.delete_mempool_tx(&db, &txid3, &tx3)?;
        assert_eq!(slp_mempool.slp_tx_data(&txid3), None);
        assert_eq!(slp_mempool.slp_tx_error(&txid3), None);
        assert_eq!(
            slp_mempool.token_stats_delta(&token_id2),
            Some(&TokenStats {
                total_minted: 0,
                total_burned: 1000,
            })
        );

        slp_mempool.delete_mempool_tx(&db, &txid4, &tx4)?;
        assert_eq!(slp_mempool.slp_tx_data(&txid4), None);
        assert_eq!(slp_mempool.slp_tx_error(&txid4), None);
        assert_eq!(slp_mempool.token_stats_delta(&token_id2), None);

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

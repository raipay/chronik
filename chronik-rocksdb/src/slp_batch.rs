use bitcoinsuite_slp::{validate_slp_tx, SlpBurn, SlpParseData, SlpSpentOutput, SlpTx};
use thiserror::Error;

use std::collections::{HashMap, HashSet};

use bitcoinsuite_core::UnhashedTx;

use crate::OutpointEntry;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum BatchError {
    #[error("Batch contains txs forming a circle: {0:?}")]
    FoundTxCircle(HashSet<u64>),
}

pub struct BatchSlpTx {
    pub tx: UnhashedTx,
    pub parsed_tx_data: SlpParseData,
    pub input_tx_nums: Vec<u64>,
}

pub fn validate_slp_batch(
    mut txs: HashMap<u64, BatchSlpTx>,
    mut known_slp_outputs: HashMap<OutpointEntry, Option<SlpSpentOutput>>,
) -> Result<HashMap<u64, SlpTx>, BatchError> {
    let mut result = HashMap::new();
    let tx_nums = txs.keys().copied().collect::<HashSet<_>>();
    loop {
        let mut next_round = HashMap::new();
        let mut is_only_orphans = true;
        'tx_loop: for (tx_num, batch_tx) in txs {
            // Check whether all input tokens for this tx are known
            for (input, &input_tx_num) in batch_tx.tx.inputs.iter().zip(&batch_tx.input_tx_nums) {
                if input.prev_out.is_coinbase() {
                    // coinbase txs cannot have token inputs
                    continue;
                }
                let outpoint = OutpointEntry {
                    tx_num: input_tx_num,
                    out_idx: input.prev_out.out_idx,
                };
                // input_tx_num in:
                //   txs and known_slp_outputs -> token known
                //   txs but not known_slp_outputs -> token not known yet
                //   not txs but in known_slp_outputs -> token known
                //   neither -> assumed to be 0
                if !known_slp_outputs.contains_key(&outpoint) && tx_nums.contains(&input_tx_num) {
                    // Input token not yet known, validate in later iteration of outer loop
                    next_round.insert(tx_num, batch_tx);
                    continue 'tx_loop;
                }
            }
            is_only_orphans = false;
            let spent_outputs = batch_tx
                .tx
                .inputs
                .iter()
                .zip(&batch_tx.input_tx_nums)
                .map(|(input, &input_tx_num)| {
                    let outpoint = OutpointEntry {
                        tx_num: input_tx_num,
                        out_idx: input.prev_out.out_idx,
                    };
                    known_slp_outputs
                        .get(&outpoint)
                        .and_then(|spent_output| spent_output.as_ref())
                })
                .collect::<Vec<_>>();
            match validate_slp_tx(batch_tx.parsed_tx_data, &spent_outputs) {
                Ok(valid_tx_data) => {
                    for (out_idx, &token) in
                        valid_tx_data.slp_tx_data.output_tokens.iter().enumerate()
                    {
                        known_slp_outputs.insert(
                            OutpointEntry {
                                tx_num,
                                out_idx: out_idx as u32,
                            },
                            Some(SlpSpentOutput {
                                token_id: valid_tx_data.slp_tx_data.token_id.clone(),
                                token_type: valid_tx_data.slp_tx_data.slp_token_type,
                                token,
                                group_token_id: valid_tx_data.slp_tx_data.group_token_id.clone(),
                            }),
                        );
                    }
                    result.insert(
                        tx_num,
                        SlpTx::new(
                            batch_tx.tx,
                            Some(valid_tx_data.slp_tx_data),
                            valid_tx_data.slp_burns,
                        ),
                    );
                }
                Err(_err) => {
                    let num_outputs = batch_tx.tx.outputs.len();
                    result.insert(
                        tx_num,
                        SlpTx::new(
                            batch_tx.tx,
                            None,
                            spent_outputs
                                .iter()
                                .map(|spent_output| {
                                    spent_output.map(|spent_output| {
                                        Box::new(SlpBurn {
                                            token: spent_output.token,
                                            token_id: spent_output.token_id.clone(),
                                        })
                                    })
                                })
                                .collect(),
                        ),
                    );
                    for out_idx in 0..num_outputs {
                        known_slp_outputs.insert(
                            OutpointEntry {
                                tx_num,
                                out_idx: out_idx as u32,
                            },
                            None,
                        );
                    }
                }
            }
        }
        if is_only_orphans {
            return Err(BatchError::FoundTxCircle(next_round.into_keys().collect()));
        }
        if next_round.is_empty() {
            return Ok(result);
        }
        txs = next_round;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bitcoinsuite_core::{OutPoint, Sha256d, TxInput, TxOutput, UnhashedTx};
    use bitcoinsuite_slp::{
        SlpBurn, SlpParseData, SlpSpentOutput, SlpToken, SlpTokenType, SlpTx, SlpTxData, SlpTxType,
        TokenId,
    };
    use pretty_assertions::assert_eq;

    use crate::{validate_slp_batch, BatchError, BatchSlpTx, OutpointEntry};

    #[test]
    fn test_validate_slp_batch_circle() {
        // Run multiple times, random hash will process in different order each time
        for _ in 0..20 {
            let parsed_tx_data = SlpParseData {
                output_tokens: vec![],
                slp_token_type: SlpTokenType::Fungible,
                slp_tx_type: SlpTxType::Send,
                token_id: TokenId::new(Sha256d::new([10; 32])),
            };
            let txs = [
                (
                    1,
                    BatchSlpTx {
                        tx: make_tx([2], 2),
                        input_tx_nums: vec![2],
                        parsed_tx_data: parsed_tx_data.clone(),
                    },
                ),
                (
                    2,
                    BatchSlpTx {
                        tx: make_tx([2], 2),
                        input_tx_nums: vec![1],
                        parsed_tx_data: parsed_tx_data.clone(),
                    },
                ),
            ]
            .into_iter()
            .collect();
            let known_slp_outputs = HashMap::new();
            assert_eq!(
                validate_slp_batch(txs, known_slp_outputs),
                Err(BatchError::FoundTxCircle([1, 2].into_iter().collect()))
            );
        }
    }

    #[test]
    fn test_validate_slp_batch_genesis() {
        // Run multiple times, random hash will process in different order each time
        for _ in 0..20 {
            let token_id = TokenId::new(Sha256d::new([10; 32]));
            let parsed_tx_data = SlpParseData {
                output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(10)],
                slp_token_type: SlpTokenType::Fungible,
                slp_tx_type: SlpTxType::Genesis(Default::default()),
                token_id: token_id.clone(),
            };
            let txs = [
                (
                    3,
                    BatchSlpTx {
                        tx: make_tx([1], 2),
                        input_tx_nums: vec![2],
                        parsed_tx_data: SlpParseData {
                            slp_tx_type: SlpTxType::Send,
                            ..parsed_tx_data.clone()
                        },
                    },
                ),
                (
                    2,
                    BatchSlpTx {
                        tx: make_tx([1], 2),
                        input_tx_nums: vec![1],
                        parsed_tx_data,
                    },
                ),
            ]
            .into_iter()
            .collect();
            let known_slp_outputs = HashMap::new();
            assert_eq!(
                validate_slp_batch(txs, known_slp_outputs),
                Ok([
                    (
                        2,
                        SlpTx::new(
                            make_tx([1], 2),
                            Some(SlpTxData {
                                input_tokens: vec![SlpToken::EMPTY],
                                output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(10)],
                                slp_token_type: SlpTokenType::Fungible,
                                slp_tx_type: SlpTxType::Genesis(Default::default()),
                                token_id: token_id.clone(),
                                group_token_id: None,
                            }),
                            vec![None],
                        ),
                    ),
                    (
                        3,
                        SlpTx::new(
                            make_tx([1], 2),
                            Some(SlpTxData {
                                input_tokens: vec![SlpToken::amount(10)],
                                output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(10)],
                                slp_token_type: SlpTokenType::Fungible,
                                slp_tx_type: SlpTxType::Send,
                                token_id: token_id.clone(),
                                group_token_id: None,
                            }),
                            vec![None],
                        ),
                    ),
                ]
                .into_iter()
                .collect()),
            );
        }
    }

    #[test]
    fn test_validate_slp_batch_mint() {
        // Run multiple times, random hash will process in different order each time
        for _ in 0..20 {
            let token_id = TokenId::new(Sha256d::new([8; 32]));
            let parsed_tx_data = SlpParseData {
                output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(10)],
                slp_token_type: SlpTokenType::Fungible,
                slp_tx_type: SlpTxType::Mint,
                token_id: token_id.clone(),
            };
            let tx = UnhashedTx {
                inputs: vec![TxInput {
                    prev_out: OutPoint {
                        txid: Sha256d::new([2; 32]),
                        out_idx: 1,
                    },
                    ..Default::default()
                }],
                outputs: vec![TxOutput::default(); 2],
                ..Default::default()
            };
            let txs = [
                (
                    3,
                    BatchSlpTx {
                        tx: tx.clone(),
                        input_tx_nums: vec![2],
                        parsed_tx_data: SlpParseData {
                            slp_tx_type: SlpTxType::Send,
                            ..parsed_tx_data.clone()
                        },
                    },
                ),
                (
                    2,
                    BatchSlpTx {
                        tx: tx.clone(),
                        input_tx_nums: vec![1],
                        parsed_tx_data,
                    },
                ),
            ]
            .into_iter()
            .collect();
            let known_slp_outputs = [(
                OutpointEntry {
                    tx_num: 1,
                    out_idx: 1,
                },
                Some(SlpSpentOutput {
                    token_id: token_id.clone(),
                    token_type: SlpTokenType::Fungible,
                    token: SlpToken::MINT_BATON,
                    group_token_id: None,
                }),
            )]
            .into_iter()
            .collect();
            assert_eq!(
                validate_slp_batch(txs, known_slp_outputs),
                Ok([
                    (
                        2,
                        SlpTx::new(
                            tx.clone(),
                            Some(SlpTxData {
                                input_tokens: vec![SlpToken::MINT_BATON],
                                output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(10)],
                                slp_token_type: SlpTokenType::Fungible,
                                slp_tx_type: SlpTxType::Mint,
                                token_id: token_id.clone(),
                                group_token_id: None,
                            }),
                            vec![None],
                        ),
                    ),
                    (
                        3,
                        SlpTx::new(
                            tx.clone(),
                            Some(SlpTxData {
                                input_tokens: vec![SlpToken::amount(10)],
                                output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(10)],
                                slp_token_type: SlpTokenType::Fungible,
                                slp_tx_type: SlpTxType::Send,
                                token_id: token_id.clone(),
                                group_token_id: None,
                            }),
                            vec![None],
                        ),
                    ),
                ]
                .into_iter()
                .collect()),
            );
        }
    }

    #[test]
    fn test_validate_slp_batch_complex() {
        // Run multiple times, random hash will process in different order each time
        for _ in 0..200 {
            let token_id1 = TokenId::new(Sha256d::new([8; 32]));
            let token_id2_group = TokenId::new(Sha256d::new([9; 32]));
            let token_id2_child = TokenId::new(Sha256d::new([10; 32]));
            let token_id3 = TokenId::new(Sha256d::new([11; 32]));
            let txs = [
                (
                    11,
                    BatchSlpTx {
                        tx: make_tx([1], 2),
                        input_tx_nums: vec![3],
                        parsed_tx_data: SlpParseData {
                            output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(10)],
                            slp_token_type: SlpTokenType::Fungible,
                            slp_tx_type: SlpTxType::Genesis(Default::default()),
                            token_id: token_id1.clone(),
                        },
                    },
                ),
                (
                    12,
                    BatchSlpTx {
                        tx: make_tx([1], 2),
                        input_tx_nums: vec![1],
                        parsed_tx_data: SlpParseData {
                            output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(1)],
                            slp_token_type: SlpTokenType::Nft1Child,
                            slp_tx_type: SlpTxType::Genesis(Default::default()),
                            token_id: token_id2_child.clone(),
                        },
                    },
                ),
                (
                    13,
                    BatchSlpTx {
                        tx: make_tx([1], 3),
                        input_tx_nums: vec![11],
                        parsed_tx_data: SlpParseData {
                            output_tokens: vec![
                                SlpToken::EMPTY,
                                SlpToken::amount(3),
                                SlpToken::amount(7),
                            ],
                            slp_token_type: SlpTokenType::Fungible,
                            slp_tx_type: SlpTxType::Send,
                            token_id: token_id1.clone(),
                        },
                    },
                ),
                (
                    // overspend, burns the token
                    14,
                    BatchSlpTx {
                        tx: make_tx([1], 2),
                        input_tx_nums: vec![13],
                        parsed_tx_data: SlpParseData {
                            output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(4)],
                            slp_token_type: SlpTokenType::Fungible,
                            slp_tx_type: SlpTxType::Send,
                            token_id: token_id1.clone(),
                        },
                    },
                ),
                (
                    // burned input
                    15,
                    BatchSlpTx {
                        tx: make_tx([1], 2),
                        input_tx_nums: vec![14],
                        parsed_tx_data: SlpParseData {
                            output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(1)],
                            slp_token_type: SlpTokenType::Fungible,
                            slp_tx_type: SlpTxType::Send,
                            token_id: token_id1.clone(),
                        },
                    },
                ),
                (
                    16,
                    BatchSlpTx {
                        tx: make_tx([1], 2),
                        input_tx_nums: vec![2],
                        parsed_tx_data: SlpParseData {
                            output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(6)],
                            slp_token_type: SlpTokenType::Fungible,
                            slp_tx_type: SlpTxType::Send,
                            token_id: token_id3.clone(),
                        },
                    },
                ),
            ]
            .into_iter()
            .collect();
            let known_slp_outputs = [
                (
                    OutpointEntry {
                        tx_num: 1,
                        out_idx: 1,
                    },
                    Some(SlpSpentOutput {
                        token_id: token_id2_group.clone(),
                        token_type: SlpTokenType::Nft1Group,
                        token: SlpToken::amount(1),
                        group_token_id: None,
                    }),
                ),
                (
                    OutpointEntry {
                        tx_num: 2,
                        out_idx: 1,
                    },
                    Some(SlpSpentOutput {
                        token_id: token_id3.clone(),
                        token_type: SlpTokenType::Fungible,
                        token: SlpToken::amount(6),
                        group_token_id: None,
                    }),
                ),
            ]
            .into_iter()
            .collect();
            let batch = validate_slp_batch(txs, known_slp_outputs).unwrap();
            let mut batch = batch.into_iter().collect::<Vec<_>>();
            batch.sort_unstable_by_key(|&(key, _)| key);
            assert_eq!(
                batch,
                [
                    (
                        11,
                        SlpTx::new(
                            make_tx([1], 2),
                            Some(SlpTxData {
                                input_tokens: vec![SlpToken::EMPTY],
                                output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(10)],
                                slp_token_type: SlpTokenType::Fungible,
                                slp_tx_type: SlpTxType::Genesis(Default::default()),
                                token_id: token_id1.clone(),
                                group_token_id: None,
                            }),
                            vec![None],
                        ),
                    ),
                    (
                        12,
                        SlpTx::new(
                            make_tx([1], 2),
                            Some(SlpTxData {
                                input_tokens: vec![SlpToken::amount(1)],
                                output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(1)],
                                slp_token_type: SlpTokenType::Nft1Child,
                                slp_tx_type: SlpTxType::Genesis(Default::default()),
                                token_id: token_id2_child.clone(),
                                group_token_id: Some(token_id2_group.clone().into()),
                            }),
                            vec![None],
                        ),
                    ),
                    (
                        13,
                        SlpTx::new(
                            make_tx([1], 3),
                            Some(SlpTxData {
                                input_tokens: vec![SlpToken::amount(10)],
                                output_tokens: vec![
                                    SlpToken::EMPTY,
                                    SlpToken::amount(3),
                                    SlpToken::amount(7)
                                ],
                                slp_token_type: SlpTokenType::Fungible,
                                slp_tx_type: SlpTxType::Send,
                                token_id: token_id1.clone(),
                                group_token_id: None,
                            }),
                            vec![None],
                        ),
                    ),
                    (
                        14,
                        SlpTx::new(
                            make_tx([1], 2),
                            None,
                            vec![Some(Box::new(SlpBurn {
                                token: SlpToken::amount(3),
                                token_id: token_id1.clone(),
                            }))],
                        ),
                    ),
                    (15, SlpTx::new(make_tx([1], 2), None, vec![None])),
                    (
                        16,
                        SlpTx::new(
                            make_tx([1], 2),
                            Some(SlpTxData {
                                input_tokens: vec![SlpToken::amount(6)],
                                output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(6)],
                                slp_token_type: SlpTokenType::Fungible,
                                slp_tx_type: SlpTxType::Send,
                                token_id: token_id3.clone(),
                                group_token_id: None,
                            }),
                            vec![None],
                        ),
                    ),
                ]
                .into_iter()
                .collect::<Vec<_>>(),
            );
        }
    }

    fn make_tx<const N: usize>(input_out_indices: [u32; N], num_outputs: usize) -> UnhashedTx {
        UnhashedTx {
            inputs: input_out_indices
                .into_iter()
                .map(|out_idx| TxInput {
                    prev_out: OutPoint {
                        txid: Sha256d::new([2; 32]),
                        out_idx,
                    },
                    ..Default::default()
                })
                .collect(),
            outputs: vec![TxOutput::default(); num_outputs],
            ..Default::default()
        }
    }
}

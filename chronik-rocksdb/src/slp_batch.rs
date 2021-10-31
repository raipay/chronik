use bitcoinsuite_slp::{
    validate_slp_tx, SlpBurn, SlpError, SlpParseData, SlpSpentOutput, SlpValidTxData,
};
use thiserror::Error;

use std::collections::{HashMap, HashSet};

use bitcoinsuite_core::UnhashedTx;

use crate::{OutpointEntry, TxNum};

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum BatchError {
    #[error("Batch contains txs forming a circle: {0:?}")]
    FoundTxCircle(HashSet<TxNum>),
}

pub struct BatchSlpTx<'a> {
    pub tx: &'a UnhashedTx,
    pub parsed_tx_data: SlpParseData,
    pub input_tx_nums: Vec<Option<TxNum>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlpInvalidTxData {
    pub slp_burns: Vec<Option<Box<SlpBurn>>>,
    pub slp_error: SlpError,
}

pub type SlpValidHashMap = HashMap<TxNum, SlpValidTxData>;
pub type SlpInvalidHashMap = HashMap<TxNum, SlpInvalidTxData>;

pub fn validate_slp_batch(
    mut txs: HashMap<TxNum, BatchSlpTx>,
    mut known_slp_outputs: HashMap<OutpointEntry, Option<SlpSpentOutput>>,
) -> Result<(SlpValidHashMap, SlpInvalidHashMap), BatchError> {
    let mut valid_results = HashMap::new();
    let mut invalid_results = HashMap::new();
    let tx_nums = txs.keys().copied().collect::<HashSet<_>>();
    loop {
        let mut next_round = HashMap::new();
        let mut is_only_orphans = true;
        'tx_loop: for (tx_num, batch_tx) in txs {
            // Check whether all input tokens for this tx are known
            for (input, &input_tx_num) in batch_tx.tx.inputs.iter().zip(&batch_tx.input_tx_nums) {
                // Input doesn't exist (e.g. coinbase), assume to be 0
                let input_tx_num = match input_tx_num {
                    Some(input_tx_num) => input_tx_num,
                    None => continue,
                };
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
                        tx_num: input_tx_num?,
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
                    valid_results.insert(tx_num, valid_tx_data);
                }
                Err(err) => {
                    let num_outputs = batch_tx.tx.outputs.len();
                    invalid_results.insert(
                        tx_num,
                        SlpInvalidTxData {
                            slp_burns: spent_outputs
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
                            slp_error: err,
                        },
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
            return Ok((valid_results, invalid_results));
        }
        txs = next_round;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bitcoinsuite_core::{OutPoint, Sha256d, TxInput, TxOutput, UnhashedTx};
    use bitcoinsuite_slp::{
        SlpAmount, SlpBurn, SlpError, SlpParseData, SlpSpentOutput, SlpToken, SlpTokenType,
        SlpTxData, SlpTxType, SlpValidTxData, TokenId,
    };
    use pretty_assertions::assert_eq;

    use crate::{validate_slp_batch, BatchError, BatchSlpTx, OutpointEntry, SlpInvalidTxData};

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
                        input_tx_nums: [2].into_iter().map(Some).collect(),
                        parsed_tx_data: parsed_tx_data.clone(),
                    },
                ),
                (
                    2,
                    BatchSlpTx {
                        tx: make_tx([2], 2),
                        input_tx_nums: [1].into_iter().map(Some).collect(),
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
                        input_tx_nums: [2].into_iter().map(Some).collect(),
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
                        input_tx_nums: [1].into_iter().map(Some).collect(),
                        parsed_tx_data,
                    },
                ),
            ]
            .into_iter()
            .collect();
            let known_slp_outputs = HashMap::new();
            assert_eq!(
                validate_slp_batch(txs, known_slp_outputs),
                Ok((
                    [
                        (
                            2,
                            SlpValidTxData {
                                slp_tx_data: SlpTxData {
                                    input_tokens: vec![SlpToken::EMPTY],
                                    output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(10)],
                                    slp_token_type: SlpTokenType::Fungible,
                                    slp_tx_type: SlpTxType::Genesis(Default::default()),
                                    token_id: token_id.clone(),
                                    group_token_id: None,
                                },
                                slp_burns: vec![None],
                            },
                        ),
                        (
                            3,
                            SlpValidTxData {
                                slp_tx_data: SlpTxData {
                                    input_tokens: vec![SlpToken::amount(10)],
                                    output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(10)],
                                    slp_token_type: SlpTokenType::Fungible,
                                    slp_tx_type: SlpTxType::Send,
                                    token_id: token_id.clone(),
                                    group_token_id: None,
                                },
                                slp_burns: vec![None],
                            },
                        ),
                    ]
                    .into_iter()
                    .collect(),
                    HashMap::new(),
                )),
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
                        tx: &tx,
                        input_tx_nums: [2].into_iter().map(Some).collect(),
                        parsed_tx_data: SlpParseData {
                            slp_tx_type: SlpTxType::Send,
                            ..parsed_tx_data.clone()
                        },
                    },
                ),
                (
                    2,
                    BatchSlpTx {
                        tx: &tx,
                        input_tx_nums: [1].into_iter().map(Some).collect(),
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
                Ok((
                    [
                        (
                            2,
                            SlpValidTxData {
                                slp_tx_data: SlpTxData {
                                    input_tokens: vec![SlpToken::MINT_BATON],
                                    output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(10)],
                                    slp_token_type: SlpTokenType::Fungible,
                                    slp_tx_type: SlpTxType::Mint,
                                    token_id: token_id.clone(),
                                    group_token_id: None,
                                },
                                slp_burns: vec![None],
                            },
                        ),
                        (
                            3,
                            SlpValidTxData {
                                slp_tx_data: SlpTxData {
                                    input_tokens: vec![SlpToken::amount(10)],
                                    output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(10)],
                                    slp_token_type: SlpTokenType::Fungible,
                                    slp_tx_type: SlpTxType::Send,
                                    token_id: token_id.clone(),
                                    group_token_id: None,
                                },
                                slp_burns: vec![None],
                            },
                        ),
                    ]
                    .into_iter()
                    .collect(),
                    HashMap::new()
                )),
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
                        input_tx_nums: [3].into_iter().map(Some).collect(),
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
                        input_tx_nums: [1].into_iter().map(Some).collect(),
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
                        input_tx_nums: [11].into_iter().map(Some).collect(),
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
                        input_tx_nums: [13].into_iter().map(Some).collect(),
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
                        input_tx_nums: [14].into_iter().map(Some).collect(),
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
                        input_tx_nums: [2].into_iter().map(Some).collect(),
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
            let (valid_batch, invalid_batch) = validate_slp_batch(txs, known_slp_outputs).unwrap();
            let mut valid_batch = valid_batch.into_iter().collect::<Vec<_>>();
            valid_batch.sort_unstable_by_key(|&(key, _)| key);
            let mut invalid_batch = invalid_batch.into_iter().collect::<Vec<_>>();
            invalid_batch.sort_unstable_by_key(|&(key, _)| key);
            assert_eq!(
                valid_batch,
                [
                    (
                        11,
                        SlpValidTxData {
                            slp_tx_data: SlpTxData {
                                input_tokens: vec![SlpToken::EMPTY],
                                output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(10)],
                                slp_token_type: SlpTokenType::Fungible,
                                slp_tx_type: SlpTxType::Genesis(Default::default()),
                                token_id: token_id1.clone(),
                                group_token_id: None,
                            },
                            slp_burns: vec![None],
                        },
                    ),
                    (
                        12,
                        SlpValidTxData {
                            slp_tx_data: SlpTxData {
                                input_tokens: vec![SlpToken::amount(1)],
                                output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(1)],
                                slp_token_type: SlpTokenType::Nft1Child,
                                slp_tx_type: SlpTxType::Genesis(Default::default()),
                                token_id: token_id2_child.clone(),
                                group_token_id: Some(token_id2_group.clone().into()),
                            },
                            slp_burns: vec![None],
                        },
                    ),
                    (
                        13,
                        SlpValidTxData {
                            slp_tx_data: SlpTxData {
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
                            },
                            slp_burns: vec![None],
                        },
                    ),
                    (
                        16,
                        SlpValidTxData {
                            slp_tx_data: SlpTxData {
                                input_tokens: vec![SlpToken::amount(6)],
                                output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(6)],
                                slp_token_type: SlpTokenType::Fungible,
                                slp_tx_type: SlpTxType::Send,
                                token_id: token_id3.clone(),
                                group_token_id: None,
                            },
                            slp_burns: vec![None],
                        },
                    ),
                ]
                .into_iter()
                .collect::<Vec<_>>(),
            );
            assert_eq!(
                invalid_batch,
                [
                    (
                        14,
                        SlpInvalidTxData {
                            slp_burns: vec![Some(Box::new(SlpBurn {
                                token: SlpToken::amount(3),
                                token_id: token_id1.clone(),
                            }))],
                            slp_error: SlpError::OutputSumExceedInputSum {
                                input_sum: SlpAmount::new(3),
                                output_sum: SlpAmount::new(4),
                            },
                        },
                    ),
                    (
                        15,
                        SlpInvalidTxData {
                            slp_burns: vec![None],
                            slp_error: SlpError::OutputSumExceedInputSum {
                                input_sum: SlpAmount::ZERO,
                                output_sum: SlpAmount::new(1),
                            },
                        },
                    ),
                ]
                .into_iter()
                .collect::<Vec<_>>(),
            );
        }
    }

    fn make_tx<const N: usize>(
        input_out_indices: [u32; N],
        num_outputs: usize,
    ) -> &'static UnhashedTx {
        Box::leak(Box::new(UnhashedTx {
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
        }))
    }
}

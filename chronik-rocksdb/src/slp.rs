use std::collections::{HashMap, HashSet};

use bitcoinsuite_core::{OutPoint, Sha256d, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Result};
use bitcoinsuite_slp::{
    parse_slp_tx, SlpAmount, SlpBurn, SlpError, SlpGenesisInfo, SlpParseData, SlpSpentOutput,
    SlpToken, SlpTokenType, SlpTxData, SlpTxType, SlpValidTxData, TokenId,
};
use byteorder::{BE, LE};
use rayon::iter::{
    Either, IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator,
    ParallelIterator,
};
use rocksdb::{ColumnFamilyDescriptor, IteratorMode, Options, WriteBatch};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zerocopy::{AsBytes, FromBytes, Unaligned, I128, U32};

use crate::{
    data::interpret, validate_slp_batch, BatchSlpTx, Db, OutpointEntry, SlpInvalidTxData,
    SlpValidHashMap, TxNum, TxNumZC, CF,
};

pub const CF_SLP_TOKEN_ID_BY_NUM: &str = "slp_token_id_by_num";
pub const CF_SLP_TOKEN_NUM_BY_ID: &str = "slp_token_num_by_id";
pub const CF_SLP_TOKEN_METADATA: &str = "slp_token_metadata";
pub const CF_SLP_TX_DATA: &str = "slp_tx_data";
pub const CF_SLP_TX_INVALID_MESSAGE: &str = "slp_tx_invalid_message";
pub const CF_SLP_TOKEN_STATS: &str = "slp_token_stats";

type TokenNum = u32;
type TokenNumZC = U32<BE>;

pub struct SlpWriter<'a> {
    db: &'a Db,
}

pub struct SlpReader<'a> {
    db: &'a Db,
}

#[derive(Debug, Clone, FromBytes, AsBytes, Unaligned, PartialEq, Eq, Default)]
#[repr(C)]
struct TokenStatsData {
    // Total number of coins minted via GENESIS or MINT
    total_minted: I128<LE>,
    // Total number of coins burned (in any way)
    total_burned: I128<LE>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenStats {
    // Total number of coins minted via GENESIS or MINT
    pub total_minted: i128,
    // Total number of coins burned (in any way)
    pub total_burned: i128,
}

struct SlpInputToken<'t> {
    token_id: &'t TokenId,
    token: &'t SlpToken,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum SlpWriterError {
    #[critical()]
    #[error("Failed fetching input tx_num: Unknown input spent: {0:?}")]
    UnknownInputSpent(OutPoint),

    #[critical()]
    #[error("Inconsistent CF_SLP_TOKEN_ID_BY_NUM, token num {0} not found")]
    InconsistentDbTokenIdByNum(TokenNum),

    #[critical()]
    #[error("Inconsistent CF_SLP_TOKEN_ID_BY_NUM, token ID {0:?} not found")]
    InconsistentDbTokenNumById(TokenId),

    #[critical()]
    #[error("Inconsistent CF_SLP_TX_DATA, tx {0} has unknown token num {1}")]
    InconsistentDbNoSuchTokenNum(TxNum, TokenNum),

    #[critical()]
    #[error("Inconsistent CF_SLP_TX_DATA, tx {0} has unknown token ID {1:?}")]
    InconsistentDbNoSuchTokenId(TxNum, TokenId),

    #[critical()]
    #[error("Inconsistent SLP GENESIS, tx {0} has null token")]
    InconsistentDbNullTokenGenesis(TxNum),

    #[critical()]
    #[error("Inconsistent slp entry, tx {0} has null token")]
    InconsistentDbNullTokenGroupId(TxNum),

    #[critical()]
    #[error("Inconsistent token_num_by_id, token {0:?} does not exist")]
    InconsistentTokenNumById(TokenId),
}

use self::SlpWriterError::*;

#[derive(Deserialize, Serialize, Clone)]
enum SerSlpToken {
    Amount(u64),
    MintBaton,
}

#[derive(Deserialize, Serialize, Clone)]
enum SerSlpTxType {
    Genesis = 0,
    Send = 1,
    Mint = 2,
    Burn = 4,
    Unknown = 3,
}

#[derive(Deserialize, Serialize, Clone)]
struct SerSlpBurn {
    token_id_num: Option<TokenNum>,
    token: SerSlpToken,
}

#[derive(Deserialize, Serialize, Clone)]
struct SerSlpTxEntry {
    input_tokens: Vec<SerSlpToken>,
    output_tokens: Vec<SerSlpToken>,
    slp_token_type: SlpTokenType,
    slp_tx_type: SerSlpTxType,
    token_num: Option<TokenNum>,
    group_token_num: Option<TokenNum>,
    slp_burns: Vec<Option<SerSlpBurn>>,
}

impl<'a> SlpWriter<'a> {
    pub fn add_cfs(columns: &mut Vec<ColumnFamilyDescriptor>) {
        columns.push(ColumnFamilyDescriptor::new(
            CF_SLP_TOKEN_ID_BY_NUM,
            Options::default(),
        ));
        columns.push(ColumnFamilyDescriptor::new(
            CF_SLP_TOKEN_NUM_BY_ID,
            Options::default(),
        ));
        columns.push(ColumnFamilyDescriptor::new(
            CF_SLP_TOKEN_METADATA,
            Options::default(),
        ));
        columns.push(ColumnFamilyDescriptor::new(
            CF_SLP_TX_DATA,
            Options::default(),
        ));
        columns.push(ColumnFamilyDescriptor::new(
            CF_SLP_TX_INVALID_MESSAGE,
            Options::default(),
        ));
        columns.push(ColumnFamilyDescriptor::new(
            CF_SLP_TOKEN_STATS,
            Options::default(),
        ));
    }

    pub fn new(db: &'a Db) -> Result<Self> {
        db.cf(CF_SLP_TOKEN_ID_BY_NUM)?;
        db.cf(CF_SLP_TOKEN_NUM_BY_ID)?;
        db.cf(CF_SLP_TOKEN_METADATA)?;
        db.cf(CF_SLP_TX_DATA)?;
        db.cf(CF_SLP_TX_INVALID_MESSAGE)?;
        Ok(SlpWriter { db })
    }

    pub fn insert_block_txs<'b>(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: TxNum,
        txs: &[UnhashedTx],
        txid_fn: impl Fn(usize) -> &'b Sha256d + Send + Sync,
        input_tx_nums: &[Vec<TxNum>],
    ) -> Result<()> {
        let (parsed_slp_txs, invalid_parsed_slp_txs) = Self::parse_block_slp_txs(txs, &txid_fn);
        let next_token_num = self.get_next_token_num()?;
        // Short-circuit for block without any SLP txs, and if there's no tokens yet
        if parsed_slp_txs.is_empty() && invalid_parsed_slp_txs.is_empty() && next_token_num == 0 {
            return Ok(());
        }
        // Fetch the SLP state of all inputs
        let spent_slp_outputs = self.fetch_spent_slp_outputs(txs, input_tx_nums)?;
        // Bundle SLP txs into BatchSlpTxs
        let batch_txs = self.build_batch_txs(parsed_slp_txs, txs, input_tx_nums);
        // Bundle batch_txs into known_slp_outputs
        let known_slp_outputs = self.build_known_slp_outputs(&batch_txs, &spent_slp_outputs);
        // Turn vec of (tx_idx, batch_tx) into HashMap of tx_num => batch_tx
        let batch_txs: HashMap<TxNum, BatchSlpTx> = batch_txs
            .into_iter()
            .map(|(tx_idx, batch_tx)| (first_tx_num + tx_idx as TxNum, batch_tx))
            .collect();
        // Validate SLP batch
        let (valid_slp_txs, invalid_slp_txs) = validate_slp_batch(batch_txs, known_slp_outputs)?;
        // Insert new tokens
        let mut token_num_by_id = self.insert_new_tokens(batch, valid_slp_txs.values())?;
        // Insert SLP txs
        self.insert_new_valid_txs(batch, valid_slp_txs.iter(), &mut token_num_by_id)?;
        // Insert token stats
        self.update_token_stats(
            batch,
            first_tx_num,
            txs,
            &valid_slp_txs,
            input_tx_nums,
            &spent_slp_outputs,
            &mut token_num_by_id,
            |a, b| a + b,
        )?;
        // Insert invalid SLP txs
        self.insert_new_invalid_txs(batch, first_tx_num, invalid_parsed_slp_txs, invalid_slp_txs);
        Ok(())
    }

    /// Parse txs, split into valid and invalid (skip non-SLP)
    #[allow(clippy::type_complexity)]
    fn parse_block_slp_txs<'b>(
        txs: &[UnhashedTx],
        txid_fn: &(impl Fn(usize) -> &'b Sha256d + Send + Sync),
    ) -> (Vec<(usize, SlpParseData)>, Vec<(usize, SlpError)>) {
        txs.par_iter()
            .enumerate()
            .filter_map(|(tx_idx, tx)| {
                let txid = txid_fn(tx_idx);
                match parse_slp_tx(txid, tx) {
                    Ok(slp_parse) => Some((tx_idx, Ok(slp_parse))),
                    Err(err) => match is_ignored_error(&err) {
                        true => None,
                        false => {
                            eprintln!("Invalid SLP tx {}: {}", txid, err);
                            Some((tx_idx, Err(err)))
                        }
                    },
                }
            })
            .partition_map(|(tx_idx, tx_result)| match tx_result {
                Ok(slp_parse_data) => Either::Left((tx_idx, slp_parse_data)),
                Err(err) => Either::Right((tx_idx, err)),
            })
    }

    fn fetch_spent_slp_outputs(
        &self,
        txs: &[UnhashedTx],
        input_tx_nums: &[Vec<TxNum>],
    ) -> Result<Vec<Vec<Option<SlpSpentOutput>>>> {
        txs.par_iter()
            .skip(1)
            .zip(input_tx_nums)
            .map(|(tx, tx_input_nums)| {
                tx.inputs
                    .par_iter()
                    .zip(tx_input_nums)
                    .map(|(input, &input_tx_num)| {
                        self.fetch_slp_output(input.prev_out.out_idx, input_tx_num)
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()
    }

    fn fetch_slp_output(
        &self,
        out_idx: u32,
        input_tx_num: TxNum,
    ) -> Result<Option<SlpSpentOutput>> {
        let slp_tx_data = self
            .db
            .get(self.cf_slp_tx_data(), TxNumZC::new(input_tx_num).as_bytes())?;
        let slp_tx_entry = match slp_tx_data {
            Some(slp_tx_data) => bincode::deserialize::<SerSlpTxEntry>(&slp_tx_data)?,
            None => return Ok(None),
        };
        let token_id = match slp_tx_entry.token_num {
            Some(token_num) => get_token_id_by_token_num(self.db, token_num)?,
            None => TokenId::new(Sha256d::new([0; 32])),
        };
        let group_token_id = match slp_tx_entry.group_token_num {
            Some(group_token_num) => Some(get_token_id_by_token_num(self.db, group_token_num)?),
            None => None,
        };
        let ser_token = slp_tx_entry
            .output_tokens
            .get(out_idx as usize)
            .cloned()
            .unwrap_or_default();
        Ok(Some(SlpSpentOutput {
            token_id,
            token_type: slp_tx_entry.slp_token_type,
            token: ser_token.to_token(),
            group_token_id: group_token_id.map(Box::new),
        }))
    }

    fn build_batch_txs<'b>(
        &self,
        parsed_slp_txs: Vec<(usize, SlpParseData)>,
        txs: &'b [UnhashedTx],
        input_tx_nums: &[Vec<TxNum>],
    ) -> Vec<(usize, BatchSlpTx<'b>)> {
        parsed_slp_txs
            .into_par_iter()
            .map(|(tx_idx, parsed_tx_data)| {
                let tx = &txs[tx_idx];
                (
                    tx_idx,
                    BatchSlpTx {
                        tx,
                        parsed_tx_data,
                        input_tx_nums: match tx_idx.checked_sub(1) {
                            Some(input_tx_num_idx) => input_tx_nums[input_tx_num_idx]
                                .iter()
                                .map(|&input_tx_num| Some(input_tx_num))
                                .collect::<Vec<_>>(),
                            None => vec![None],
                        },
                    },
                )
            })
            .collect()
    }

    fn build_known_slp_outputs(
        &self,
        batch_txs: &[(usize, BatchSlpTx)],
        spent_slp_outputs: &[Vec<Option<SlpSpentOutput>>],
    ) -> HashMap<OutpointEntry, Option<SlpSpentOutput>> {
        batch_txs
            .par_iter()
            .flat_map(|&(tx_idx, ref batch_tx)| {
                batch_tx
                    .tx
                    .inputs
                    .par_iter()
                    .zip(&batch_tx.input_tx_nums)
                    .enumerate()
                    .filter_map(move |(input_idx, (input, &input_tx_num))| {
                        let input_tx_num = input_tx_num?;
                        let spent_outputs = &spent_slp_outputs[tx_idx.checked_sub(1)?];
                        let spent_output = spent_outputs[input_idx].clone()?;
                        let outpoint = OutpointEntry {
                            tx_num: input_tx_num,
                            out_idx: input.prev_out.out_idx,
                        };
                        Some((outpoint, Some(spent_output)))
                    })
            })
            .collect()
    }

    fn insert_new_tokens<'b>(
        &self,
        batch: &mut WriteBatch,
        valid_tx_data: impl Iterator<Item = &'b SlpValidTxData>,
    ) -> Result<HashMap<[u8; 32], TokenNum>> {
        let mut next_token_num = self.get_next_token_num()?;
        let mut token_num_by_id = HashMap::new();
        for slp_tx in valid_tx_data {
            let slp_tx_data = &slp_tx.slp_tx_data;
            if let SlpTxType::Genesis(genesis) = &slp_tx_data.slp_tx_type {
                let ser_genesis_info = bincode::serialize(&genesis)?;
                let token_num = TokenNumZC::new(next_token_num);
                batch.put_cf(
                    self.cf_slp_token_metadata(),
                    token_num.as_bytes(),
                    &ser_genesis_info,
                );
                batch.put_cf(
                    self.cf_slp_token_id_by_num(),
                    token_num.as_bytes(),
                    &slp_tx_data.token_id.as_slice_be(),
                );
                batch.put_cf(
                    self.cf_slp_token_num_by_id(),
                    &slp_tx_data.token_id.as_slice_be(),
                    token_num.as_bytes(),
                );
                token_num_by_id.insert(slp_tx_data.token_id.token_id_be(), next_token_num);
                next_token_num += 1;
            }
        }
        Ok(token_num_by_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn update_token_stats(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: TxNum,
        txs: &[UnhashedTx],
        valid_txs: &SlpValidHashMap,
        input_tx_nums: &[Vec<TxNum>],
        spent_slp_outputs: &[Vec<Option<SlpSpentOutput>>],
        token_num_by_id: &mut HashMap<[u8; 32], TokenNum>,
        op: impl Fn(i128, i128) -> i128,
    ) -> Result<()> {
        let mut minted = HashMap::new();
        let mut burned = HashMap::new();
        for (tx_idx, tx) in txs.iter().enumerate() {
            let tx_num = first_tx_num + tx_idx as TxNum;
            let slp_token_inputs = match tx_idx {
                0 => vec![None],
                _ => tx
                    .inputs
                    .iter()
                    .zip(&input_tx_nums[tx_idx - 1])
                    .zip(&spent_slp_outputs[tx_idx - 1])
                    .map(
                        |((input, &input_tx_num), spent_slp_output)| match spent_slp_output {
                            Some(spent_slp_output) => Some(SlpInputToken {
                                token_id: &spent_slp_output.token_id,
                                token: &spent_slp_output.token,
                            }),
                            None => valid_txs.get(&input_tx_num).and_then(|slp| {
                                Some(SlpInputToken {
                                    token_id: &slp.slp_tx_data.token_id,
                                    token: slp
                                        .slp_tx_data
                                        .output_tokens
                                        .get(input.prev_out.out_idx as usize)?,
                                })
                            }),
                        },
                    )
                    .collect::<Vec<_>>(),
            };
            let valid_slp_tx = valid_txs.get(&tx_num);
            self.calc_token_supply_delta(&mut minted, &mut burned, &slp_token_inputs, valid_slp_tx);
        }
        let stats_token_ids = burned.keys().chain(minted.keys()).collect::<HashSet<_>>();
        for token_id in stats_token_ids {
            let token_id = TokenId::from_slice_be(token_id)?;
            let token_num = self
                .get_token_num_by_token_id(token_num_by_id, &token_id)?
                .ok_or_else(|| InconsistentTokenNumById(token_id.clone()))?;
            let token_num_zc = TokenNumZC::new(token_num);
            let token_stats_data = self
                .db
                .get(self.cf_slp_token_stats(), token_num_zc.as_bytes())?;
            let mut token_stats_data = match token_stats_data {
                Some(token_stats_data) => interpret::<TokenStatsData>(&token_stats_data)?.clone(),
                None => TokenStatsData::default(),
            };
            if let Some(&mint_amount) = minted.get(token_id.as_slice_be()) {
                let new_total_minted = op(token_stats_data.total_minted.get(), mint_amount);
                token_stats_data.total_minted = new_total_minted.into();
            }
            if let Some(&burn_amount) = burned.get(token_id.as_slice_be()) {
                let new_total_burned = op(token_stats_data.total_burned.get(), burn_amount);
                token_stats_data.total_burned = new_total_burned.into();
            }
            batch.put_cf(
                self.cf_slp_token_stats(),
                token_num_zc.as_bytes(),
                token_stats_data.as_bytes(),
            );
        }
        Ok(())
    }

    fn calc_token_supply_delta(
        &self,
        minted: &mut HashMap<[u8; 32], i128>,
        burned: &mut HashMap<[u8; 32], i128>,
        slp_token_inputs: &[Option<SlpInputToken<'_>>],
        valid_slp_tx: Option<&SlpValidTxData>,
    ) {
        let null_token = TokenId::new(Sha256d::new([0; 32]));
        match valid_slp_tx {
            // SEND already has the burns calculated
            Some(valid_slp_tx) if valid_slp_tx.slp_tx_data.slp_tx_type == SlpTxType::Send => {
                for burn in valid_slp_tx.slp_burns.iter().flatten() {
                    let burned_amount = burned.entry(burn.token_id.token_id_be()).or_default();
                    *burned_amount += burn.token.amount.base_amount();
                }
                return; // SEND never mints
            }
            // Everything except SEND burns all inputs
            // Note: We consider the required NFT1Parent input for NFT1Child a burn here
            _ => {
                for spent_output in slp_token_inputs.iter().flatten() {
                    if spent_output.token.amount == SlpAmount::ZERO
                        || spent_output.token_id == &null_token
                    {
                        continue;
                    }
                    let burned_amount = burned
                        .entry(spent_output.token_id.token_id_be())
                        .or_default();
                    *burned_amount += spent_output.token.amount.base_amount();
                }
            }
        };
        let valid_slp_tx = match valid_slp_tx {
            Some(valid_slp_tx) => valid_slp_tx,
            None => return,
        };
        let slp_tx_data = &valid_slp_tx.slp_tx_data;
        // GENESIS and MINT can mint
        if let SlpTxType::Genesis(_) | SlpTxType::Mint = &slp_tx_data.slp_tx_type {
            for token in &slp_tx_data.output_tokens {
                let minted_amount = minted
                    .entry(slp_tx_data.token_id.token_id_be())
                    .or_default();
                *minted_amount += token.amount.base_amount();
            }
        }
    }

    fn insert_new_valid_txs<'b>(
        &self,
        batch: &mut WriteBatch,
        valid_slp_txs: impl Iterator<Item = (&'b TxNum, &'b SlpValidTxData)>,
        token_num_by_id: &mut HashMap<[u8; 32], TokenNum>,
    ) -> Result<()> {
        for (&tx_num, slp_tx) in valid_slp_txs {
            let slp_tx_data = &slp_tx.slp_tx_data;
            let slp_tx_type = match slp_tx_data.slp_tx_type {
                SlpTxType::Genesis(_) => SerSlpTxType::Genesis,
                SlpTxType::Mint => SerSlpTxType::Mint,
                SlpTxType::Send => SerSlpTxType::Send,
                SlpTxType::Burn(_) => SerSlpTxType::Burn,
                SlpTxType::Unknown => SerSlpTxType::Unknown,
            };
            let token_num = match slp_tx_data.slp_token_type {
                SlpTokenType::Unknown => None,
                _ => Some(
                    self.get_token_num_by_token_id(token_num_by_id, &slp_tx_data.token_id)?
                        .ok_or_else(|| {
                            InconsistentDbNoSuchTokenId(tx_num, slp_tx_data.token_id.clone())
                        })?,
                ),
            };
            let group_token_num = match &slp_tx_data.group_token_id {
                Some(group_token_id) => Some(
                    self.get_token_num_by_token_id(token_num_by_id, group_token_id)?
                        .ok_or(InconsistentDbNullTokenGroupId(tx_num))?,
                ),
                None => None,
            };
            let mut slp_burns = Vec::with_capacity(slp_tx.slp_burns.len());
            for slp_burn in &slp_tx.slp_burns {
                match slp_burn {
                    Some(slp_burn) => {
                        slp_burns.push(Some(SerSlpBurn {
                            token_id_num: self
                                .get_token_num_by_token_id(token_num_by_id, &slp_burn.token_id)?,
                            token: SerSlpToken::from_token(&slp_burn.token),
                        }));
                    }
                    None => slp_burns.push(None),
                }
            }
            let ser_entry = SerSlpTxEntry {
                input_tokens: slp_tx_data
                    .input_tokens
                    .iter()
                    .map(SerSlpToken::from_token)
                    .collect(),
                output_tokens: slp_tx_data
                    .output_tokens
                    .iter()
                    .map(SerSlpToken::from_token)
                    .collect(),
                slp_token_type: slp_tx_data.slp_token_type,
                slp_tx_type,
                token_num,
                group_token_num,
                slp_burns,
            };
            let ser_entry = bincode::serialize(&ser_entry)?;
            batch.put_cf(
                self.cf_slp_tx_data(),
                TxNumZC::new(tx_num).as_bytes(),
                &ser_entry,
            );
        }
        Ok(())
    }

    fn insert_new_invalid_txs(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: TxNum,
        invalid_parsed_slp_txs: impl IntoIterator<Item = (usize, SlpError)>,
        invalid_slp_txs: impl IntoIterator<Item = (TxNum, SlpInvalidTxData)>,
    ) {
        let mut insert = |tx_num: TxNum, slp_error: &SlpError| {
            batch.put_cf(
                self.cf_slp_tx_invalid_message(),
                TxNumZC::new(tx_num).as_bytes(),
                slp_error.to_string().as_bytes(),
            );
        };
        for (tx_idx, slp_error) in invalid_parsed_slp_txs {
            let tx_num = first_tx_num + tx_idx as TxNum;
            insert(tx_num, &slp_error);
        }
        for (tx_num, invalid_tx_data) in invalid_slp_txs {
            insert(tx_num, &invalid_tx_data.slp_error);
        }
    }

    pub fn delete_block_txs<'b>(
        &self,
        batch: &mut WriteBatch,
        first_tx_num: TxNum,
        txs: &[UnhashedTx],
        txid_fn: impl Fn(usize) -> &'b Sha256d + Send + Sync,
        input_tx_nums: &[Vec<TxNum>],
    ) -> Result<()> {
        let (delete_valid_txs, delete_invalid_txs): (Vec<Result<(_, _)>>, Vec<TxNum>) = txs
            .par_iter()
            .enumerate()
            .filter_map(|(tx_idx, tx)| -> Option<_> {
                let tx_num = first_tx_num + tx_idx as TxNum;
                let txid = txid_fn(tx_idx);
                match parse_slp_tx(txid, tx) {
                    Ok(slp_parse_data) => Some(Either::Left(
                        self.fetch_delete_data(tx_num, &slp_parse_data),
                    )),
                    Err(err) => match is_ignored_error(&err) {
                        true => None,
                        false => Some(Either::Right(tx_num)),
                    },
                }
            })
            .partition_map(|either| either);
        // Fetch the SLP state of all inputs
        let spent_slp_outputs = self.fetch_spent_slp_outputs(txs, input_tx_nums)?;
        let delete_valid_txs = delete_valid_txs.into_iter().collect::<Result<Vec<_>>>()?;
        let mut valid_slp_txs = HashMap::with_capacity(delete_valid_txs.len());
        let mut token_num_by_id = HashMap::new();
        for (tx_num, delete_token) in delete_valid_txs {
            let tx_num_zc = TxNumZC::new(tx_num);
            batch.delete_cf(self.cf_slp_tx_data(), tx_num_zc.as_bytes());
            batch.delete_cf(self.cf_slp_tx_invalid_message(), tx_num_zc.as_bytes());
            if let Some((delete_token_num, delete_slp)) = delete_token {
                if matches!(delete_slp.slp_tx_data.slp_tx_type, SlpTxType::Genesis(_)) {
                    let delete_token_num_zc = TokenNumZC::new(delete_token_num);
                    batch.delete_cf(
                        self.cf_slp_token_id_by_num(),
                        delete_token_num_zc.as_bytes(),
                    );
                    batch.delete_cf(self.cf_slp_token_metadata(), delete_token_num_zc.as_bytes());
                    batch.delete_cf(
                        self.cf_slp_token_num_by_id(),
                        delete_slp.slp_tx_data.token_id.as_slice_be(),
                    );
                }
                token_num_by_id
                    .entry(delete_slp.slp_tx_data.token_id.token_id_be())
                    .or_insert(delete_token_num);
                valid_slp_txs.insert(tx_num, delete_slp);
            }
        }
        self.update_token_stats(
            batch,
            first_tx_num,
            txs,
            &valid_slp_txs,
            input_tx_nums,
            &spent_slp_outputs,
            &mut token_num_by_id,
            |a, b| a - b,
        )?;
        for tx_num in delete_invalid_txs {
            let tx_num = TxNumZC::new(tx_num);
            batch.delete_cf(self.cf_slp_tx_invalid_message(), tx_num.as_bytes());
        }
        Ok(())
    }

    fn fetch_delete_data(
        &self,
        tx_num: TxNum,
        slp_parse_data: &SlpParseData,
    ) -> Result<(TxNum, Option<(TokenNum, SlpValidTxData)>)> {
        let slp_reader = SlpReader::new(self.db)?;
        if let Some(token_num) = slp_reader.token_num_by_id(&slp_parse_data.token_id)? {
            if let Some(slp) = slp_reader.slp_data_by_tx_num(tx_num)? {
                return Ok((tx_num, Some((token_num, slp))));
            }
        }
        Ok((tx_num, None))
    }

    fn get_next_token_num(&self) -> Result<TokenNum> {
        let mut iterator = self
            .db
            .rocks()
            .iterator_cf(self.cf_slp_token_id_by_num(), IteratorMode::End);
        match iterator.next() {
            Some((key, _)) => Ok(interpret::<TokenNumZC>(&key)?.get() + 1),
            None => Ok(0),
        }
    }

    fn get_token_num_by_token_id(
        &self,
        token_num_by_id: &mut HashMap<[u8; 32], TokenNum>,
        token_id: &TokenId,
    ) -> Result<Option<TokenNum>> {
        let token_id_be = token_id.token_id_be();
        if token_id_be == [0; 32] {
            return Ok(None);
        }
        match token_num_by_id.get(&token_id_be) {
            Some(&token_num) => Ok(Some(token_num)),
            None => {
                let token_num = self
                    .db
                    .get(self.cf_slp_token_num_by_id(), token_id_be)?
                    .ok_or_else(|| InconsistentDbTokenNumById(token_id.clone()))?;
                let token_num = interpret::<TokenNumZC>(&token_num)?.get();
                token_num_by_id.insert(token_id_be, token_num);
                Ok(Some(token_num))
            }
        }
    }

    fn cf_slp_token_id_by_num(&self) -> &CF {
        self.db.cf(CF_SLP_TOKEN_ID_BY_NUM).unwrap()
    }

    fn cf_slp_token_num_by_id(&self) -> &CF {
        self.db.cf(CF_SLP_TOKEN_NUM_BY_ID).unwrap()
    }

    fn cf_slp_token_metadata(&self) -> &CF {
        self.db.cf(CF_SLP_TOKEN_METADATA).unwrap()
    }

    fn cf_slp_tx_data(&self) -> &CF {
        self.db.cf(CF_SLP_TX_DATA).unwrap()
    }

    fn cf_slp_tx_invalid_message(&self) -> &CF {
        self.db.cf(CF_SLP_TX_INVALID_MESSAGE).unwrap()
    }

    fn cf_slp_token_stats(&self) -> &CF {
        self.db.cf(CF_SLP_TOKEN_STATS).unwrap()
    }
}

impl<'a> SlpReader<'a> {
    pub fn new(db: &'a Db) -> Result<Self> {
        let _ = db.cf(CF_SLP_TOKEN_METADATA)?;
        let _ = db.cf(CF_SLP_TOKEN_NUM_BY_ID)?;
        let _ = db.cf(CF_SLP_TX_DATA)?;
        let _ = db.cf(CF_SLP_TX_INVALID_MESSAGE)?;
        Ok(SlpReader { db })
    }

    pub fn token_by_token_num(&self, token_num: TokenNum) -> Result<Option<SlpGenesisInfo>> {
        let token_num = TokenNumZC::new(token_num);
        match self
            .db
            .get(self.cf_slp_token_metadata(), token_num.as_bytes())?
        {
            Some(slp_genesis_info) => Ok(Some(bincode::deserialize(&slp_genesis_info)?)),
            None => Ok(None),
        }
    }

    pub fn token_num_by_id(&self, token_id: &TokenId) -> Result<Option<TokenNum>> {
        let token_id_be = token_id.token_id_be();
        let token_num = match self.db.get(self.cf_slp_token_num_by_id(), token_id_be)? {
            Some(token_num) => token_num,
            None => return Ok(None),
        };
        let token_num = interpret::<TokenNumZC>(&token_num)?.get();
        Ok(Some(token_num))
    }

    pub fn slp_data_by_tx_num(&self, tx_num: TxNum) -> Result<Option<SlpValidTxData>> {
        let tx_num = TxNumZC::new(tx_num);
        let slp_tx_data = match self.db.get(self.cf_slp_tx_data(), tx_num.as_bytes())? {
            Some(slp_tx_data) => bincode::deserialize::<SerSlpTxEntry>(&slp_tx_data)?,
            None => return Ok(None),
        };
        let slp_burns = slp_tx_data
            .slp_burns
            .iter()
            .map(|burn| {
                burn.as_ref()
                    .map(|burn| {
                        Ok(Box::new(SlpBurn {
                            token: burn.token.to_token(),
                            token_id: match burn.token_id_num {
                                Some(token_num) => get_token_id_by_token_num(self.db, token_num)?,
                                None => TokenId::new(Sha256d::new([0; 32])),
                            },
                        }))
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>>>()?;
        let input_tokens = slp_tx_data
            .input_tokens
            .iter()
            .map(SerSlpToken::to_token)
            .collect::<Vec<_>>();
        let slp_tx_data = SlpTxData {
            input_tokens: input_tokens.clone(),
            output_tokens: slp_tx_data
                .output_tokens
                .iter()
                .map(SerSlpToken::to_token)
                .collect(),
            slp_token_type: slp_tx_data.slp_token_type,
            slp_tx_type: match slp_tx_data.slp_tx_type {
                SerSlpTxType::Genesis => {
                    let token_num = slp_tx_data
                        .token_num
                        .ok_or_else(|| InconsistentDbNullTokenGenesis(tx_num.get()))?;
                    let slp_genesis_info = self
                        .token_by_token_num(token_num)?
                        .ok_or_else(|| InconsistentDbNoSuchTokenNum(tx_num.get(), token_num))?;
                    SlpTxType::Genesis(Box::new(slp_genesis_info))
                }
                SerSlpTxType::Mint => SlpTxType::Mint,
                SerSlpTxType::Send => SlpTxType::Send,
                SerSlpTxType::Burn => {
                    let burn_amount = input_tokens
                        .iter()
                        .map(|token| token.amount)
                        .sum::<SlpAmount>();
                    SlpTxType::Burn(burn_amount.base_amount().try_into().unwrap())
                }
                SerSlpTxType::Unknown => SlpTxType::Unknown,
            },
            token_id: match slp_tx_data.token_num {
                Some(token_num) => get_token_id_by_token_num(self.db, token_num)?,
                None => TokenId::new(Sha256d::new([0; 32])),
            },
            group_token_id: slp_tx_data
                .group_token_num
                .map(|group_token_num| -> Result<_> {
                    Ok(Box::new(get_token_id_by_token_num(
                        self.db,
                        group_token_num,
                    )?))
                })
                .transpose()?,
        };
        Ok(Some(SlpValidTxData {
            slp_tx_data,
            slp_burns,
        }))
    }

    pub fn slp_invalid_message_tx_num(&self, tx_num: TxNum) -> Result<Option<String>> {
        let tx_num = TxNumZC::new(tx_num);
        match self
            .db
            .get(self.cf_slp_tx_invalid_message(), tx_num.as_bytes())?
        {
            Some(message) => Ok(Some(std::str::from_utf8(&message)?.to_string())),
            None => Ok(None),
        }
    }

    pub fn token_stats_by_token_num(&self, token_num: TokenNum) -> Result<Option<TokenStats>> {
        let token_stats_data = self.db.get(
            self.cf_slp_token_stats(),
            TokenNumZC::new(token_num).as_bytes(),
        )?;
        let token_stats_data = match &token_stats_data {
            Some(token_stats_data) => interpret::<TokenStatsData>(token_stats_data)?,
            None => return Ok(None),
        };
        Ok(Some(TokenStats {
            total_burned: token_stats_data.total_burned.get(),
            total_minted: token_stats_data.total_minted.get(),
        }))
    }

    fn cf_slp_token_num_by_id(&self) -> &CF {
        self.db.cf(CF_SLP_TOKEN_NUM_BY_ID).unwrap()
    }

    fn cf_slp_token_metadata(&self) -> &CF {
        self.db.cf(CF_SLP_TOKEN_METADATA).unwrap()
    }

    fn cf_slp_tx_data(&self) -> &CF {
        self.db.cf(CF_SLP_TX_DATA).unwrap()
    }

    fn cf_slp_tx_invalid_message(&self) -> &CF {
        self.db.cf(CF_SLP_TX_INVALID_MESSAGE).unwrap()
    }

    fn cf_slp_token_stats(&self) -> &CF {
        self.db.cf(CF_SLP_TOKEN_STATS).unwrap()
    }
}

impl Default for SerSlpToken {
    fn default() -> Self {
        SerSlpToken::Amount(0)
    }
}

impl SerSlpToken {
    fn to_token(&self) -> SlpToken {
        match self {
            &SerSlpToken::Amount(amount) => SlpToken::amount(amount.into()),
            SerSlpToken::MintBaton => SlpToken::MINT_BATON,
        }
    }

    fn from_token(token: &SlpToken) -> Self {
        match token.is_mint_baton {
            true => SerSlpToken::MintBaton,
            false => SerSlpToken::Amount(token.amount.base_amount().try_into().unwrap()),
        }
    }
}

fn get_token_id_by_token_num(db: &Db, token_num: TokenNum) -> Result<TokenId> {
    let token_id = db
        .get(
            db.cf(CF_SLP_TOKEN_ID_BY_NUM)?,
            TokenNumZC::new(token_num).as_bytes(),
        )?
        .ok_or(InconsistentDbTokenIdByNum(token_num))?;
    let token_id = TokenId::from_slice_be(&token_id)?;
    Ok(token_id)
}

/// Ignore txs which don't look like SLP at all
pub fn is_ignored_error(slp_error: &SlpError) -> bool {
    matches!(
        slp_error,
        SlpError::NoOpcodes
            | SlpError::MissingOpReturn { .. }
            | SlpError::InvalidLokadId { .. }
            | SlpError::BytesError { .. }
    )
}

#[cfg(test)]
mod tests {
    use bitcoinsuite_core::{Hashed, OutPoint, Script, Sha256d, TxInput, TxOutput, UnhashedTx};
    use bitcoinsuite_error::Result;
    use bitcoinsuite_slp::{
        genesis_opreturn, mint_opreturn, send_opreturn, SlpAmount, SlpBurn, SlpError,
        SlpGenesisInfo, SlpToken, SlpTokenType, SlpTxData, SlpTxType, TokenId,
    };
    use pretty_assertions::assert_eq;
    use rocksdb::WriteBatch;

    use crate::{
        input_tx_nums::fetch_input_tx_nums, BlockHeight, BlockTxs, Db, SlpReader, SlpWriter,
        TokenStats, TxEntry, TxNum, TxWriter,
    };

    enum Outcome {
        NotSlp,
        Valid(SlpTxData),
        ValidBurn(SlpTxData, Vec<Option<Box<SlpBurn>>>),
        Invalid(SlpError),
    }

    #[test]
    fn test_slp_writer() -> Result<()> {
        let blocks = [
            make_block(
                [
                    make_tx(
                        (1, [(0, 0xffff_ffff)], 3),
                        Script::opreturn(&[&[0; 100]]),
                        Outcome::NotSlp,
                    ),
                    // GENESIS: mint fungible token
                    make_tx(
                        (2, [(1, 1)], 3),
                        genesis_opreturn(
                            &SlpGenesisInfo::default(),
                            SlpTokenType::Fungible,
                            Some(2),
                            10,
                        ),
                        Outcome::Valid(SlpTxData {
                            input_tokens: vec![SlpToken::EMPTY],
                            output_tokens: vec![
                                SlpToken::EMPTY,
                                SlpToken::amount(10),
                                SlpToken::MINT_BATON,
                            ],
                            slp_token_type: SlpTokenType::Fungible,
                            slp_tx_type: SlpTxType::Genesis(SlpGenesisInfo::default().into()),
                            token_id: TokenId::new(make_hash(2)),
                            group_token_id: None,
                        }),
                    ),
                    // MINT fungible tokens
                    make_tx(
                        (3, [(2, 2)], 4),
                        mint_opreturn(
                            &TokenId::new(make_hash(2)),
                            SlpTokenType::Fungible,
                            Some(3),
                            4,
                        ),
                        Outcome::Valid(SlpTxData {
                            input_tokens: vec![SlpToken::MINT_BATON],
                            output_tokens: vec![
                                SlpToken::EMPTY,
                                SlpToken::amount(4),
                                SlpToken::EMPTY,
                                SlpToken::MINT_BATON,
                            ],
                            slp_token_type: SlpTokenType::Fungible,
                            slp_tx_type: SlpTxType::Mint,
                            token_id: TokenId::new(make_hash(2)),
                            group_token_id: None,
                        }),
                    ),
                    // SEND fungible token
                    make_tx(
                        (4, [(2, 1), (3, 1)], 3),
                        send_opreturn(
                            &TokenId::new(make_hash(2)),
                            SlpTokenType::Fungible,
                            &[SlpAmount::new(11), SlpAmount::new(3)],
                        ),
                        Outcome::Valid(SlpTxData {
                            input_tokens: vec![SlpToken::amount(10), SlpToken::amount(4)],
                            output_tokens: vec![
                                SlpToken::EMPTY,
                                SlpToken::amount(11),
                                SlpToken::amount(3),
                            ],
                            slp_token_type: SlpTokenType::Fungible,
                            slp_tx_type: SlpTxType::Send,
                            token_id: TokenId::new(make_hash(2)),
                            group_token_id: None,
                        }),
                    ),
                ],
                [(2, (14, 0))],
            ),
            make_block(
                [
                    make_tx(
                        (11, [(0, 0xffff_ffff)], 7),
                        Script::opreturn(&[&[0; 100]]),
                        Outcome::NotSlp,
                    ),
                    // GENESIS: mint NFT1 group token
                    make_tx(
                        (12, [(1, 2)], 3),
                        genesis_opreturn(
                            &SlpGenesisInfo::default(),
                            SlpTokenType::Nft1Group,
                            Some(2),
                            100,
                        ),
                        Outcome::Valid(SlpTxData {
                            input_tokens: vec![SlpToken::EMPTY],
                            output_tokens: vec![
                                SlpToken::EMPTY,
                                SlpToken::amount(100),
                                SlpToken::MINT_BATON,
                            ],
                            slp_token_type: SlpTokenType::Nft1Group,
                            slp_tx_type: SlpTxType::Genesis(SlpGenesisInfo::default().into()),
                            token_id: TokenId::new(make_hash(12)),
                            group_token_id: None,
                        }),
                    ),
                    // GENESIS: mint another fungible token
                    make_tx(
                        (13, [(11, 1)], 3),
                        genesis_opreturn(
                            &SlpGenesisInfo::default(),
                            SlpTokenType::Fungible,
                            None,
                            1000,
                        ),
                        Outcome::Valid(SlpTxData {
                            input_tokens: vec![SlpToken::EMPTY],
                            output_tokens: vec![
                                SlpToken::EMPTY,
                                SlpToken::amount(1000),
                                SlpToken::EMPTY,
                            ],
                            slp_token_type: SlpTokenType::Fungible,
                            slp_tx_type: SlpTxType::Genesis(SlpGenesisInfo::default().into()),
                            token_id: TokenId::new(make_hash(13)),
                            group_token_id: None,
                        }),
                    ),
                    // Invalid tx: SEND fungible token, but input amount < output amount
                    make_tx(
                        (14, [(4, 2), (11, 2)], 1),
                        send_opreturn(
                            &TokenId::new(make_hash(2)),
                            SlpTokenType::Fungible,
                            &[SlpAmount::new(2), SlpAmount::new(2)],
                        ),
                        Outcome::Invalid(SlpError::OutputSumExceedInputSum {
                            input_sum: SlpAmount::new(3),
                            output_sum: SlpAmount::new(4),
                        }),
                    ),
                    // SEND NFT1 group token to two outputs
                    make_tx(
                        (15, [(12, 1), (3, 3)], 2),
                        send_opreturn(
                            &TokenId::new(make_hash(12)),
                            SlpTokenType::Nft1Group,
                            &[SlpAmount::new(1), SlpAmount::new(50)],
                        ),
                        Outcome::ValidBurn(
                            SlpTxData {
                                input_tokens: vec![SlpToken::amount(100), SlpToken::EMPTY],
                                output_tokens: vec![
                                    SlpToken::EMPTY,
                                    SlpToken::amount(1),
                                    SlpToken::amount(50),
                                ],
                                slp_token_type: SlpTokenType::Nft1Group,
                                slp_tx_type: SlpTxType::Send,
                                token_id: TokenId::new(make_hash(12)),
                                group_token_id: None,
                            },
                            vec![
                                Some(Box::new(SlpBurn {
                                    token_id: TokenId::new(make_hash(12)),
                                    token: SlpToken::amount(49),
                                })),
                                Some(Box::new(SlpBurn {
                                    token_id: TokenId::new(make_hash(2)),
                                    token: SlpToken::MINT_BATON,
                                })),
                            ],
                        ),
                    ),
                    // GENESIS NFT1 child token based on NFT1 group token
                    make_tx(
                        (16, [(15, 1)], 3),
                        genesis_opreturn(
                            &SlpGenesisInfo::default(),
                            SlpTokenType::Nft1Child,
                            None,
                            1,
                        ),
                        Outcome::Valid(SlpTxData {
                            input_tokens: vec![SlpToken::amount(1)],
                            output_tokens: vec![
                                SlpToken::EMPTY,
                                SlpToken::amount(1),
                                SlpToken::EMPTY,
                            ],
                            slp_token_type: SlpTokenType::Nft1Child,
                            slp_tx_type: SlpTxType::Genesis(SlpGenesisInfo::default().into()),
                            token_id: TokenId::new(make_hash(16)),
                            group_token_id: Some(TokenId::new(make_hash(12)).into()),
                        }),
                    ),
                ],
                [
                    (2, (14, 3)),    // burns 3 fungible tokens
                    (12, (100, 50)), // burns 49 Nft1Group tokens, redeems 1
                    (13, (1000, 0)), // new fungible token
                    (16, (1, 0)),    // new Nft1Child token
                ],
            ),
            make_block(
                [
                    // GENESIS in coinbase also allowed
                    make_tx(
                        (21, [(0, 0xffff_ffff)], 2),
                        genesis_opreturn(
                            &SlpGenesisInfo::default(),
                            SlpTokenType::Fungible,
                            None,
                            1,
                        ),
                        Outcome::Valid(SlpTxData {
                            input_tokens: vec![SlpToken::EMPTY],
                            output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(1)],
                            slp_token_type: SlpTokenType::Fungible,
                            slp_tx_type: SlpTxType::Genesis(SlpGenesisInfo::default().into()),
                            token_id: TokenId::new(make_hash(21)),
                            group_token_id: None,
                        }),
                    ),
                    // GENESIS NFT1 child across blocks
                    make_tx(
                        (22, [(15, 2)], 2),
                        genesis_opreturn(
                            &SlpGenesisInfo::default(),
                            SlpTokenType::Nft1Child,
                            None,
                            1,
                        ),
                        Outcome::Valid(SlpTxData {
                            input_tokens: vec![SlpToken::amount(50)],
                            output_tokens: vec![SlpToken::EMPTY, SlpToken::amount(1)],
                            slp_token_type: SlpTokenType::Nft1Child,
                            slp_tx_type: SlpTxType::Genesis(SlpGenesisInfo::default().into()),
                            token_id: TokenId::new(make_hash(22)),
                            group_token_id: Some(TokenId::new(make_hash(12)).into()),
                        }),
                    ),
                    // Invalid SEND: Reversed token_id
                    make_tx(
                        (23, [(4, 1)], 3),
                        send_opreturn(
                            &TokenId::new({
                                let mut hash = make_hash(2).byte_array().array();
                                hash.reverse();
                                Sha256d::new(hash)
                            }),
                            SlpTokenType::Fungible,
                            &[SlpAmount::new(1)],
                        ),
                        Outcome::Invalid(SlpError::OutputSumExceedInputSum {
                            input_sum: SlpAmount::new(0),
                            output_sum: SlpAmount::new(1),
                        }),
                    ),
                ],
                [
                    (2, (14, 14)),
                    (12, (100, 100)), // burns remaining Nft1Group tokens
                    (13, (1000, 0)),
                    (16, (1, 0)),
                    (21, (1, 0)),
                    (22, (1, 0)),
                ],
            ),
            make_block(
                [
                    make_tx(
                        (31, [(0, 0xffff_ffff)], 5),
                        Script::opreturn(&[&[0; 100]]),
                        Outcome::NotSlp,
                    ),
                    // Not SLP: wrong LOKAD ID
                    make_tx(
                        (32, [(11, 3)], 2),
                        Script::opreturn(&[b"SLP", b"\x01"]),
                        Outcome::NotSlp,
                    ),
                    // Invalid SLP: too few pushops
                    make_tx(
                        (33, [(11, 4)], 2),
                        Script::opreturn(&[b"SLP\0", b"\x01"]),
                        Outcome::Invalid(SlpError::TooFewPushes {
                            actual: 2,
                            expected: 3,
                        }),
                    ),
                    // Invalid SLP tx: invalid tx type "INVALID"
                    make_tx(
                        (34, [(11, 5)], 2),
                        Script::opreturn(&[b"SLP\0", b"\x01", b"INVALID"]),
                        Outcome::Invalid(SlpError::InvalidTxType(b"INVALID".as_ref().into())),
                    ),
                    // Valid SLP tx but unknown token type 0xff
                    make_tx(
                        (35, [(11, 6)], 2),
                        Script::opreturn(&[b"SLP\0", b"\xff", b"INCOGNITO"]),
                        Outcome::Valid(SlpTxData {
                            input_tokens: vec![SlpToken::EMPTY],
                            output_tokens: vec![SlpToken::EMPTY, SlpToken::EMPTY],
                            slp_token_type: SlpTokenType::Unknown,
                            slp_tx_type: SlpTxType::Unknown,
                            token_id: TokenId::new(Sha256d::new([0; 32])),
                            group_token_id: None,
                        }),
                    ),
                    // Valid SLP tx but unknown token type 0xff, spending unknown token type
                    make_tx(
                        (36, [(35, 1)], 2),
                        Script::opreturn(&[b"SLP\0", b"\xff", b"INCOGNITO"]),
                        Outcome::ValidBurn(
                            SlpTxData {
                                input_tokens: vec![SlpToken::EMPTY],
                                output_tokens: vec![SlpToken::EMPTY, SlpToken::EMPTY],
                                slp_token_type: SlpTokenType::Unknown,
                                slp_tx_type: SlpTxType::Unknown,
                                token_id: TokenId::new(Sha256d::new([0; 32])),
                                group_token_id: None,
                            },
                            vec![Some(Box::new(SlpBurn {
                                token: SlpToken::EMPTY,
                                token_id: TokenId::new(Sha256d::new([0; 32])),
                            }))],
                        ),
                    ),
                ],
                [
                    (2, (14, 14)),
                    (12, (100, 100)),
                    (13, (1000, 0)),
                    (16, (1, 0)),
                    (21, (1, 0)),
                    (22, (1, 0)),
                ],
            ),
            make_block(
                [
                    make_tx(
                        (41, [(0, 0xffff_ffff)], 1),
                        Script::opreturn(&[&[0; 100]]),
                        Outcome::NotSlp,
                    ),
                    make_tx(
                        (42, [(13, 1)], 3),
                        Script::opreturn(&[&[0; 100]]),
                        Outcome::NotSlp,
                    ),
                ],
                [
                    (2, (14, 14)),
                    (12, (100, 100)),
                    (13, (1000, 1000)), // burns fungible tokens
                    (16, (1, 0)),
                    (21, (1, 0)),
                    (22, (1, 0)),
                ],
            ),
        ];
        bitcoinsuite_error::install()?;
        let tempdir = tempdir::TempDir::new("slp-indexer-rocks--utxos")?;
        let db = Db::open(tempdir.path())?;
        let tx_writer = TxWriter::new(&db)?;
        let slp_writer = SlpWriter::new(&db)?;
        let slp_reader = SlpReader::new(&db)?;
        let mut first_tx_num = 0;
        let mut previous_token_stats: Option<Vec<(TokenId, TokenStats)>> = None;
        for (block_height, (txids, txs, outcomes, token_stats)) in blocks.into_iter().enumerate() {
            let mut batch = WriteBatch::default();
            let input_tx_nums = fetch_input_tx_nums(&db, first_tx_num, |idx| &txids[idx], &txs)?;
            slp_writer.insert_block_txs(
                &mut batch,
                first_tx_num,
                &txs,
                |idx| &txids[idx],
                &input_tx_nums,
            )?;
            let block_txs = txids
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
            // Validate SLP txs and insert
            tx_writer.insert_block_txs(
                &mut batch,
                &BlockTxs {
                    txs: block_txs,
                    block_height: block_height as BlockHeight,
                },
            )?;
            db.write_batch(batch)?;
            for (tx_idx, outcome) in outcomes.iter().enumerate() {
                let tx_num = first_tx_num + tx_idx as TxNum;
                let txid = &txids[tx_idx];
                let result = slp_reader.slp_data_by_tx_num(tx_num)?;
                let message = slp_reader.slp_invalid_message_tx_num(tx_num)?;
                if let Outcome::Valid(slp_data) | Outcome::ValidBurn(slp_data, _) = outcome {
                    if let SlpTxType::Genesis(genesis) = &slp_data.slp_tx_type {
                        let token_num = slp_reader
                            .token_num_by_id(&TokenId::new(txid.clone()))?
                            .unwrap_or_else(|| panic!("No token num for {}", txid));
                        let token_data = slp_reader
                            .token_by_token_num(token_num)?
                            .unwrap_or_else(|| panic!("No token data for {}", txid));
                        assert_eq!(
                            genesis.as_ref(),
                            &token_data,
                            "Mismatch genesis data for token ID {}",
                            txid
                        );
                    }
                }
                match outcome {
                    Outcome::NotSlp => {
                        assert_eq!(result, None, "Expected no SLP for txid {}", txid);
                        assert_eq!(message, None, "Expected no error for txid {}", txid);
                    }
                    Outcome::Invalid(expected_slp_error) => {
                        assert_eq!(result, None, "Expected no SLP for txid {}", txid);
                        assert_eq!(
                            message,
                            Some(expected_slp_error.to_string()),
                            "Expected error message for txid {}",
                            txid
                        );
                    }
                    Outcome::Valid(expected_slp) => {
                        let actual_slp =
                            result.unwrap_or_else(|| panic!("Expected SLP for txid {}", txid));
                        assert_eq!(
                            &actual_slp.slp_tx_data, expected_slp,
                            "Expected equal SlpTxData for txid {}",
                            txid
                        );
                        assert_eq!(
                            actual_slp.slp_burns,
                            vec![None; actual_slp.slp_burns.len()],
                            "Expected no burns for txid {}",
                            txid
                        );
                        assert_eq!(message, None, "Expected no error for txid {}", txid);
                    }
                    Outcome::ValidBurn(expected_slp, expected_burns) => {
                        let actual_slp =
                            result.unwrap_or_else(|| panic!("Expected Some for txid {}", txid));
                        assert_eq!(
                            &actual_slp.slp_tx_data, expected_slp,
                            "Expected equal SlpTxData for txid {}",
                            txid
                        );
                        assert_eq!(
                            &actual_slp.slp_burns, expected_burns,
                            "Expected burns for txid {}",
                            txid
                        );
                        assert_eq!(message, None, "Expected no error for txid {}", txid);
                    }
                }
            }
            // Verify token stats
            for (token_id, expected_stats) in &token_stats {
                let token_num = slp_reader.token_num_by_id(token_id)?.unwrap();
                let actual_stats = slp_reader.token_stats_by_token_num(token_num)?;
                assert_eq!(
                    Some(expected_stats.clone()),
                    actual_stats,
                    "for token_num {}",
                    token_num
                );
            }
            // Delete block
            let mut batch = WriteBatch::default();
            slp_writer.delete_block_txs(
                &mut batch,
                first_tx_num,
                &txs,
                |idx| &txids[idx],
                &input_tx_nums,
            )?;
            db.write_batch(batch)?;
            for (tx_idx, outcome) in outcomes.iter().enumerate() {
                let tx_num = first_tx_num + tx_idx as TxNum;
                let txid = &txids[tx_idx];
                let result = slp_reader.slp_data_by_tx_num(tx_num)?;
                let message = slp_reader.slp_invalid_message_tx_num(tx_num)?;
                if let Outcome::Valid(slp_data) | Outcome::ValidBurn(slp_data, _) = outcome {
                    if let SlpTxType::Genesis(_) = &slp_data.slp_tx_type {
                        let token_num = slp_reader.token_num_by_id(&TokenId::new(txid.clone()))?;
                        assert_eq!(token_num, None, "Expected no token for txid {}", txid);
                    }
                }
                assert_eq!(result, None, "Expected no SLP for txid {}", txid);
                assert_eq!(message, None, "Expected no error for txid {}", txid);
            }
            if let Some(previous_token_stats) = &previous_token_stats {
                // Verify previous token stats
                for (token_id, expected_stats) in previous_token_stats {
                    let token_num = slp_reader.token_num_by_id(token_id)?.unwrap();
                    let actual_stats = slp_reader.token_stats_by_token_num(token_num)?;
                    assert_eq!(
                        Some(expected_stats.clone()),
                        actual_stats,
                        "for token_num {}",
                        token_num
                    );
                }
            }
            previous_token_stats = Some(token_stats);
            // Add block back in again before continuing
            let mut batch = WriteBatch::default();
            slp_writer.insert_block_txs(
                &mut batch,
                first_tx_num,
                &txs,
                |idx| &txids[idx],
                &input_tx_nums,
            )?;
            db.write_batch(batch)?;
            first_tx_num += txids.len() as TxNum;
        }
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    fn make_block<const N: usize, const M: usize>(
        txs: [(Sha256d, UnhashedTx, Outcome); N],
        token_stats: [(u8, (i128, i128)); M],
    ) -> (
        Vec<Sha256d>,
        Vec<UnhashedTx>,
        Vec<Outcome>,
        Vec<(TokenId, TokenStats)>,
    ) {
        let (txids, rest): (Vec<_>, Vec<_>) = txs
            .into_iter()
            .map(|(txid, tx, outcome)| (txid, (tx, outcome)))
            .unzip();
        let (txs, outcomes): (Vec<_>, Vec<_>) = rest.into_iter().unzip();
        let token_stats = token_stats
            .into_iter()
            .map(|(token_byte, (mint, burn))| {
                (
                    TokenId::new(make_hash(token_byte)),
                    TokenStats {
                        total_minted: mint,
                        total_burned: burn,
                    },
                )
            })
            .collect();
        (txids, txs, outcomes, token_stats)
    }

    fn make_tx<const N: usize>(
        shape: (u8, [(u8, u32); N], usize),
        slp_script: Script,
        outcome: Outcome,
    ) -> (Sha256d, UnhashedTx, Outcome) {
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
            outcome,
        )
    }

    fn make_hash(byte: u8) -> Sha256d {
        let mut hash = [0; 32];
        hash[31] = byte;
        Sha256d::new(hash)
    }
}

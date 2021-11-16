use std::collections::HashMap;

use bitcoinsuite_core::{OutPoint, Sha256d, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Result};
use rayon::iter::{IndexedParallelIterator, IntoParallelRefIterator, ParallelIterator};
use thiserror::Error;

use crate::{Db, TxNum, TxReader};

#[derive(Debug, Error, ErrorMeta)]
pub enum InputTxNumsError {
    #[critical()]
    #[error("Unknown input spent: {0:?}")]
    UnknownInputSpent(OutPoint),
}

use self::InputTxNumsError::*;

pub fn fetch_input_tx_nums<'b>(
    db: &Db,
    first_tx_num: TxNum,
    txids_fn: impl Fn(usize) -> &'b Sha256d,
    txs: &[UnhashedTx],
) -> Result<Vec<Vec<TxNum>>> {
    let mut tx_num = first_tx_num;
    let mut new_tx_nums = HashMap::new();
    for tx_idx in 0..txs.len() {
        let txid = txids_fn(tx_idx);
        new_tx_nums.insert(txid.clone(), tx_num);
        tx_num += 1;
    }
    let tx_reader = TxReader::new(db)?;
    // tx_nums for each spent input
    txs.par_iter()
        .skip(1)
        .map(|tx| {
            tx.inputs
                .iter()
                .map(|input| {
                    Ok(match new_tx_nums.get(&input.prev_out.txid) {
                        Some(&tx_num) => tx_num,
                        None => tx_reader
                            .tx_num_by_txid(&input.prev_out.txid)?
                            .ok_or_else(|| UnknownInputSpent(input.prev_out.clone()))?,
                    })
                })
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()
}

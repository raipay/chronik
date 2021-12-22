use bitcoinsuite_bitcoind::BitcoindError;
use bitcoinsuite_core::{BitcoinCode, Hashed, Sha256d, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Result};
use bitcoinsuite_slp::{SlpAmount, SlpBurn, SlpError, SlpToken};
use chronik_rocksdb::is_ignored_error;
use thiserror::Error;

pub struct Broadcast<'a> {
    indexer: &'a SlpIndexer,
}

#[derive(Debug, Error, ErrorMeta, PartialEq, Eq)]
pub enum BroadcastError {
    #[invalid_user_input()]
    #[error("Invalid SLP tx: {0}")]
    InvalidSlpTx(SlpError),

    #[invalid_user_input()]
    #[error("Invalid SLP burns: {0}")]
    InvalidSlpBurns(SlpBurns),

    #[invalid_user_input()]
    #[error("Bitcoind rejected tx: {0}")]
    BitcoindRejectedTx(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlpBurns(pub Vec<Option<Box<SlpBurn>>>);

use crate::SlpIndexer;

use self::BroadcastError::*;

impl<'a> Broadcast<'a> {
    pub fn new(indexer: &'a SlpIndexer) -> Self {
        Broadcast { indexer }
    }

    fn check_no_slp_burn(
        &self,
        tx: &UnhashedTx,
    ) -> Result<std::result::Result<(), BroadcastError>> {
        let dummy_txid = Sha256d::default();
        let result = self
            .indexer
            .db()
            .validate_slp_tx(&self.indexer.data, &dummy_txid, tx)?;
        match result {
            Ok(valid_tx_data) => {
                if valid_tx_data.slp_burns.iter().any(Option::is_some) {
                    return Ok(Err(InvalidSlpBurns(SlpBurns(valid_tx_data.slp_burns))));
                }
            }
            Err(slp_error) => {
                if !is_ignored_error(&slp_error) {
                    return Ok(Err(InvalidSlpTx(slp_error)));
                }
            }
        }
        Ok(Ok(()))
    }

    pub fn broadcast_tx(&self, tx: &UnhashedTx, check_slp: bool) -> Result<Sha256d> {
        if check_slp {
            self.check_no_slp_burn(tx)??;
        }
        let raw_tx = tx.ser();
        let result = self
            .indexer
            .bitcoind
            .cmd_string("sendrawtransaction", &[&raw_tx.hex()]);
        let report = match result {
            Ok(txid_hex) => return Ok(Sha256d::from_hex_be(&txid_hex)?),
            Err(report) => report,
        };
        match report.downcast::<BitcoindError>()? {
            BitcoindError::JsonRpc(rpc_message) => Err(BitcoindRejectedTx(rpc_message).into()),
            bitcoind_error => Err(bitcoind_error.into()),
        }
    }

    pub fn test_mempool_accept(
        &self,
        tx: &UnhashedTx,
        check_slp: bool,
    ) -> Result<std::result::Result<(), BroadcastError>> {
        if check_slp {
            let result = self.check_no_slp_burn(tx)?;
            if result.is_err() {
                return Ok(result);
            }
        }
        let raw_tx = tx.ser();
        let result = self
            .indexer
            .bitcoind
            .cmd_json("testmempoolaccept", &[&format!("[\"{}\"]", raw_tx.hex())]);
        match result {
            Ok(json_result) => {
                let tx_result = &json_result[0];
                if !tx_result["allowed"].as_bool().expect("No 'allowed' field") {
                    return Ok(Err(BroadcastError::BitcoindRejectedTx(
                        tx_result["reject-reason"]
                            .as_str()
                            .expect("No 'reject-reason' field")
                            .to_string(),
                    )));
                }
                Ok(Ok(()))
            }
            Err(report) => match report.downcast::<BitcoindError>()? {
                BitcoindError::JsonRpc(rpc_message) => Ok(Err(BitcoindRejectedTx(rpc_message))),
                bitcoind_error => Err(bitcoind_error.into()),
            },
        }
    }
}

impl std::fmt::Display for SlpBurns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut had_burn = false;
        for (input_idx, burn) in self.0.iter().enumerate() {
            if let Some(burn) = burn {
                if burn.token == SlpToken::EMPTY {
                    continue;
                }
                if had_burn {
                    write!(f, ", ")?;
                }
                write!(f, "input at index {} ", input_idx)?;
                if burn.token.amount != SlpAmount::ZERO {
                    write!(f, "burns {} base tokens ", burn.token.amount)?;
                }
                if burn.token.is_mint_baton {
                    write!(f, "burns mint baton ")?;
                }
                write!(f, "of token ID {}", burn.token_id.hash())?;
                had_burn = true;
            }
        }
        Ok(())
    }
}

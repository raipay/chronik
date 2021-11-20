use bitcoinsuite_core::{
    compression::read_undo_coin, encoding::read_compact_size, BitcoinCode, Bytes, Coin, OutPoint,
    Sha256d, UnhashedTx,
};
use bitcoinsuite_error::Result;
use bitcoinsuite_slp::{RichTx, RichTxBlock, SlpBurn};
use chronik_rocksdb::{Block, BlockTx, MempoolTxEntry, TxNum};

use crate::SlpIndexer;

pub struct Txs<'a> {
    indexer: &'a SlpIndexer,
}

impl<'a> Txs<'a> {
    pub fn new(indexer: &'a SlpIndexer) -> Self {
        Txs { indexer }
    }

    pub fn rich_tx_by_txid(&self, txid: &Sha256d) -> Result<Option<RichTx>> {
        if let Some(entry) = self.indexer.db_mempool().tx(txid) {
            return Ok(Some(self.rich_mempool_tx(txid, entry)?));
        }
        match self.indexer.db().txs()?.tx_and_num_by_txid(txid)? {
            Some((tx_num, block_tx)) => Ok(Some(self.rich_block_tx(tx_num, &block_tx)?)),
            None => Ok(None),
        }
    }

    pub(crate) fn rich_mempool_tx(&self, txid: &Sha256d, entry: &MempoolTxEntry) -> Result<RichTx> {
        let tx = entry.tx.clone().hashed();
        let slp_tx_data = self.indexer.db_mempool_slp().slp_tx_data(txid);
        let mut spends = vec![None; tx.outputs().len()];
        if let Some(spent_set) = self.indexer.db_mempool().spends(txid) {
            for &(out_idx, ref txid, input_idx) in spent_set {
                spends[out_idx as usize] = Some(OutPoint {
                    txid: txid.clone(),
                    out_idx: input_idx,
                })
            }
        }
        let (slp_burns, slp_error_msg) = match slp_tx_data {
            Some(slp_tx_data) => (slp_tx_data.slp_burns.clone(), None),
            None => {
                let slp_burns = tx
                    .inputs()
                    .iter()
                    .map(|input| self.output_token_burn(&input.prev_out))
                    .collect::<Result<Vec<_>>>()?;
                let slp_error_msg = self
                    .indexer
                    .db_mempool_slp()
                    .slp_tx_error(txid)
                    .map(|error| error.to_string());
                (slp_burns, slp_error_msg)
            }
        };
        return Ok(RichTx {
            tx,
            txid: txid.clone(),
            block: None,
            slp_tx_data: slp_tx_data.map(|slp_tx_data| slp_tx_data.slp_tx_data.clone().into()),
            spent_coins: Some(
                entry
                    .spent_outputs
                    .iter()
                    .map(|tx_output| Coin {
                        tx_output: tx_output.clone(),
                        ..Default::default()
                    })
                    .collect(),
            ),
            spends,
            slp_burns,
            slp_error_msg,
            time_first_seen: entry.time_first_seen,
            network: self.indexer.network,
        });
    }

    pub(crate) fn rich_block_tx(&self, tx_num: TxNum, block_tx: &BlockTx) -> Result<RichTx> {
        let block_reader = self.indexer.db().blocks()?;
        let block = block_reader
            .by_height(block_tx.block_height)?
            .expect("Inconsistent db");
        let raw_tx = self.indexer.rpc_interface.get_block_slice(
            block.file_num,
            block_tx.entry.data_pos,
            block_tx.entry.tx_size,
        )?;
        let spent_coins = match block_tx.entry.undo_pos {
            0 => None,
            _ => {
                let undo_data = self.indexer.rpc_interface.get_undo_slice(
                    block.file_num,
                    block_tx.entry.undo_pos,
                    block_tx.entry.undo_size,
                )?;
                let mut undo_data = Bytes::from_bytes(undo_data);
                let num_inputs = read_compact_size(&mut undo_data)?;
                let spent_coins = (0..num_inputs)
                    .map(|_| Ok(read_undo_coin(self.indexer.ecc.as_ref(), &mut undo_data)?))
                    .collect::<Result<Vec<_>>>()?;
                Some(spent_coins)
            }
        };
        self.rich_block_tx_prefetched(tx_num, block_tx, raw_tx.into(), spent_coins, &block)
    }

    pub(crate) fn rich_block_tx_prefetched(
        &self,
        tx_num: TxNum,
        block_tx: &BlockTx,
        mut raw_tx: Bytes,
        spent_coins: Option<Vec<Coin>>,
        block: &Block,
    ) -> Result<RichTx> {
        let txid = &block_tx.entry.txid;
        let spend_reader = self.indexer.db().spends()?;
        let tx_reader = self.indexer.db().txs()?;
        let slp_reader = self.indexer.db().slp()?;
        let tx = UnhashedTx::deser(&mut raw_tx)?;
        let mut spends = vec![None; tx.outputs.len()];
        for spend_entry in spend_reader.spends_by_tx_num(tx_num)? {
            spends[spend_entry.out_idx as usize] = Some(OutPoint {
                txid: tx_reader
                    .txid_by_tx_num(spend_entry.tx_num)?
                    .unwrap_or_default(),
                out_idx: spend_entry.input_idx,
            })
        }
        if let Some(spent_set) = self.indexer.db_mempool().spends(txid) {
            for &(out_idx, ref txid, input_idx) in spent_set {
                spends[out_idx as usize] = Some(OutPoint {
                    txid: txid.clone(),
                    out_idx: input_idx,
                })
            }
        }
        let (slp_tx_data, slp_burns) = match slp_reader.slp_data_by_tx_num(tx_num)? {
            Some((slp_tx_data, slp_burns)) => (Some(slp_tx_data), slp_burns),
            None => (
                None,
                tx.inputs
                    .iter()
                    .map(|input| self.output_token_burn(&input.prev_out))
                    .collect::<Result<Vec<_>>>()?,
            ),
        };
        let slp_error_msg = slp_reader.slp_invalid_message_tx_num(tx_num)?;
        Ok(RichTx {
            tx: tx.hashed(),
            txid: txid.clone(),
            block: Some(RichTxBlock {
                height: block_tx.block_height,
                hash: block.hash.clone(),
                timestamp: block.timestamp,
            }),
            slp_tx_data: slp_tx_data.map(|slp_tx_data| slp_tx_data.into()),
            spent_coins,
            spends,
            slp_burns,
            slp_error_msg,
            time_first_seen: block_tx.entry.time_first_seen,
            network: self.indexer.network,
        })
    }

    pub fn raw_tx_by_id(&self, txid: &Sha256d) -> Result<Option<Bytes>> {
        if let Some(entry) = self.indexer.db_mempool().tx(txid) {
            return Ok(Some(entry.tx.ser()));
        }
        let tx_reader = self.indexer.db().txs()?;
        let block_reader = self.indexer.db().blocks()?;
        let block_tx = match tx_reader.by_txid(txid)? {
            Some(tuple) => tuple,
            None => return Ok(None),
        };
        let block = block_reader
            .by_height(block_tx.block_height)?
            .expect("Inconsistent db");
        let raw_tx = self.indexer.rpc_interface.get_block_slice(
            block.file_num,
            block_tx.entry.data_pos,
            block_tx.entry.tx_size,
        )?;
        let raw_tx = Bytes::from_bytes(raw_tx);
        Ok(Some(raw_tx))
    }

    fn output_token_burn(&self, outpoint: &OutPoint) -> Result<Option<Box<SlpBurn>>> {
        if outpoint.is_coinbase() {
            return Ok(None);
        }
        if let Some(slp_tx_data) = self.indexer.db_mempool_slp().slp_tx_data(&outpoint.txid) {
            return Ok(Some(Box::new(SlpBurn {
                token: slp_tx_data
                    .slp_tx_data
                    .output_tokens
                    .get(outpoint.out_idx as usize)
                    .cloned()
                    .unwrap_or_default(),
                token_id: slp_tx_data.slp_tx_data.token_id.clone(),
            })));
        }
        if self.indexer.db_mempool().tx(&outpoint.txid).is_some() {
            return Ok(None);
        }
        let tx_reader = self.indexer.db().txs()?;
        let slp_reader = self.indexer.db().slp()?;
        let tx_num = tx_reader
            .tx_num_by_txid(&outpoint.txid)?
            .expect("Inconsistent index");
        match slp_reader.slp_data_by_tx_num(tx_num)? {
            Some((slp_tx_data, _)) => {
                let token = slp_tx_data
                    .output_tokens
                    .get(outpoint.out_idx as usize)
                    .cloned()
                    .unwrap_or_default();
                Ok(Some(Box::new(SlpBurn {
                    token,
                    token_id: slp_tx_data.token_id,
                })))
            }
            None => Ok(None),
        }
    }
}

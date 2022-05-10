use bitcoinsuite_core::{TxOutput, UnhashedTx};
use bitcoinsuite_error::Result;
use byteorder::LE;
use rocksdb::{ColumnFamilyDescriptor, Options, WriteBatch};
use zerocopy::{AsBytes, FromBytes, Unaligned, I64, U64};

use crate::{data::interpret, Block, BlockHeight, BlockHeightZC, BlockTxs, Db, CF};

pub const CF_BLOCK_STATS: &str = "block_stats";

pub struct BlockStatsWriter<'a> {
    cf_block_stats: &'a CF,
}

pub struct BlockStatsReader<'a> {
    db: &'a Db,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct BlockStats {
    /// Block size of this block in bytes (including headers etc.)
    pub block_size: u64,
    /// Number of txs in this block
    pub num_txs: u64,
    /// Total number of tx inputs in block (including coinbase)
    pub num_inputs: u64,
    /// Total number of tx output in block (including coinbase)
    pub num_outputs: u64,
    /// Total number of satoshis spent by tx inputs
    pub sum_input_sats: i64,
    /// Block reward for this block
    pub sum_coinbase_output_sats: i64,
    /// Total number of satoshis in non-coinbase tx outputs
    pub sum_normal_output_sats: i64,
    /// Total number of satoshis burned using OP_RETURN
    pub sum_burned_sats: i64,
}

#[derive(Debug, Clone, FromBytes, AsBytes, Unaligned)]
#[repr(C)]
struct BlockStatsData {
    block_size: U64<LE>,
    num_txs: U64<LE>,
    num_inputs: U64<LE>,
    num_outputs: U64<LE>,
    sum_input_sats: I64<LE>,
    sum_normal_output_sats: I64<LE>,
    sum_coinbase_output_sats: I64<LE>,
    sum_burned_sats: I64<LE>,
}

impl<'a> BlockStatsWriter<'a> {
    pub fn add_cfs(columns: &mut Vec<ColumnFamilyDescriptor>) {
        columns.push(ColumnFamilyDescriptor::new(
            CF_BLOCK_STATS,
            Options::default(),
        ));
    }

    pub fn new(db: &'a Db) -> Result<Self> {
        let cf_block_stats = db.cf(CF_BLOCK_STATS)?;
        Ok(BlockStatsWriter { cf_block_stats })
    }

    pub fn insert_block_txs<'b>(
        &self,
        batch: &mut WriteBatch,
        block: &Block,
        txs: &[UnhashedTx],
        block_txs: &BlockTxs,
        block_spent_output_fn: impl Fn(/*tx_pos:*/ usize, /*out_idx:*/ usize) -> &'b TxOutput,
    ) -> Result<()> {
        let mut num_inputs = 0;
        let mut num_outputs = 0;
        let mut sum_input_sats = 0;
        let mut sum_normal_output_sats = 0;
        let mut sum_coinbase_output_sats = 0;
        let mut sum_burned_sats = 0;
        for tx in txs {
            sum_burned_sats += tx
                .outputs
                .iter()
                .filter(|output| output.script.is_opreturn())
                .map(|output| output.value)
                .sum::<i64>();
            let tx_output_sats = tx.outputs.iter().map(|output| output.value).sum::<i64>();
            if tx.inputs[0].prev_out.is_coinbase() {
                sum_coinbase_output_sats += tx_output_sats;
            } else {
                sum_normal_output_sats += tx_output_sats;
            }
            num_inputs += tx.inputs.len();
            num_outputs += tx.outputs.len();
        }
        for (tx_pos, tx) in txs.iter().skip(1).enumerate() {
            sum_input_sats += (0..tx.inputs.len())
                .map(|input_idx| block_spent_output_fn(tx_pos, input_idx).value)
                .sum::<i64>();
        }
        let block_intro_size = block_txs.txs[0].data_pos - block.data_pos;
        let sum_tx_size = block_txs
            .txs
            .iter()
            .map(|tx| tx.tx_size as u64)
            .sum::<u64>();
        let block_size = block_intro_size as u64 + sum_tx_size;
        let block_stats_data = BlockStatsData {
            block_size: U64::new(block_size),
            num_txs: U64::new(txs.len() as u64),
            num_inputs: U64::new(num_inputs as u64),
            num_outputs: U64::new(num_outputs as u64),
            sum_input_sats: I64::new(sum_input_sats),
            sum_normal_output_sats: I64::new(sum_normal_output_sats),
            sum_coinbase_output_sats: I64::new(sum_coinbase_output_sats),
            sum_burned_sats: I64::new(sum_burned_sats),
        };
        let block_height = BlockHeightZC::new(block.height);
        batch.put_cf(
            self.cf_block_stats,
            block_height.as_bytes(),
            block_stats_data.as_bytes(),
        );
        Ok(())
    }

    pub fn delete_by_height(&self, batch: &mut WriteBatch, height: BlockHeight) -> Result<()> {
        let height = BlockHeightZC::new(height);
        batch.delete_cf(self.cf_block_stats, height.as_bytes());
        Ok(())
    }
}

impl<'a> BlockStatsReader<'a> {
    pub fn new(db: &'a Db) -> Result<Self> {
        db.cf(CF_BLOCK_STATS)?;
        Ok(BlockStatsReader { db })
    }

    pub fn by_height(&self, block_height: BlockHeight) -> Result<Option<BlockStats>> {
        let block_height = BlockHeightZC::new(block_height);
        let block_stats = match self.db.get(self.cf_block_stats(), block_height.as_bytes())? {
            Some(block_stats) => block_stats,
            None => return Ok(None),
        };
        let block_stats = interpret::<BlockStatsData>(&block_stats)?;
        Ok(Some(BlockStats {
            block_size: block_stats.block_size.get(),
            num_txs: block_stats.num_txs.get(),
            num_inputs: block_stats.num_inputs.get(),
            num_outputs: block_stats.num_outputs.get(),
            sum_input_sats: block_stats.sum_input_sats.get(),
            sum_coinbase_output_sats: block_stats.sum_coinbase_output_sats.get(),
            sum_normal_output_sats: block_stats.sum_normal_output_sats.get(),
            sum_burned_sats: block_stats.sum_burned_sats.get(),
        }))
    }

    fn cf_block_stats(&self) -> &CF {
        self.db.cf(CF_BLOCK_STATS).unwrap()
    }
}

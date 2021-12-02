use bitcoinsuite_bitcoind_nng::BlockIdentifier;
use bitcoinsuite_core::{BitcoinCode, BitcoinHeader, LotusHeader, Network, Sha256d};
use bitcoinsuite_error::{ErrorMeta, Result};
use bitcoinsuite_slp::RichTx;
use chronik_rocksdb::{Block, BlockHeight, BlockReader};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use thiserror::Error;

use crate::SlpIndexer;

pub struct Blocks<'a> {
    indexer: &'a SlpIndexer,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum BlocksError {
    #[critical()]
    #[error("Inconsistent db, block hash doesn't exist: {0}")]
    InconsistentNoSuchBlock(Sha256d),

    #[critical()]
    #[error("Inconsistent db, txid doesn't exist: {0}")]
    InconsistentNoSuchBlockTx(Sha256d),
}

use self::BlocksError::*;

impl<'a> Blocks<'a> {
    pub fn new(indexer: &'a SlpIndexer) -> Self {
        Blocks { indexer }
    }

    pub fn height(&self) -> Result<BlockHeight> {
        self.reader()?.height()
    }

    pub fn tip(&self) -> Result<Option<Block>> {
        self.reader()?.tip()
    }

    pub fn by_hash(&self, hash: &Sha256d) -> Result<Option<Block>> {
        self.reader()?.by_hash(hash)
    }

    pub fn by_height(&self, height: BlockHeight) -> Result<Option<Block>> {
        self.reader()?.by_height(height)
    }

    pub fn raw_header(&self, block: &Block) -> Result<Option<Vec<u8>>> {
        let header_size = match self.indexer.network {
            Network::BCH | Network::XEC | Network::XRG => BitcoinHeader::default().ser().len(),
            Network::XPI => LotusHeader::default().ser().len(),
        };
        let header = self.indexer.rpc_interface.get_block_slice(
            block.file_num,
            block.data_pos,
            header_size as u32,
        )?;
        Ok(Some(header))
    }

    pub fn block_txs_by_hash(&self, hash: &Sha256d) -> Result<Vec<RichTx>> {
        self.block_txs_by_identifier(BlockIdentifier::Hash(hash.clone()))
    }

    pub fn block_txs_by_height(&self, height: BlockHeight) -> Result<Vec<RichTx>> {
        self.block_txs_by_identifier(BlockIdentifier::Height(height))
    }

    fn block_txs_by_identifier(&self, block_id: BlockIdentifier) -> Result<Vec<RichTx>> {
        let nng_block = self.indexer.rpc_interface.get_block(block_id)?;
        let txs = self.indexer.txs();
        let db_txs = self.indexer.db().txs()?;
        let db_blocks = self.indexer.db().blocks()?;
        let block = db_blocks
            .by_hash(&nng_block.header.hash)?
            .ok_or_else(|| InconsistentNoSuchBlock(nng_block.header.hash.clone()))?;
        nng_block
            .txs
            .into_par_iter()
            .map(|nng_block_tx| {
                let (tx_num, block_tx) = db_txs
                    .tx_and_num_by_txid(&nng_block_tx.tx.txid)?
                    .ok_or_else(|| InconsistentNoSuchBlockTx(nng_block_tx.tx.txid.clone()))?;
                txs.rich_block_tx_prefetched(
                    tx_num,
                    &block_tx,
                    nng_block_tx.tx.raw.into(),
                    nng_block_tx.tx.spent_coins,
                    &block,
                )
            })
            .collect::<Result<_>>()
    }

    fn reader(&self) -> Result<BlockReader> {
        self.indexer.db.blocks()
    }
}

use bitcoinsuite_core::{BitcoinCode, BitcoinHeader, LotusHeader, Network, Sha256d};
use bitcoinsuite_error::Result;
use chronik_rocksdb::{Block, BlockHeight, BlockReader};

use crate::SlpIndexer;

pub struct Blocks<'a> {
    indexer: &'a SlpIndexer,
}

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

    fn reader(&self) -> Result<BlockReader> {
        self.indexer.db.blocks()
    }
}

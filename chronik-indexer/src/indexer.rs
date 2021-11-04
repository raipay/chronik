use bitcoinsuite_bitcoind::cli::BitcoinCli;
use bitcoinsuite_bitcoind_nng::{Message, PubInterface, RpcInterface};
use bitcoinsuite_core::{BitcoinCode, Bytes, Hashed, Sha256d, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Result};
use thiserror::Error;

use chronik_rocksdb::{
    Block, BlockReader, BlockTxs, IndexDb, IndexMemData, OutputsReader, SpendsReader, TxEntry,
    TxReader, UtxosReader,
};

pub struct SlpIndexer {
    db: IndexDb,
    bitcoind: BitcoinCli,
    rpc_interface: RpcInterface,
    pub_interface: PubInterface,
    data: IndexMemData,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum SlpIndexerError {
    #[critical()]
    #[error(
        "Index and node diverged: index height is {index_height}, tip is {index_tip}, \
             node height is {node_height}, tip is {node_tip}"
    )]
    IndexDiverged {
        index_height: i32,
        index_tip: Sha256d,
        node_height: i32,
        node_tip: Sha256d,
    },
    #[critical()]
    #[error("Unexpected plugin message: {0:?}")]
    UnexpectedPluginMessage(Message),
}

impl SlpIndexer {
    pub fn new(
        db: IndexDb,
        bitcoind: BitcoinCli,
        rpc_interface: RpcInterface,
        pub_interface: PubInterface,
        data: IndexMemData,
    ) -> Result<Self> {
        pub_interface.subscribe("------------")?;
        Ok(SlpIndexer {
            db,
            bitcoind,
            rpc_interface,
            pub_interface,
            data,
        })
    }

    /// returns whether Initial Block Download has finished and the index is sync'd
    pub fn catchup_step(&mut self) -> Result<bool> {
        let blockchain_info = self.bitcoind.cmd_json("getblockchaininfo", &[])?;
        let tip = self.db.blocks()?.tip()?;
        let tip_ref = tip.as_ref();
        let index_height = tip_ref.map(|block| block.height).unwrap_or(-1);
        let index_best_block_hash = tip_ref.map(|block| block.hash.clone()).unwrap_or_default();
        let node_height = blockchain_info["blocks"].as_i32().unwrap();
        let node_best_block_hash = blockchain_info["bestblockhash"].as_str().unwrap();
        // Sanity check: block tips match, if on equal height
        if node_height == index_height
            && blockchain_info["bestblockhash"] != index_best_block_hash.to_hex_be()
        {
            return Err(SlpIndexerError::IndexDiverged {
                index_height,
                index_tip: index_best_block_hash,
                node_height,
                node_tip: Sha256d::from_hex_be(node_best_block_hash)?,
            }
            .into());
        }
        // Sanity check: index higher than node
        if index_height > node_height {
            return Err(SlpIndexerError::IndexDiverged {
                index_height,
                index_tip: index_best_block_hash,
                node_height,
                node_tip: Sha256d::from_hex_be(node_best_block_hash)?,
            }
            .into());
        }
        if !blockchain_info["initialblockdownload"]
            .as_bool()
            .unwrap_or_default()
        {
            // Index and node fully sync'd
            if node_height == index_height {
                return Ok(true);
            }
        } else {
            // Node not fully sync'd, but index up-to-date, so we wait for the next block
            if node_height == index_height {
                self.pub_interface.unsubscribe("------------")?;
                self.pub_interface.subscribe("blkconnected")?;
                let msg = self.pub_interface.recv()?;
                self.pub_interface.unsubscribe("blkconnected")?;
                self.pub_interface.subscribe("------------")?;
                match msg {
                    Message::BlockConnected(block_connected) => {
                        self._handle_block(tip, block_connected.block)?;
                        self.pub_interface.subscribe("------------")?;
                        return Ok(false);
                    }
                    msg => return Err(SlpIndexerError::UnexpectedPluginMessage(msg).into()),
                }
            }
        }

        // Index did not catch up with node, use historic blocks
        let t_rpc_blocks = std::time::Instant::now();
        let blocks = self.rpc_interface.get_block_range(index_height + 1, 50)?;
        println!(
            "t_rpc_blocks: {}",
            t_rpc_blocks.elapsed().as_secs_f64() * 1000.0
        );
        let t_handle_blocks = std::time::Instant::now();
        for block in blocks {
            let tip = self.db.blocks()?.tip()?;
            self._handle_block(tip, block)?;
        }
        println!(
            "t_handle_blocks: {}",
            t_handle_blocks.elapsed().as_secs_f64() * 1000.0
        );

        Ok(false)
    }

    pub fn leave_catchup(&self) -> Result<()> {
        self.pub_interface.unsubscribe("------------")?;
        self.pub_interface.subscribe("blkconnected")?;
        self.pub_interface.subscribe("blkdisconctd")?;
        Ok(())
    }

    pub fn process_next_msg(&mut self) -> Result<()> {
        let msg = self.pub_interface.recv()?;
        match msg {
            Message::BlockConnected(block_connected) => {
                println!("Got BlockConnected {}", block_connected.block.header.hash);
                let tip = self.db.blocks()?.tip()?;
                self._handle_block(tip, block_connected.block)?;
            }
            Message::BlockDisconnected(block_disconnected) => {
                println!(
                    "Got BlockDisconnected {}",
                    block_disconnected.block.header.hash
                );
                let tip = self.db.blocks()?.tip()?;
                self._handle_block_disconnected(tip, block_disconnected.block)?;
            }
            msg => return Err(SlpIndexerError::UnexpectedPluginMessage(msg).into()),
        }
        Ok(())
    }

    pub fn blocks(&self) -> Result<BlockReader> {
        self.db.blocks()
    }

    pub fn txs(&self) -> Result<TxReader> {
        self.db.txs()
    }

    pub fn outputs(&self) -> Result<OutputsReader> {
        self.db.outputs()
    }

    pub fn utxos(&self) -> Result<UtxosReader> {
        self.db.utxos()
    }

    pub fn spends(&self) -> Result<SpendsReader> {
        self.db.spends()
    }

    fn _block_txs(block: &bitcoinsuite_bitcoind_nng::Block) -> Result<Vec<UnhashedTx>> {
        block
            .txs
            .iter()
            .map(|tx| {
                let mut raw_tx = Bytes::from_slice(&tx.tx.raw);
                UnhashedTx::deser(&mut raw_tx).map_err(Into::into)
            })
            .collect()
    }

    fn _handle_block(
        &mut self,
        tip: Option<Block>,
        block: bitcoinsuite_bitcoind_nng::Block,
    ) -> Result<()> {
        let next_height = tip.as_ref().map(|tip| tip.height + 1).unwrap_or(0);
        let txs = Self::_block_txs(&block)?;
        let db_block = Block {
            hash: block.header.hash.clone(),
            prev_hash: block.header.prev_hash,
            height: next_height,
            n_bits: block.header.n_bits,
            timestamp: block.header.timestamp.try_into().unwrap(),
            file_num: block.file_num,
        };
        let num_txs = block.txs.len();
        let db_txs = block
            .txs
            .iter()
            .map(|tx| TxEntry {
                txid: tx.tx.txid.clone(),
                data_pos: tx.data_pos,
                tx_size: tx.tx.raw.len() as u32,
            })
            .collect::<Vec<_>>();
        let db_block_txs = BlockTxs {
            txs: db_txs,
            block_height: next_height,
        };
        self.db.insert_block(
            &db_block,
            &db_block_txs,
            &txs,
            |tx_pos, input_idx| {
                &block.txs[tx_pos + 1].tx.spent_coins.as_ref().unwrap()[input_idx]
                    .tx_output
                    .script
            },
            &mut self.data,
        )?;
        println!(
            "Added block {} with {} txs, height {}",
            block.header.hash, num_txs, next_height
        );
        Ok(())
    }

    fn _handle_block_disconnected(
        &mut self,
        tip: Option<Block>,
        block: bitcoinsuite_bitcoind_nng::Block,
    ) -> Result<()> {
        let txs = Self::_block_txs(&block)?;
        let tip = tip.unwrap();
        let txids_fn = |idx: usize| &block.txs[idx].tx.txid;
        self.db.delete_block(
            &block.header.hash,
            tip.height,
            txids_fn,
            &txs,
            |tx_pos, input_idx| {
                &block.txs[tx_pos + 1].tx.spent_coins.as_ref().unwrap()[input_idx]
                    .tx_output
                    .script
            },
            &mut self.data,
        )?;
        println!(
            "Removed block {} via BlockDisconnected message",
            block.header.hash
        );
        Ok(())
    }
}

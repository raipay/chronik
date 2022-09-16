use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bitcoinsuite_bitcoind::rpc_client::BitcoindRpcClient;
use bitcoinsuite_bitcoind_nng::{BlockTx, MempoolTx, Message, PubInterface, RpcInterface};
use bitcoinsuite_core::{
    ecc::Ecc, BitcoinCode, Bytes, Hashed, Network, Script, Sha256d, UnhashedTx,
};
use bitcoinsuite_error::{ErrorMeta, Result};
use chronik_rocksdb::{
    script_payloads, Block, BlockHeight, BlockTxs, IndexDb, IndexMemData, MempoolData,
    MempoolSlpData, MempoolTxEntry, TransientBlockDataReader, TxEntry,
};
use thiserror::Error;
use tokio::sync::RwLock;

use crate::{
    broadcast::Broadcast,
    subscribers::{
        SubscribeAllTxsMessage, SubscribeBlockMessage, SubscribeScriptMessage, Subscribers,
    },
    txs::Txs,
    Blocks, ScriptHistory, Tokens, Utxos,
};

pub struct SlpIndexer {
    pub(crate) db: IndexDb,
    pub(crate) bitcoind: BitcoindRpcClient,
    pub(crate) rpc_interface: RpcInterface,
    pub(crate) pub_interface: PubInterface,
    pub(crate) data: IndexMemData,
    pub(crate) network: Network,
    pub(crate) ecc: Arc<dyn Ecc + Sync + Send>,
    subscribers: Subscribers,
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
        bitcoind: BitcoindRpcClient,
        rpc_interface: RpcInterface,
        pub_interface: PubInterface,
        data: IndexMemData,
        network: Network,
        ecc: Arc<dyn Ecc + Sync + Send>,
    ) -> Result<Self> {
        db.check_db_version()?;
        pub_interface.subscribe("------------")?;
        Ok(SlpIndexer {
            db,
            bitcoind,
            rpc_interface,
            pub_interface,
            data,
            network,
            ecc,
            subscribers: Subscribers::default(),
        })
    }

    /// returns whether Initial Block Download has finished and the index is sync'd
    pub async fn catchup_step(&mut self) -> Result<bool> {
        let blockchain_info = self.bitcoind.cmd_json("getblockchaininfo", &[]).await?;
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
                        self.handle_block(tip, block_connected.block)?;
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
            self.handle_block(tip, block)?;
        }
        println!(
            "t_handle_blocks: {}",
            t_handle_blocks.elapsed().as_secs_f64() * 1000.0
        );

        Ok(false)
    }

    pub fn leave_catchup(&mut self) -> Result<()> {
        let mempool = self.rpc_interface.get_mempool()?;
        self.pub_interface.unsubscribe("------------")?;
        self.pub_interface.subscribe("blkconnected")?;
        self.pub_interface.subscribe("blkdisconctd")?;
        self.pub_interface.subscribe("mempooltxadd")?;
        self.pub_interface.subscribe("mempooltxrem")?;
        let txs = mempool
            .into_iter()
            .map(|mempool_tx| {
                let mut raw_tx = Bytes::from_bytes(mempool_tx.tx.raw);
                let tx = UnhashedTx::deser(&mut raw_tx)?;
                let spent_coins = mempool_tx.tx.spent_coins.unwrap_or_default();
                let entry = MempoolTxEntry {
                    tx,
                    spent_coins,
                    time_first_seen: mempool_tx.time,
                };
                Ok((mempool_tx.tx.txid, entry))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        println!("Found {} txs in mempool", txs.len());
        self.db.insert_mempool_batch_txs(&mut self.data, txs)?;
        Ok(())
    }

    pub fn process_msg(&mut self, msg: Message) -> Result<()> {
        match msg {
            Message::BlockConnected(block_connected) => {
                println!("Got BlockConnected {}", block_connected.block.header.hash);
                let tip = self.db.blocks()?.tip()?;
                self.handle_block(tip, block_connected.block)?;
            }
            Message::BlockDisconnected(block_disconnected) => {
                println!(
                    "Got BlockDisconnected {}",
                    block_disconnected.block.header.hash
                );
                let tip = self.db.blocks()?.tip()?;
                self.handle_block_disconnected(tip, block_disconnected.block)?;
            }
            Message::TransactionAddedToMempool(mempool_tx_added) => {
                println!(
                    "Got TransactionAddedToMempool {}",
                    mempool_tx_added.mempool_tx.tx.txid,
                );
                self.handle_tx_added_to_mempool(mempool_tx_added.mempool_tx)?;
            }
            Message::TransactionRemovedFromMempool(mempool_tx_removed) => {
                println!(
                    "Got TransactionRemovedFromMempool {}",
                    mempool_tx_removed.txid
                );
                self.handle_tx_removed_from_mempool(mempool_tx_removed.txid)?;
            }
            msg => return Err(SlpIndexerError::UnexpectedPluginMessage(msg).into()),
        }
        Ok(())
    }

    pub fn process_next_msg(&mut self) -> Result<()> {
        let msg = self.pub_interface.recv()?;
        self.process_msg(msg)?;
        Ok(())
    }

    pub fn bitcoind_rpc(&self) -> &BitcoindRpcClient {
        &self.bitcoind
    }

    pub fn db(&self) -> &IndexDb {
        &self.db
    }

    pub fn db_mempool(&self) -> &MempoolData {
        self.db.mempool(&self.data)
    }

    pub fn db_mempool_slp(&self) -> &MempoolSlpData {
        self.db.mempool_slp(&self.data)
    }

    pub fn txs(&self) -> Txs {
        Txs::new(self)
    }

    pub fn blocks(&self) -> Blocks {
        Blocks::new(self)
    }

    pub fn script_history(&self) -> ScriptHistory {
        ScriptHistory::new(self)
    }

    pub fn utxos(&self) -> Utxos {
        Utxos::new(self)
    }

    pub fn tokens(&self) -> Tokens {
        Tokens::new(self)
    }

    pub fn broadcast(&self) -> Broadcast {
        Broadcast::new(self)
    }

    pub fn subscribers_mut(&mut self) -> &mut Subscribers {
        &mut self.subscribers
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

    fn handle_block(
        &mut self,
        tip: Option<Block>,
        block: bitcoinsuite_bitcoind_nng::Block,
    ) -> Result<()> {
        let next_height = tip.as_ref().map(|tip| tip.height + 1).unwrap_or(0);
        let txs = Self::_block_txs(&block)?;
        Self::broadcast_block_msg(
            &mut self.subscribers,
            block.header.hash.clone(),
            &txs,
            &block.txs,
            true,
        );
        let db_block = Block {
            hash: block.header.hash.clone(),
            prev_hash: block.header.prev_hash,
            height: next_height,
            n_bits: block.header.n_bits,
            timestamp: block.header.timestamp.try_into().unwrap(),
            file_num: block.file_num,
            data_pos: block.data_pos,
        };
        let transient_block_data = self.db.transient_data().read_block(next_height)?;
        let mut transient_data_reader =
            TransientBlockDataReader::new(match &transient_block_data {
                Some(block_data) => &block_data.tx_data,
                None => &[],
            });
        let num_txs = block.txs.len();
        let db_txs = block
            .txs
            .iter()
            .zip(&txs)
            .map(|(block_tx, tx)| {
                let txid = &block_tx.tx.txid;
                let time_first_seen = match self.db_mempool().tx(txid) {
                    Some(entry) => entry.time_first_seen,
                    None => match transient_data_reader.read_for_next_txid(txid) {
                        Some(transient_entry) => transient_entry.time_first_seen,
                        None => 0, // indicates unknown
                    },
                };
                TxEntry {
                    txid: txid.clone(),
                    data_pos: block_tx.data_pos,
                    tx_size: block_tx.tx.raw.len() as u32,
                    undo_pos: block_tx.undo_pos,
                    undo_size: block_tx.undo_size,
                    time_first_seen,
                    is_coinbase: tx.inputs[0].prev_out.is_coinbase(),
                }
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
                &block.txs[tx_pos + 1].tx.spent_coins.as_ref().unwrap()[input_idx].tx_output
            },
            &mut self.data,
        )?;
        self.update_transient_data(next_height)?;
        println!(
            "Added block {} with {} txs, height {}",
            block.header.hash, num_txs, next_height,
        );
        Ok(())
    }

    fn handle_block_disconnected(
        &mut self,
        tip: Option<Block>,
        block: bitcoinsuite_bitcoind_nng::Block,
    ) -> Result<()> {
        let txs = Self::_block_txs(&block)?;
        Self::broadcast_block_msg(
            &mut self.subscribers,
            block.header.hash.clone(),
            &txs,
            &block.txs,
            false,
        );
        let tip = tip.unwrap();
        let txids_fn = |idx: usize| &block.txs[idx].tx.txid;
        self.db.delete_block(
            &block.header.hash,
            tip.height,
            txids_fn,
            &txs,
            |tx_pos, input_idx| {
                &block.txs[tx_pos + 1].tx.spent_coins.as_ref().unwrap()[input_idx].tx_output
            },
            &mut self.data,
        )?;
        self.db.transient_data_writer().delete_block(tip.height)?;
        println!(
            "Removed block {} via BlockDisconnected message",
            block.header.hash
        );
        Ok(())
    }

    fn handle_tx_added_to_mempool(&mut self, mempool_tx: MempoolTx) -> Result<()> {
        let nng_tx = mempool_tx.tx;
        let mut raw_tx = Bytes::from_bytes(nng_tx.raw);
        let tx = UnhashedTx::deser(&mut raw_tx)?;
        let spent_coins = nng_tx.spent_coins.unwrap_or_default();
        Self::broadcast_msg(
            &mut self.subscribers,
            SubscribeScriptMessage::AddedToMempool(nng_tx.txid.clone()),
            spent_coins
                .iter()
                .map(|spent_output| &spent_output.tx_output.script),
            tx.outputs.iter().map(|spent_output| &spent_output.script),
        );
        self.subscribers
            .broadcast_to_all_txs(SubscribeAllTxsMessage::AddedToMempool(nng_tx.txid.clone()));
        let entry = MempoolTxEntry {
            tx,
            spent_coins,
            time_first_seen: mempool_tx.time,
        };
        self.db
            .insert_mempool_tx(&mut self.data, nng_tx.txid, entry)?;
        Ok(())
    }

    fn handle_tx_removed_from_mempool(&mut self, txid: Sha256d) -> Result<()> {
        if let Some(tx) = self.db.mempool(&self.data).tx(&txid) {
            Self::broadcast_msg(
                &mut self.subscribers,
                SubscribeScriptMessage::RemovedFromMempool(txid.clone()),
                tx.spent_coins
                    .iter()
                    .map(|spent_coin| &spent_coin.tx_output.script),
                tx.tx.outputs.iter().map(|output| &output.script),
            );
        }
        self.db.remove_mempool_tx(&mut self.data, &txid)?;
        Ok(())
    }

    fn broadcast_msg<'a>(
        subscribers: &mut Subscribers,
        msg: SubscribeScriptMessage,
        spent_scripts: impl IntoIterator<Item = &'a Script>,
        output_scripts: impl IntoIterator<Item = &'a Script>,
    ) {
        let mut notified_payloads = HashSet::new();
        for script in spent_scripts.into_iter().chain(output_scripts) {
            for script_payload in script_payloads(script) {
                let script_payload = script_payload.payload;
                if !notified_payloads.contains(&script_payload) {
                    subscribers.broadcast_to_script(&script_payload, msg.clone());
                    notified_payloads.insert(script_payload);
                }
            }
        }
    }

    fn broadcast_block_msg(
        subscribers: &mut Subscribers,
        block_hash: Sha256d,
        txs: &[UnhashedTx],
        block_txs: &[BlockTx],
        is_confirmed: bool,
    ) {
        subscribers.broadcast_to_blocks(if is_confirmed {
            SubscribeBlockMessage::BlockConnected(block_hash)
        } else {
            SubscribeBlockMessage::BlockDisconnected(block_hash)
        });
        for (tx, block_tx) in txs.iter().zip(block_txs) {
            let spent_scripts = block_tx.tx.spent_coins.iter().flat_map(|spent_coins| {
                spent_coins
                    .iter()
                    .map(|spent_coin| &spent_coin.tx_output.script)
            });
            Self::broadcast_msg(
                subscribers,
                match is_confirmed {
                    true => SubscribeScriptMessage::Confirmed(block_tx.tx.txid.clone()),
                    false => SubscribeScriptMessage::Reorg(block_tx.tx.txid.clone()),
                },
                spent_scripts,
                tx.outputs.iter().map(|output| &output.script),
            )
        }
    }

    fn update_transient_data(&mut self, tip_height: BlockHeight) -> Result<()> {
        let next_block_height = self.db.transient_data().next_block_height().unwrap();
        // Only update if transient data caught up 12 blocks deep.
        // This overlaps with run_transient_data_catchup in case there is a race condition.
        // Since this requires Write access and run_transient_data_catchup requires Read access,
        // both will never update simultaneously.
        if next_block_height + 12 <= tip_height {
            return Ok(());
        }
        for block_height in next_block_height..=tip_height {
            self.db.transient_data_writer().update_block(block_height)?;
        }
        Ok(())
    }
}

pub async fn run_transient_data_catchup(slp_indexer: &RwLock<SlpIndexer>) -> Result<()> {
    loop {
        let slp_indexer = slp_indexer.read().await;
        let tip = match slp_indexer.db().blocks().unwrap().tip().unwrap() {
            Some(tip) => tip,
            None => break,
        };
        let next_block_height = slp_indexer
            .db()
            .transient_data()
            .next_block_height()
            .unwrap();
        // Stop when we're 10 blocks away from tip
        if next_block_height + 10 > tip.height {
            break;
        }
        slp_indexer
            .db()
            .transient_data_writer()
            .update_block(next_block_height)
            .unwrap();
        if next_block_height % 100 == 0 {
            println!("Synced transient data up to height {}", next_block_height);
        }
    }
    Ok(())
}

use std::collections::HashMap;

use bitcoinsuite_core::Sha256d;
use chronik_rocksdb::ScriptPayload;
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeScriptMessage {
    AddedToMempool(Sha256d),
    RemovedFromMempool(Sha256d),
    Confirmed(Sha256d),
    Reorg(Sha256d),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeBlockMessage {
    BlockConnected(Sha256d),
    BlockDisconnected(Sha256d),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeAllTxsMessage {
    AddedToMempool(Sha256d),
}

const SCRIPT_CHANNEL_CAPACITY: usize = 16;
const BLOCK_CHANNEL_CAPACITY: usize = 16;
const ALL_TX_CHANNEL_CAPACITY: usize = 64;

#[derive(Debug, Clone)]
pub struct Subscribers {
    subs_script: HashMap<ScriptPayload, broadcast::Sender<SubscribeScriptMessage>>,
    subs_block: broadcast::Sender<SubscribeBlockMessage>,
    subs_all_tx: broadcast::Sender<SubscribeAllTxsMessage>,
}

impl Subscribers {
    pub fn subscribe_to_script(
        &mut self,
        script: &ScriptPayload,
    ) -> broadcast::Receiver<SubscribeScriptMessage> {
        match self.subs_script.get(script) {
            Some(sender) => sender.subscribe(),
            None => {
                let (sender, receiver) = broadcast::channel(SCRIPT_CHANNEL_CAPACITY);
                self.subs_script.insert(script.clone(), sender);
                receiver
            }
        }
    }

    /// Clean unsubscribe
    pub fn unsubscribe_from_script(&mut self, script: &ScriptPayload) {
        if let Some(sender) = self.subs_script.get(script) {
            if sender.receiver_count() == 0 {
                self.subs_script.remove(script);
            }
        }
    }

    pub fn subscribe_to_blocks(&self) -> broadcast::Receiver<SubscribeBlockMessage> {
        self.subs_block.subscribe()
    }

    pub fn subscribe_to_all_txs(&self) -> broadcast::Receiver<SubscribeAllTxsMessage> {
        self.subs_all_tx.subscribe()
    }

    pub(crate) fn broadcast_to_script(
        &mut self,
        script: &ScriptPayload,
        msg: SubscribeScriptMessage,
    ) {
        if let Some(sender) = self.subs_script.get(script) {
            // Unclean unsubscribe
            if sender.send(msg).is_err() {
                self.subs_script.remove(script);
            }
        }
    }

    pub(crate) fn broadcast_to_blocks(&mut self, msg: SubscribeBlockMessage) {
        if self.subs_block.receiver_count() > 0 {
            if let Err(err) = self.subs_block.send(msg) {
                eprintln!("Unexpected send error: {}", err);
            }
        }
    }

    pub(crate) fn broadcast_to_all_txs(&mut self, msg: SubscribeAllTxsMessage) {
        if self.subs_all_tx.receiver_count() > 0 {
            if let Err(err) = self.subs_all_tx.send(msg) {
                eprintln!("Unexpected send error (broadcast_to_all_txs): {}", err);
            }
        }
    }
}

impl Default for Subscribers {
    fn default() -> Self {
        Subscribers {
            subs_script: Default::default(),
            subs_block: broadcast::channel(BLOCK_CHANNEL_CAPACITY).0,
            subs_all_tx: broadcast::channel(ALL_TX_CHANNEL_CAPACITY).0,
        }
    }
}

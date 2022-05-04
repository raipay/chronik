use bitcoinsuite_error::Result;
use bitcoinsuite_slp::TokenId;
use chronik_rocksdb::TokenStats;

use crate::SlpIndexer;

pub struct Tokens<'a> {
    indexer: &'a SlpIndexer,
}

impl<'a> Tokens<'a> {
    pub fn new(indexer: &'a SlpIndexer) -> Self {
        Tokens { indexer }
    }

    pub fn token_stats_by_token_id(&self, token_id: &TokenId) -> Result<Option<TokenStats>> {
        let slp_reader = self.indexer.db.slp()?;
        let db_token_stats = match slp_reader.token_num_by_id(token_id)? {
            Some(token_num) => self.indexer.db.slp()?.token_stats_by_token_num(token_num)?,
            None => None,
        };
        let mempool_token_stats = self.indexer.db_mempool_slp().token_stats_delta(token_id);
        match (db_token_stats, mempool_token_stats) {
            (None, None) => Ok(None),
            (None, Some(token_stats)) => Ok(Some(token_stats.clone())),
            (Some(token_stats), None) => Ok(Some(token_stats)),
            (Some(mut token_stats), Some(mempool_token_stats)) => {
                token_stats.total_minted += mempool_token_stats.total_minted;
                token_stats.total_burned += mempool_token_stats.total_burned;
                Ok(Some(token_stats))
            }
        }
    }
}

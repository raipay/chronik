use std::{ffi::OsString, io::Read, net::SocketAddr, path::PathBuf, str::FromStr, sync::Arc};

use bitcoinsuite_bitcoind::cli::BitcoinCli;
use bitcoinsuite_bitcoind_nng::{PubInterface, RpcInterface};
use bitcoinsuite_core::Network;
use bitcoinsuite_ecc_secp256k1::EccSecp256k1;
use bitcoinsuite_error::{ErrorMeta, Result, WrapErr};
use chronik_http::ChronikServer;
use chronik_indexer::SlpIndexer;
use chronik_rocksdb::{Db, IndexDb, IndexMemData, ScriptTxsConf};
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::RwLock;

const SCRIPT_TXS_PAGE_SIZE: usize = 1000;

#[derive(Deserialize, Debug, Clone)]
struct ChronikConf {
    host: SocketAddr,
    bitcoin_cli: PathBuf,
    datadir: PathBuf,
    nng_pub_url: String,
    nng_rpc_url: String,
    db_path: PathBuf,
    cache_script_history: usize,
    network: Network,
}

#[derive(Error, ErrorMeta, Debug)]
pub enum ChronikExeError {
    #[critical()]
    #[error("No configuration file provided. Specify like this: cargo run -- <config path>")]
    NoConfigFile,

    #[critical()]
    #[error("Opening configuration file {0} failed")]
    OpenConfigFail(String),

    #[critical()]
    #[error("Failed to read configuration file {0}")]
    ReadConfigFail(String),

    #[critical()]
    #[error("Invalid configuration file {0}")]
    InvalidConfigFail(String),
}

use self::ChronikExeError::*;

#[tokio::main]
async fn main() -> Result<()> {
    bitcoinsuite_error::install()?;

    let conf_path = std::env::args().nth(1).ok_or(NoConfigFile)?;
    let mut file =
        std::fs::File::open(&conf_path).wrap_err_with(|| OpenConfigFail(conf_path.clone()))?;
    let mut conf_contents = String::new();
    file.read_to_string(&mut conf_contents)
        .wrap_err_with(|| ReadConfigFail(conf_path.clone()))?;
    let conf: ChronikConf =
        toml::from_str(&conf_contents).wrap_err_with(|| InvalidConfigFail(conf_path.clone()))?;

    let client = BitcoinCli {
        bitcoincli_path: conf.bitcoin_cli,
        datadir_arg: OsString::from_str(&format!("-datadir={}", conf.datadir.to_string_lossy()))?,
    };
    let pub_interface = PubInterface::open(&conf.nng_pub_url)?;
    let rpc_interface = RpcInterface::open(&conf.nng_rpc_url)?;

    let db = Db::open(&conf.db_path)?;

    let db = IndexDb::new(
        db,
        ScriptTxsConf {
            page_size: SCRIPT_TXS_PAGE_SIZE,
        },
    );
    let data = IndexMemData::new(conf.cache_script_history);
    let mut slp_indexer = SlpIndexer::new(
        db,
        client.clone(),
        rpc_interface,
        pub_interface.clone(),
        data,
        conf.network,
        Arc::new(EccSecp256k1::default()),
    )?;

    while !slp_indexer.catchup_step()? {}
    slp_indexer.leave_catchup()?;

    let slp_indexer = Arc::new(RwLock::new(slp_indexer));

    let server = ChronikServer {
        addr: conf.host,
        slp_indexer: Arc::clone(&slp_indexer),
    };
    tokio::spawn(server.run());

    loop {
        let msg = tokio::task::spawn_blocking({
            let pub_interface = pub_interface.clone();
            move || pub_interface.recv()
        })
        .await??;
        slp_indexer.write().await.process_msg(msg)?;
    }
}
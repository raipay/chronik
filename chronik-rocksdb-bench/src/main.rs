use std::time::Instant;

use bitcoinsuite_core::{
    build_bitcoin_block, build_bitcoin_coinbase, BitcoinCode, Bytes, Op, OutPoint, Script,
    SequenceNo, Sha256d, TxInput, TxOutput, UnhashedTx,
};
use bitcoinsuite_error::Result;
use bitcoinsuite_test_utils_blockchain::build_tx;
use chronik_rocksdb::{Block, BlockTxs, Db, IndexDb, IndexMemData, OutputsConf, TxEntry};
use rand::{distributions::WeightedIndex, prelude::Distribution, Rng, SeedableRng};
use tempdir::TempDir;

fn main() -> Result<()> {
    let num_blocks = 200;
    let cache_size = 10_000;
    let mut blocks = Vec::new();

    let anyone_script = Script::from_slice(&[0x51]);

    // Generate 101 initial blocks
    let mut prev_block_hash = Sha256d::default();
    let mut timestamp = 1296688602;
    let num_initial_utxos = 101;
    let mut initial_utxos = Vec::with_capacity(num_initial_utxos);
    for block_idx in 0..num_initial_utxos {
        let coinbase = build_bitcoin_coinbase(block_idx as i32, anyone_script.to_p2sh());
        let coinbase = coinbase.hashed();
        let coinbase_txid = coinbase.hash().clone();
        let amount = coinbase.outputs()[0].value;
        let block = build_bitcoin_block(prev_block_hash, timestamp, coinbase, vec![]);
        timestamp += 600;
        prev_block_hash = block.header.calc_hash();
        blocks.push((block, vec![]));
        initial_utxos.push((
            OutPoint {
                txid: coinbase_txid.clone(),
                out_idx: 0,
            },
            amount,
        ));
    }
    // make 100 blocks with each 1 fan-out tx
    let mut counter = 1000u32;
    let mut utxos = Vec::new();
    let num_fan_out_outputs: usize = 100;
    for (prev_out, value) in initial_utxos.into_iter().skip(1) {
        let value = (value - 100_000) / num_fan_out_outputs as i64;
        let scripts = (0..num_fan_out_outputs)
            .into_iter()
            .map(|_| {
                counter += 1;
                script_from_counter(counter)
            })
            .collect::<Vec<_>>();
        let outputs = scripts
            .iter()
            .map(|script| TxOutput {
                script: script.to_p2sh(),
                value,
            })
            .collect::<Vec<_>>();
        let spent_scripts = vec![anyone_script.to_p2sh()];
        let tx = build_tx(prev_out, &anyone_script, outputs);
        let tx = tx.hashed();
        for (out_idx, script) in scripts.into_iter().enumerate() {
            utxos.push((
                OutPoint {
                    txid: tx.hash().clone(),
                    out_idx: out_idx as u32,
                },
                script,
                value,
            ));
        }
        let coinbase = build_bitcoin_coinbase(blocks.len() as i32, anyone_script.to_p2sh());
        let coinbase = coinbase.hashed();
        let block = build_bitcoin_block(prev_block_hash, timestamp, coinbase, vec![tx]);
        timestamp += 600;
        prev_block_hash = block.header.calc_hash();
        blocks.push((block, vec![spent_scripts]));
    }

    println!("generating {} blocks...", num_blocks);
    let script_counter_weights = &[
        // somewhat realistic script distribution
        // last "1000" means unique address
        1000, 200, 50, 10, 5, 4, 3, 2, 1, 1000,
    ];
    let script_counter_dist = WeightedIndex::new(script_counter_weights)?;
    let mut rng = rand::rngs::StdRng::from_seed([42; 32]);
    for i in 0..num_blocks {
        if i % 10 == 0 {
            println!("generated {} blocks, {} outputs", i, counter);
        }
        let num_txs = rng.gen_range(1..3000);
        let mut txs = Vec::with_capacity(num_txs);
        let mut block_spent_outputs = Vec::with_capacity(num_txs);
        for _ in 0..num_txs {
            let num_inputs = rng.gen_range(2..8);
            let num_outputs = rng.gen_range(2..8);
            let mut inputs = Vec::new();
            let mut input_sum = 0;
            let mut spent_scripts = Vec::with_capacity(num_inputs);
            for _ in 0..num_inputs {
                let (prev_out, script, value) = utxos.remove(rng.gen_range(0..utxos.len()));
                inputs.push(TxInput {
                    prev_out,
                    script: Script::new(script.bytecode().ser()),
                    sequence: SequenceNo::finalized(),
                    ..Default::default()
                });
                spent_scripts.push(script.to_p2sh());
                input_sum += value;
            }
            let output_value = (input_sum - 10_000) / num_outputs;
            if output_value < 1000 {
                continue;
            }
            let scripts = (0..num_outputs)
                .into_iter()
                .map(|_| {
                    counter += 1;
                    let script_idx = script_counter_dist.sample(&mut rng);
                    if script_idx == script_counter_weights.len() - 1 {
                        script_from_counter(counter)
                    } else {
                        script_from_counter(script_idx as u32 + 100)
                    }
                })
                .collect::<Vec<_>>();
            let outputs = scripts
                .iter()
                .map(|script| TxOutput {
                    script: script.to_p2sh(),
                    value: output_value,
                })
                .collect::<Vec<_>>();
            let tx = UnhashedTx {
                version: 1,
                inputs,
                outputs,
                lock_time: 0,
            };
            let tx = tx.hashed();
            for (out_idx, script) in scripts.into_iter().enumerate() {
                utxos.push((
                    OutPoint {
                        txid: tx.hash().clone(),
                        out_idx: out_idx as u32,
                    },
                    script,
                    output_value,
                ));
            }
            let txid = tx.hash().clone();
            txs.push(tx);
            block_spent_outputs.push((txid, spent_scripts));
        }
        block_spent_outputs.sort_unstable_by_key(|(txid, _)| txid.clone());
        let block_spent_outputs = block_spent_outputs
            .into_iter()
            .map(|(_, script)| script)
            .collect();
        let coinbase = build_bitcoin_coinbase(blocks.len() as i32, anyone_script.to_p2sh());
        let coinbase = coinbase.hashed();
        let block = build_bitcoin_block(prev_block_hash, timestamp, coinbase, txs);
        timestamp += 600;
        prev_block_hash = block.header.calc_hash();
        blocks.push((block, block_spent_outputs));
    }

    bitcoinsuite_error::install()?;
    println!("Inserting blocks...");
    println!("Cache size: {}", cache_size);
    let dir = TempDir::new("chronik-rocksdb-bench")?;
    let outputs_conf = OutputsConf { page_size: 1000 };
    let db = Db::open(dir.path().join("index.rocksdb"))?;
    let db = IndexDb::new(db, outputs_conf);
    let mut data = IndexMemData::new(cache_size);
    let t = Instant::now();
    for (block_height, (block, block_spent_scripts)) in blocks.iter().enumerate() {
        let db_block = Block {
            hash: block.header.calc_hash(),
            prev_hash: block.header.prev_block.clone(),
            height: block_height as i32,
            n_bits: block.header.bits,
            timestamp: block.header.timestamp.into(),
            file_num: 0,
            data_pos: 0,
        };
        let block_txs = BlockTxs {
            txs: block
                .txs
                .iter()
                .map(|tx| TxEntry {
                    txid: tx.hash().clone(),
                    tx_size: tx.raw().len() as u32,
                    data_pos: 0,
                    undo_pos: 0,
                    undo_size: 0,
                    time_first_seen: 0,
                })
                .collect(),
            block_height: block_height as i32,
        };
        let txs = block
            .txs
            .iter()
            .map(|tx| tx.unhashed_tx().clone())
            .collect::<Vec<_>>();
        db.insert_block(
            &db_block,
            &block_txs,
            &txs,
            |tx_pos, input_idx| &block_spent_scripts[tx_pos][input_idx],
            &mut data,
        )?;
    }
    let dt = t.elapsed();
    println!("Took {:?}", dt);
    let timings = db.timings();
    println!("Overview:");
    println!("{}", timings.timings);
    println!("Outputs:");
    println!("{}", timings.outputs_timings);
    println!("UTXOs:");
    println!("{}", timings.utxos_timings);

    Ok(())
}

fn script_from_counter(counter: u32) -> Script {
    Script::from_ops(
        vec![Op::Push(
            4,
            Bytes::from_bytes(counter.to_be_bytes().to_vec()),
        )]
        .into_iter(),
    )
    .unwrap()
}

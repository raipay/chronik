use std::collections::{BTreeSet, HashMap};

use bitcoinsuite_core::{Bytes, OutPoint, Sha256d, TxOutput, UnhashedTx};
use bitcoinsuite_error::{ErrorMeta, Result};
use thiserror::Error;

use crate::{script_payload::script_payloads, PayloadPrefix};

#[derive(Debug, PartialEq, Eq, Default)]
pub struct MempoolData {
    txs: HashMap<Sha256d, MempoolTxEntry>,
    script_txs: HashMap<Bytes, BTreeSet<(i64, Sha256d)>>,
    utxos: HashMap<Bytes, UtxoDelta>,
    spends: HashMap<Sha256d, BTreeSet<(u32, Sha256d, u32)>>,
}

#[derive(Debug, PartialEq, Eq, Default)]
pub struct MempoolTxEntry {
    pub tx: UnhashedTx,
    pub spent_outputs: Vec<TxOutput>,
    pub time_first_seen: i64,
}

#[derive(Debug, PartialEq, Eq, Default)]
pub struct UtxoDelta {
    pub inserts: BTreeSet<OutPoint>,
    pub deletes: BTreeSet<OutPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MempoolDeleteMode {
    Remove,
    Mined,
}

#[derive(Debug, Error, ErrorMeta)]
pub enum MempoolDataError {
    #[critical()]
    #[error("No such mempool tx: {0}")]
    NoSuchTx(Sha256d),

    #[critical()]
    #[error("UTXO {0:?} already exists in mempool")]
    DuplicateUtxo(OutPoint),

    #[critical()]
    #[error("Tx {0} already exists in mempool")]
    DuplicateTx(Sha256d),

    #[critical()]
    #[error("UTXO {0:?} already spent in mempool")]
    UtxoAlreadySpent(OutPoint),

    #[critical()]
    #[error("Output {0:?} already spent in mempool")]
    OutputAlreadySpent(OutPoint),

    #[critical()]
    #[error("UTXO {0:?} already unspent in mempool")]
    UtxoAlreadyUnspent(OutPoint),

    #[critical()]
    #[error("Output {0:?} already unspent in mempool")]
    OutputAlreadyUnspent(OutPoint),

    #[critical()]
    #[error("UTXO {0:?} doesn't exist in mempool")]
    UtxoDoesntExist(OutPoint),
}

use self::MempoolDataError::*;

impl MempoolData {
    pub fn insert_mempool_tx(
        &mut self,
        txid: Sha256d,
        tx: UnhashedTx,
        spent_outputs: Vec<TxOutput>,
        time_first_seen: i64,
    ) -> Result<()> {
        for (out_idx, output) in tx.outputs.iter().enumerate() {
            let outpoint = OutPoint {
                txid: txid.clone(),
                out_idx: out_idx as u32,
            };
            for (prefix, mut script_payload) in script_payloads(&output.script) {
                script_payload.insert(0, prefix as u8);
                let script_payload = Bytes::from_bytes(script_payload);
                {
                    if !self.script_txs.contains_key(&script_payload) {
                        self.script_txs
                            .insert(script_payload.clone(), BTreeSet::new());
                    }
                    let txs = self
                        .script_txs
                        .get_mut(&script_payload)
                        .expect("Impossible");
                    txs.insert((time_first_seen, txid.clone()));
                }
                {
                    if !self.utxos.contains_key(&script_payload) {
                        self.utxos
                            .insert(script_payload.clone(), UtxoDelta::default());
                    }
                    let delta = self.utxos.get_mut(&script_payload).expect("Impossible");
                    if !delta.inserts.insert(outpoint.clone()) {
                        return Err(DuplicateUtxo(outpoint).into());
                    }
                }
            }
        }
        for (input_idx, (input, spent_output)) in tx.inputs.iter().zip(&spent_outputs).enumerate() {
            for (prefix, mut script_payload) in script_payloads(&spent_output.script) {
                script_payload.insert(0, prefix as u8);
                let script_payload = Bytes::from_bytes(script_payload);
                {
                    if !self.script_txs.contains_key(&script_payload) {
                        self.script_txs
                            .insert(script_payload.clone(), BTreeSet::new());
                    }
                    let txs = self
                        .script_txs
                        .get_mut(&script_payload)
                        .expect("Impossible");
                    txs.insert((time_first_seen, txid.clone()));
                }
                if !self.utxos.contains_key(&script_payload) {
                    self.utxos
                        .insert(script_payload.clone(), UtxoDelta::default());
                }
                let delta = self.utxos.get_mut(&script_payload).expect("Impossible");
                if !delta.inserts.remove(&input.prev_out) {
                    // Only add to deletes if output not in mempool
                    if !delta.deletes.insert(input.prev_out.clone()) {
                        return Err(UtxoAlreadySpent(input.prev_out.clone()).into());
                    }
                }
                if delta.inserts.is_empty() && delta.deletes.is_empty() {
                    self.utxos.remove(&script_payload).unwrap();
                }
            }
            let spends = self.spends.entry(input.prev_out.txid.clone()).or_default();
            if !spends.insert((
                input.prev_out.out_idx as u32,
                txid.clone(),
                input_idx as u32,
            )) {
                return Err(OutputAlreadySpent(input.prev_out.clone()).into());
            }
        }
        let entry = MempoolTxEntry {
            tx,
            spent_outputs,
            time_first_seen,
        };
        if self.txs.insert(txid.clone(), entry).is_some() {
            return Err(DuplicateTx(txid).into());
        }
        Ok(())
    }

    pub fn delete_mempool_tx(&mut self, txid: &Sha256d, mode: MempoolDeleteMode) -> Result<()> {
        let MempoolTxEntry {
            tx,
            spent_outputs,
            time_first_seen,
        } = match self.txs.remove(txid) {
            Some(entry) => entry,
            None => return Err(NoSuchTx(txid.clone()).into()),
        };
        for (input_idx, (input, spent_output)) in tx.inputs.iter().zip(&spent_outputs).enumerate() {
            for (prefix, mut script_payload) in script_payloads(&spent_output.script) {
                script_payload.insert(0, prefix as u8);
                let script_payload = Bytes::from_bytes(script_payload);
                if let Some(txs) = self.script_txs.get_mut(&script_payload) {
                    txs.remove(&(time_first_seen, txid.clone()));
                    if txs.is_empty() {
                        self.script_txs.remove(&script_payload);
                    }
                }
                if !self.utxos.contains_key(&script_payload) {
                    self.utxos
                        .insert(script_payload.clone(), UtxoDelta::default());
                }
                let delta = self.utxos.get_mut(&script_payload).expect("Impossible");
                if !delta.deletes.remove(&input.prev_out)
                    && !delta.inserts.insert(input.prev_out.clone())
                {
                    return Err(UtxoAlreadyUnspent(input.prev_out.clone()).into());
                }
                if delta.inserts.is_empty() && delta.deletes.is_empty() {
                    self.utxos.remove(&script_payload);
                }
            }
            let make_err = || OutputAlreadyUnspent(input.prev_out.clone());
            let spends = self
                .spends
                .get_mut(&input.prev_out.txid)
                .ok_or_else(&make_err)?;
            if !spends.remove(&(
                input.prev_out.out_idx as u32,
                txid.clone(),
                input_idx as u32,
            )) {
                return Err(make_err().into());
            }
            if spends.is_empty() {
                self.spends.remove(&input.prev_out.txid);
            }
        }
        for (out_idx, output) in tx.outputs.iter().enumerate() {
            let outpoint = OutPoint {
                txid: txid.clone(),
                out_idx: out_idx as u32,
            };
            for (prefix, mut script_payload) in script_payloads(&output.script) {
                script_payload.insert(0, prefix as u8);
                let script_payload = Bytes::from_bytes(script_payload);
                if let Some(txs) = self.script_txs.get_mut(&script_payload) {
                    txs.remove(&(time_first_seen, txid.clone()));
                    if txs.is_empty() {
                        self.script_txs.remove(&script_payload);
                    }
                }
                let delta = match mode {
                    MempoolDeleteMode::Remove => {
                        let delta = self
                            .utxos
                            .get_mut(&script_payload)
                            .ok_or_else(|| UtxoDoesntExist(outpoint.clone()))?;
                        if !delta.inserts.remove(&outpoint) {
                            return Err(UtxoDoesntExist(outpoint).into());
                        }
                        delta
                    }
                    MempoolDeleteMode::Mined => {
                        if !self.utxos.contains_key(&script_payload) {
                            self.utxos
                                .insert(script_payload.clone(), UtxoDelta::default());
                        }
                        let delta = self.utxos.get_mut(&script_payload).expect("Impossible");
                        if !delta.inserts.remove(&outpoint)
                            && !delta.deletes.insert(outpoint.clone())
                        {
                            return Err(UtxoAlreadySpent(outpoint.clone()).into());
                        }
                        delta
                    }
                };
                if delta.inserts.is_empty() && delta.deletes.is_empty() {
                    self.utxos.remove(&script_payload);
                }
            }
        }
        Ok(())
    }

    pub fn tx(&self, txid: &Sha256d) -> Option<&MempoolTxEntry> {
        self.txs.get(txid)
    }

    pub fn script_txs(
        &self,
        prefix: PayloadPrefix,
        payload: &[u8],
    ) -> Option<&BTreeSet<(i64, Sha256d)>> {
        let script_payload = [[prefix as u8].as_ref(), payload].concat();
        self.script_txs.get(script_payload.as_slice())
    }

    pub fn utxos(&self, prefix: PayloadPrefix, payload: &[u8]) -> Option<&UtxoDelta> {
        let script_payload = [[prefix as u8].as_ref(), payload].concat();
        self.utxos.get(script_payload.as_slice())
    }

    pub fn spends(&self, txid: &Sha256d) -> Option<&BTreeSet<(u32, Sha256d, u32)>> {
        self.spends.get(txid)
    }
}

impl UtxoDelta {
    pub fn inserts(&self) -> &BTreeSet<OutPoint> {
        &self.inserts
    }

    pub fn deletes(&self) -> &BTreeSet<OutPoint> {
        &self.deletes
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use bitcoinsuite_core::{
        ecc::PubKey, OutPoint, Script, Sha256d, ShaRmd160, TxInput, TxOutput, UnhashedTx,
    };
    use bitcoinsuite_error::Result;
    use pretty_assertions::assert_eq;

    use crate::{
        mempool_data::UtxoDelta, MempoolData, MempoolDeleteMode, MempoolTxEntry, PayloadPrefix,
    };

    #[test]
    fn test_mempool_data() -> Result<()> {
        use PayloadPrefix::*;
        bitcoinsuite_error::install()?;
        let mut mempool = MempoolData::default();
        let (script1, payload1) = (Script::p2pkh(&ShaRmd160::new([1; 20])), [1; 20]);
        let (script2, payload2) = (Script::p2pkh(&ShaRmd160::new([2; 20])), [2; 20]);
        let (script3, payload3) = (Script::p2sh(&ShaRmd160::new([3; 20])), [3; 20]);
        let (script4, payload4) = (Script::p2sh(&ShaRmd160::new([4; 20])), [4; 20]);
        let (script5, payload5) = (Script::p2pk(&PubKey::new_unchecked([5; 33])), [5; 33]);
        let (script6, payload6) = (Script::p2tr(&PubKey::new_unchecked([6; 33]), None), [6; 33]);
        let (script7, payload7, payload8) = (
            Script::p2tr(&PubKey::new_unchecked([7; 33]), Some([8; 32])),
            [7; 33],
            [8; 32],
        );

        let txid0 = make_hash(1);

        // Add tx 1, spending confirmed UTXOs
        let txid1 = make_hash(10);
        let tx1 = make_tx([(1, 4)], [&script2]);
        let spent_scripts1 = vec![script1.clone()];
        mempool.insert_mempool_tx(txid1.clone(), tx1.clone(), make_spents(&spent_scripts1), 90)?;
        check_tx(&mempool, &txid1, &tx1, &spent_scripts1, 90);
        check_outputs(&mempool, P2PKH, &payload2, [(90, &txid1)]);
        check_utxos(&mempool, P2PKH, &payload1, [], [(&txid0, 4)]);
        check_utxos(&mempool, P2PKH, &payload2, [(&txid1, 0)], []);
        check_spends(&mempool, &txid0, [(4, &txid1, 0)]);

        // Add tx 2, spending both confirmed and unconfirmed UTXOs
        let txid2 = make_hash(11);
        let tx2 = make_tx([(1, 5), (10, 0)], [&script1, &script4]);
        let spent_scripts2 = vec![script3, script2];
        mempool.insert_mempool_tx(txid2.clone(), tx2.clone(), make_spents(&spent_scripts2), 91)?;
        check_tx(&mempool, &txid2, &tx2, &spent_scripts2, 91);
        check_outputs(&mempool, P2PKH, &payload1, [(90, &txid1), (91, &txid2)]);
        check_outputs(&mempool, P2PKH, &payload2, [(90, &txid1), (91, &txid2)]);
        check_outputs(&mempool, P2SH, &payload3, [(91, &txid2)]);
        check_outputs(&mempool, P2SH, &payload4, [(91, &txid2)]);
        check_utxos(&mempool, P2PKH, &payload1, [(&txid2, 0)], [(&txid0, 4)]);
        check_utxos_absent(&mempool, P2PKH, &payload2);
        check_utxos(&mempool, P2SH, &payload3, [], [(&txid0, 5)]);
        check_utxos(&mempool, P2SH, &payload4, [(&txid2, 1)], []);
        check_spends(&mempool, &txid0, [(4, &txid1, 0), (5, &txid2, 0)]);
        check_spends(&mempool, &txid1, [(0, &txid2, 1)]);
        check_spends_absent(&mempool, &txid2);

        // Remove tx 2
        mempool.delete_mempool_tx(&txid2, MempoolDeleteMode::Remove)?;
        check_tx(&mempool, &txid1, &tx1, &spent_scripts1, 90);
        check_tx_absent(&mempool, &txid2);
        check_outputs(&mempool, P2PKH, &payload1, [(90, &txid1)]);
        check_outputs(&mempool, P2PKH, &payload2, [(90, &txid1)]);
        check_outputs_absent(&mempool, P2SH, &payload3);
        check_outputs_absent(&mempool, P2SH, &payload4);
        check_utxos(&mempool, P2PKH, &payload1, [], [(&txid0, 4)]);
        check_utxos(&mempool, P2PKH, &payload2, [(&txid1, 0)], []);
        check_utxos_absent(&mempool, P2SH, &payload3);
        check_utxos_absent(&mempool, P2SH, &payload4);
        check_spends(&mempool, &txid0, [(4, &txid1, 0)]);
        check_spends_absent(&mempool, &txid2);

        mempool.delete_mempool_tx(&txid1, MempoolDeleteMode::Remove)?;
        assert_eq!(mempool, MempoolData::default());

        // Add txs back in
        mempool.insert_mempool_tx(txid1.clone(), tx1.clone(), make_spents(&spent_scripts1), 90)?;
        mempool.insert_mempool_tx(txid2.clone(), tx2.clone(), make_spents(&spent_scripts2), 91)?;

        // Add tx 3
        let txid3 = make_hash(12);
        let tx3 = make_tx([(1, 6), (1, 7), (11, 0)], [&script5, &script6]);
        let spent_scripts3 = vec![script7, script1.clone(), script1];
        mempool.insert_mempool_tx(txid3.clone(), tx3.clone(), make_spents(&spent_scripts3), 92)?;
        check_tx(&mempool, &txid3, &tx3, &spent_scripts3, 92);
        check_outputs(
            &mempool,
            P2PKH,
            &payload1,
            [(90, &txid1), (91, &txid2), (92, &txid3)],
        );
        check_outputs(&mempool, P2PKH, &payload2, [(90, &txid1), (91, &txid2)]);
        check_outputs(&mempool, P2SH, &payload3, [(91, &txid2)]);
        check_outputs(&mempool, P2SH, &payload4, [(91, &txid2)]);
        check_outputs(&mempool, P2PK, &payload5, [(92, &txid3)]);
        check_outputs(&mempool, P2TRCommitment, &payload6, [(92, &txid3)]);
        check_outputs(&mempool, P2TRCommitment, &payload7, [(92, &txid3)]);
        check_utxos(&mempool, P2PKH, &payload1, [], [(&txid0, 4), (&txid0, 7)]);
        check_utxos_absent(&mempool, P2PKH, &payload2);
        check_utxos(&mempool, P2SH, &payload3, [], [(&txid0, 5)]);
        check_utxos(&mempool, P2SH, &payload4, [(&txid2, 1)], []);
        check_utxos(&mempool, P2PK, &payload5, [(&txid3, 0)], []);
        check_utxos(&mempool, P2TRCommitment, &payload6, [(&txid3, 1)], []);
        check_utxos_absent(&mempool, P2TRState, &payload6);
        check_utxos(&mempool, P2TRCommitment, &payload7, [], [(&txid0, 6)]);
        check_utxos(&mempool, P2TRState, &payload8, [], [(&txid0, 6)]);
        check_spends(
            &mempool,
            &txid0,
            [
                (4, &txid1, 0),
                (5, &txid2, 0),
                (6, &txid3, 0),
                (7, &txid3, 1),
            ],
        );
        check_spends(&mempool, &txid1, [(0, &txid2, 1)]);
        check_spends(&mempool, &txid2, [(0, &txid3, 2)]);
        check_spends_absent(&mempool, &txid3);

        // Delete txs in mempool eviction order
        mempool.delete_mempool_tx(&txid3, MempoolDeleteMode::Remove)?;
        mempool.delete_mempool_tx(&txid2, MempoolDeleteMode::Remove)?;
        mempool.delete_mempool_tx(&txid1, MempoolDeleteMode::Remove)?;
        assert_eq!(mempool, MempoolData::default());

        // Add txs back in
        mempool.insert_mempool_tx(txid1.clone(), tx1, make_spents(&spent_scripts1), 90)?;
        mempool.insert_mempool_tx(txid2.clone(), tx2.clone(), make_spents(&spent_scripts2), 91)?;
        mempool.insert_mempool_tx(txid3.clone(), tx3.clone(), make_spents(&spent_scripts3), 92)?;

        // Delete txs in block mined order
        mempool.delete_mempool_tx(&txid1, MempoolDeleteMode::Mined)?;
        check_tx_absent(&mempool, &txid1);
        check_tx(&mempool, &txid2, &tx2, &spent_scripts2, 91);
        check_tx(&mempool, &txid3, &tx3, &spent_scripts3, 92);
        check_outputs(&mempool, P2PKH, &payload1, [(91, &txid2), (92, &txid3)]);
        check_outputs(&mempool, P2PKH, &payload2, [(91, &txid2)]);
        check_outputs(&mempool, P2SH, &payload3, [(91, &txid2)]);
        check_outputs(&mempool, P2SH, &payload4, [(91, &txid2)]);
        check_outputs(&mempool, P2PK, &payload5, [(92, &txid3)]);
        check_outputs(&mempool, P2TRCommitment, &payload6, [(92, &txid3)]);
        check_utxos(&mempool, P2PKH, &payload1, [], [(&txid0, 7)]);
        check_utxos(&mempool, P2PKH, &payload2, [], [(&txid1, 0)]);
        check_utxos(&mempool, P2SH, &payload3, [], [(&txid0, 5)]);
        check_utxos(&mempool, P2SH, &payload4, [(&txid2, 1)], []);
        check_utxos(&mempool, P2PK, &payload5, [(&txid3, 0)], []);
        check_utxos(&mempool, P2TRCommitment, &payload6, [(&txid3, 1)], []);
        check_utxos_absent(&mempool, P2TRState, &payload6);
        check_utxos(&mempool, P2TRCommitment, &payload7, [], [(&txid0, 6)]);
        check_utxos(&mempool, P2TRState, &payload8, [], [(&txid0, 6)]);
        check_spends(
            &mempool,
            &txid0,
            [(5, &txid2, 0), (6, &txid3, 0), (7, &txid3, 1)],
        );
        check_spends(&mempool, &txid1, [(0, &txid2, 1)]);
        check_spends(&mempool, &txid2, [(0, &txid3, 2)]);

        mempool.delete_mempool_tx(&txid2, MempoolDeleteMode::Mined)?;
        check_tx_absent(&mempool, &txid1);
        check_tx_absent(&mempool, &txid2);
        check_outputs(&mempool, P2PKH, &payload1, [(92, &txid3)]);
        check_outputs_absent(&mempool, P2PKH, &payload2);
        check_outputs_absent(&mempool, P2SH, &payload3);
        check_outputs_absent(&mempool, P2SH, &payload4);
        check_outputs(&mempool, P2PK, &payload5, [(92, &txid3)]);
        check_outputs(&mempool, P2TRCommitment, &payload6, [(92, &txid3)]);
        check_utxos(&mempool, P2PKH, &payload1, [], [(&txid0, 7), (&txid2, 0)]);
        check_utxos_absent(&mempool, P2PKH, &payload2);
        check_utxos_absent(&mempool, P2SH, &payload3);
        check_utxos_absent(&mempool, P2SH, &payload4);
        check_utxos(&mempool, P2PK, &payload5, [(&txid3, 0)], []);
        check_utxos(&mempool, P2TRCommitment, &payload6, [(&txid3, 1)], []);
        check_utxos_absent(&mempool, P2TRState, &payload6);
        check_utxos(&mempool, P2TRCommitment, &payload7, [], [(&txid0, 6)]);
        check_utxos(&mempool, P2TRState, &payload8, [], [(&txid0, 6)]);
        check_spends(&mempool, &txid0, [(6, &txid3, 0), (7, &txid3, 1)]);
        check_spends_absent(&mempool, &txid1);
        check_spends(&mempool, &txid2, [(0, &txid3, 2)]);

        mempool.delete_mempool_tx(&txid3, MempoolDeleteMode::Mined)?;
        assert_eq!(mempool, MempoolData::default());

        Ok(())
    }

    fn check_tx(
        mempool: &MempoolData,
        txid: &Sha256d,
        expectd_tx: &UnhashedTx,
        spent_scripts: &[Script],
        time_first_seen: i64,
    ) {
        assert_eq!(
            mempool.txs.get(txid),
            Some(&MempoolTxEntry {
                tx: expectd_tx.clone(),
                spent_outputs: make_spents(spent_scripts),
                time_first_seen,
            }),
        );
    }

    fn check_tx_absent(mempool: &MempoolData, txid: &Sha256d) {
        assert_eq!(mempool.txs.get(txid), None);
    }

    fn check_outputs<const N: usize>(
        mempool: &MempoolData,
        prefix: PayloadPrefix,
        payload: &[u8],
        expected_outpoints: [(i64, &Sha256d); N],
    ) {
        let expected_outpoints = expected_outpoints
            .into_iter()
            .map(|(time, txid)| (time, txid.clone()))
            .collect::<BTreeSet<_>>();
        let script_payload = [[prefix as u8].as_ref(), payload].concat();
        assert_eq!(
            mempool.script_txs.get(script_payload.as_slice()),
            Some(&expected_outpoints),
        );
    }

    fn check_outputs_absent(mempool: &MempoolData, prefix: PayloadPrefix, payload: &[u8]) {
        let script_payload = [[prefix as u8].as_ref(), payload].concat();
        assert_eq!(mempool.script_txs.get(script_payload.as_slice()), None);
    }

    fn check_utxos<const N: usize, const M: usize>(
        mempool: &MempoolData,
        prefix: PayloadPrefix,
        payload: &[u8],
        expected_inserts: [(&Sha256d, u32); N],
        expected_deletes: [(&Sha256d, u32); M],
    ) {
        let expected_inserts = expected_inserts
            .into_iter()
            .map(|(txid, out_idx)| OutPoint {
                txid: txid.clone(),
                out_idx,
            })
            .collect::<BTreeSet<_>>();
        let expected_deletes = expected_deletes
            .into_iter()
            .map(|(txid, out_idx)| OutPoint {
                txid: txid.clone(),
                out_idx,
            })
            .collect::<BTreeSet<_>>();
        let script_payload = [[prefix as u8].as_ref(), payload].concat();
        assert_eq!(
            mempool.utxos.get(script_payload.as_slice()),
            Some(&UtxoDelta {
                inserts: expected_inserts,
                deletes: expected_deletes,
            }),
        );
    }

    fn check_utxos_absent(mempool: &MempoolData, prefix: PayloadPrefix, payload: &[u8]) {
        let script_payload = [[prefix as u8].as_ref(), payload].concat();
        assert_eq!(mempool.utxos.get(script_payload.as_slice()), None);
    }

    fn check_spends<const N: usize>(
        mempool: &MempoolData,
        txid: &Sha256d,
        expected_spends: [(u32, &Sha256d, u32); N],
    ) {
        let expected_spends = expected_spends
            .into_iter()
            .map(|(out_idx, spending_txid, spending_input_idx)| {
                (out_idx, spending_txid.clone(), spending_input_idx)
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(mempool.spends.get(txid), Some(&expected_spends));
    }

    fn check_spends_absent(mempool: &MempoolData, txid: &Sha256d) {
        assert_eq!(mempool.spends.get(txid), None);
    }

    fn make_tx<const N: usize, const M: usize>(
        inputs: [(u8, u32); N],
        outputs: [&Script; M],
    ) -> UnhashedTx {
        UnhashedTx {
            version: 1,
            inputs: inputs
                .iter()
                .map(|&(input_byte, out_idx)| TxInput {
                    prev_out: OutPoint {
                        txid: make_hash(input_byte),
                        out_idx,
                    },
                    ..Default::default()
                })
                .collect(),
            outputs: outputs
                .into_iter()
                .map(|script| TxOutput {
                    script: script.clone(),
                    value: 0,
                })
                .collect(),
            lock_time: 0,
        }
    }

    fn make_hash(byte: u8) -> Sha256d {
        let mut hash = [0; 32];
        hash[31] = byte;
        Sha256d::new(hash)
    }

    fn make_spents(scripts: &[Script]) -> Vec<TxOutput> {
        scripts
            .iter()
            .map(|script| TxOutput {
                value: 0,
                script: script.clone(),
            })
            .collect()
    }
}

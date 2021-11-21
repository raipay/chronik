use bitcoinsuite_core::{Hashed, Script, ScriptVariant};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum PayloadPrefix {
    Other = 0,
    P2PK = 1,
    P2PKLegacy = 2,
    P2PKH = 3,
    P2SH = 4,
    P2TRCommitment = 5,
    P2TRState = 6,
}

pub fn script_payloads(script: &Script) -> Vec<(PayloadPrefix, Vec<u8>)> {
    use PayloadPrefix::*;
    match script.parse_variant() {
        ScriptVariant::P2PK(pubkey) => {
            vec![(P2PK, pubkey.as_slice().to_vec())]
        }
        ScriptVariant::P2PKLegacy(pubkey) => {
            vec![(P2PKLegacy, pubkey.to_vec())]
        }
        ScriptVariant::P2PKH(hash) => vec![(P2PKH, hash.as_slice().to_vec())],
        ScriptVariant::P2SH(hash) => vec![(P2SH, hash.as_slice().to_vec())],
        ScriptVariant::P2TR(commitment, None) => {
            vec![(P2TRCommitment, commitment.as_slice().to_vec())]
        }
        ScriptVariant::P2TR(commitment, Some(state)) => vec![
            (P2TRCommitment, commitment.as_slice().to_vec()),
            (P2TRState, state.to_vec()),
        ],
        ScriptVariant::Other(script) => match script.is_opreturn() {
            true => vec![],
            false => vec![(Other, script.bytecode().to_vec())],
        },
    }
}

use bitcoinsuite_core::{ecc::PubKey, Hashed, Script, ScriptVariant, ShaRmd160};

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

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScriptPayload {
    pub payload_prefix: PayloadPrefix,
    pub payload_data: Vec<u8>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScriptPayloadState {
    pub payload: ScriptPayload,
    /// Whether `payload` is a complete representation of the `Script` it's based on or whether
    /// it's only a subset.
    pub is_partial: bool,
}

pub fn script_payloads(script: &Script) -> Vec<ScriptPayloadState> {
    use PayloadPrefix::*;
    match script.parse_variant() {
        ScriptVariant::P2PK(pubkey) => {
            vec![ScriptPayloadState {
                payload: ScriptPayload {
                    payload_prefix: P2PK,
                    payload_data: pubkey.as_slice().to_vec(),
                },
                is_partial: false,
            }]
        }
        ScriptVariant::P2PKLegacy(pubkey) => {
            vec![ScriptPayloadState {
                payload: ScriptPayload {
                    payload_prefix: P2PKLegacy,
                    payload_data: pubkey.to_vec(),
                },
                is_partial: false,
            }]
        }
        ScriptVariant::P2PKH(hash) => vec![ScriptPayloadState {
            payload: ScriptPayload {
                payload_prefix: P2PKH,
                payload_data: hash.as_slice().to_vec(),
            },
            is_partial: false,
        }],
        ScriptVariant::P2SH(hash) => vec![ScriptPayloadState {
            payload: ScriptPayload {
                payload_prefix: P2SH,
                payload_data: hash.as_slice().to_vec(),
            },
            is_partial: false,
        }],
        ScriptVariant::P2TR(commitment, None) => {
            vec![ScriptPayloadState {
                payload: ScriptPayload {
                    payload_prefix: P2TRCommitment,
                    payload_data: commitment.as_slice().to_vec(),
                },
                is_partial: false,
            }]
        }
        ScriptVariant::P2TR(commitment, Some(state)) => vec![
            ScriptPayloadState {
                payload: ScriptPayload {
                    payload_prefix: P2TRCommitment,
                    payload_data: commitment.as_slice().to_vec(),
                },
                is_partial: true,
            },
            ScriptPayloadState {
                payload: ScriptPayload {
                    payload_prefix: P2TRState,
                    payload_data: state.to_vec(),
                },
                is_partial: true,
            },
        ],
        ScriptVariant::Other(script) => match script.is_opreturn() {
            true => vec![],
            false => vec![ScriptPayloadState {
                payload: ScriptPayload {
                    payload_prefix: Other,
                    payload_data: script.bytecode().to_vec(),
                },
                is_partial: false,
            }],
        },
    }
}

impl ScriptPayload {
    pub fn into_vec(self) -> Vec<u8> {
        let mut script_payload = self.payload_data;
        script_payload.insert(0, self.payload_prefix as u8);
        script_payload
    }

    pub fn reconstruct_script(&self) -> Option<Script> {
        let data = self.payload_data.as_slice();
        Some(match self.payload_prefix {
            PayloadPrefix::Other => Script::from_slice(data),
            PayloadPrefix::P2PK => Script::p2pk(&PubKey::new_unchecked(data.try_into().ok()?)),
            PayloadPrefix::P2PKLegacy => Script::p2pk_legacy(data.try_into().ok()?),
            PayloadPrefix::P2PKH => Script::p2pkh(&ShaRmd160::from_slice(data).ok()?),
            PayloadPrefix::P2SH => Script::p2sh(&ShaRmd160::from_slice(data).ok()?),
            PayloadPrefix::P2TRCommitment => {
                Script::p2tr(&PubKey::new_unchecked(data.try_into().ok()?), None)
            }
            PayloadPrefix::P2TRState => return None,
        })
    }
}

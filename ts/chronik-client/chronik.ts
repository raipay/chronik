/* eslint-disable */
import Long from "long"
import _m0 from "protobufjs/minimal"

export const protobufPackage = "chronik"

export enum SlpTokenType {
  FUNGIBLE = 0,
  NFT1_GROUP = 1,
  NFT1_CHILD = 2,
  UNKNOWN_TOKEN_TYPE = 3,
  UNRECOGNIZED = -1,
}

export function slpTokenTypeFromJSON(object: any): SlpTokenType {
  switch (object) {
    case 0:
    case "FUNGIBLE":
      return SlpTokenType.FUNGIBLE
    case 1:
    case "NFT1_GROUP":
      return SlpTokenType.NFT1_GROUP
    case 2:
    case "NFT1_CHILD":
      return SlpTokenType.NFT1_CHILD
    case 3:
    case "UNKNOWN_TOKEN_TYPE":
      return SlpTokenType.UNKNOWN_TOKEN_TYPE
    case -1:
    case "UNRECOGNIZED":
    default:
      return SlpTokenType.UNRECOGNIZED
  }
}

export function slpTokenTypeToJSON(object: SlpTokenType): string {
  switch (object) {
    case SlpTokenType.FUNGIBLE:
      return "FUNGIBLE"
    case SlpTokenType.NFT1_GROUP:
      return "NFT1_GROUP"
    case SlpTokenType.NFT1_CHILD:
      return "NFT1_CHILD"
    case SlpTokenType.UNKNOWN_TOKEN_TYPE:
      return "UNKNOWN_TOKEN_TYPE"
    default:
      return "UNKNOWN"
  }
}

export enum SlpTxType {
  GENESIS = 0,
  SEND = 1,
  MINT = 2,
  UNKNOWN_TX_TYPE = 3,
  UNRECOGNIZED = -1,
}

export function slpTxTypeFromJSON(object: any): SlpTxType {
  switch (object) {
    case 0:
    case "GENESIS":
      return SlpTxType.GENESIS
    case 1:
    case "SEND":
      return SlpTxType.SEND
    case 2:
    case "MINT":
      return SlpTxType.MINT
    case 3:
    case "UNKNOWN_TX_TYPE":
      return SlpTxType.UNKNOWN_TX_TYPE
    case -1:
    case "UNRECOGNIZED":
    default:
      return SlpTxType.UNRECOGNIZED
  }
}

export function slpTxTypeToJSON(object: SlpTxType): string {
  switch (object) {
    case SlpTxType.GENESIS:
      return "GENESIS"
    case SlpTxType.SEND:
      return "SEND"
    case SlpTxType.MINT:
      return "MINT"
    case SlpTxType.UNKNOWN_TX_TYPE:
      return "UNKNOWN_TX_TYPE"
    default:
      return "UNKNOWN"
  }
}

export enum Network {
  BCH = 0,
  XEC = 1,
  XPI = 2,
  XRG = 3,
  UNRECOGNIZED = -1,
}

export function networkFromJSON(object: any): Network {
  switch (object) {
    case 0:
    case "BCH":
      return Network.BCH
    case 1:
    case "XEC":
      return Network.XEC
    case 2:
    case "XPI":
      return Network.XPI
    case 3:
    case "XRG":
      return Network.XRG
    case -1:
    case "UNRECOGNIZED":
    default:
      return Network.UNRECOGNIZED
  }
}

export function networkToJSON(object: Network): string {
  switch (object) {
    case Network.BCH:
      return "BCH"
    case Network.XEC:
      return "XEC"
    case Network.XPI:
      return "XPI"
    case Network.XRG:
      return "XRG"
    default:
      return "UNKNOWN"
  }
}

export enum UtxoStateVariant {
  UNSPENT = 0,
  SPENT = 1,
  NO_SUCH_TX = 2,
  NO_SUCH_OUTPUT = 3,
  UNRECOGNIZED = -1,
}

export function utxoStateVariantFromJSON(object: any): UtxoStateVariant {
  switch (object) {
    case 0:
    case "UNSPENT":
      return UtxoStateVariant.UNSPENT
    case 1:
    case "SPENT":
      return UtxoStateVariant.SPENT
    case 2:
    case "NO_SUCH_TX":
      return UtxoStateVariant.NO_SUCH_TX
    case 3:
    case "NO_SUCH_OUTPUT":
      return UtxoStateVariant.NO_SUCH_OUTPUT
    case -1:
    case "UNRECOGNIZED":
    default:
      return UtxoStateVariant.UNRECOGNIZED
  }
}

export function utxoStateVariantToJSON(object: UtxoStateVariant): string {
  switch (object) {
    case UtxoStateVariant.UNSPENT:
      return "UNSPENT"
    case UtxoStateVariant.SPENT:
      return "SPENT"
    case UtxoStateVariant.NO_SUCH_TX:
      return "NO_SUCH_TX"
    case UtxoStateVariant.NO_SUCH_OUTPUT:
      return "NO_SUCH_OUTPUT"
    default:
      return "UNKNOWN"
  }
}

export interface ValidateUtxoRequest {
  outpoints: OutPoint[]
}

export interface ValidateUtxoResponse {
  utxoStates: UtxoState[]
}

export interface BroadcastTxRequest {
  rawTx: Uint8Array
  skipSlpCheck: boolean
}

export interface BroadcastTxResponse {
  txid: Uint8Array
}

export interface Tx {
  txid: Uint8Array
  version: number
  inputs: TxInput[]
  outputs: TxOutput[]
  lockTime: number
  slpTxData: SlpTxData | undefined
  slpErrorMsg: string
  block: BlockMetadata | undefined
  timeFirstSeen: Long
  network: Network
}

export interface Utxo {
  outpoint: OutPoint | undefined
  blockHeight: number
  isCoinbase: boolean
  value: Long
  slpMeta: SlpMeta | undefined
  slpToken: SlpToken | undefined
  network: Network
}

export interface BlockInfo {
  hash: Uint8Array
  prevHash: Uint8Array
  height: number
  nBits: number
  timestamp: Long
  /** Block size of this block in bytes (including headers etc.) */
  blockSize: Long
  /** Number of txs in this block */
  numTxs: Long
  /** Total number of tx inputs in block (including coinbase) */
  numInputs: Long
  /** Total number of tx output in block (including coinbase) */
  numOutputs: Long
  /** Total number of satoshis spent by tx inputs */
  sumInputSats: Long
  /** Block reward for this block */
  sumCoinbaseOutputSats: Long
  /** Total number of satoshis in non-coinbase tx outputs */
  sumNormalOutputSats: Long
  /** Total number of satoshis burned using OP_RETURN */
  sumBurnedSats: Long
}

export interface Block {
  blockInfo: BlockInfo | undefined
  txs: Tx[]
}

export interface ScriptUtxos {
  outputScript: Uint8Array
  utxos: Utxo[]
}

export interface TxHistoryPage {
  txs: Tx[]
  numPages: number
}

export interface Utxos {
  scriptUtxos: ScriptUtxos[]
}

export interface Blocks {
  blocks: BlockInfo[]
}

export interface SlpTxData {
  slpMeta: SlpMeta | undefined
  genesisInfo: SlpGenesisInfo | undefined
}

export interface SlpMeta {
  tokenType: SlpTokenType
  txType: SlpTxType
  tokenId: Uint8Array
  groupTokenId: Uint8Array
}

export interface TxInput {
  prevOut: OutPoint | undefined
  inputScript: Uint8Array
  outputScript: Uint8Array
  value: Long
  sequenceNo: number
  slpBurn: SlpBurn | undefined
  slpToken: SlpToken | undefined
}

export interface TxOutput {
  value: Long
  outputScript: Uint8Array
  slpToken: SlpToken | undefined
  spentBy: OutPoint | undefined
}

export interface BlockMetadata {
  height: number
  hash: Uint8Array
  timestamp: Long
}

export interface OutPoint {
  txid: Uint8Array
  outIdx: number
}

export interface SlpToken {
  amount: Long
  isMintBaton: boolean
}

export interface SlpBurn {
  token: SlpToken | undefined
  tokenId: Uint8Array
}

export interface SlpGenesisInfo {
  tokenTicker: Uint8Array
  tokenName: Uint8Array
  tokenDocumentUrl: Uint8Array
  tokenDocumentHash: Uint8Array
  decimals: number
}

export interface UtxoState {
  height: number
  isConfirmed: boolean
  state: UtxoStateVariant
}

export interface Subscription {
  scriptType: string
  payload: Uint8Array
  isSubscribe: boolean
}

export interface SubscribeMsg {
  error: Error | undefined
  AddedToMempool: MsgAddedToMempool | undefined
  RemovedFromMempool: MsgRemovedFromMempool | undefined
  Confirmed: MsgConfirmed | undefined
  Reorg: MsgReorg | undefined
}

export interface MsgAddedToMempool {
  txid: Uint8Array
}

export interface MsgRemovedFromMempool {
  txid: Uint8Array
}

export interface MsgConfirmed {
  txid: Uint8Array
}

export interface MsgReorg {
  txid: Uint8Array
}

export interface Error {
  errorCode: string
  msg: string
  isUserError: boolean
}

const baseValidateUtxoRequest: object = {}

export const ValidateUtxoRequest = {
  encode(
    message: ValidateUtxoRequest,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    for (const v of message.outpoints) {
      OutPoint.encode(v!, writer.uint32(10).fork()).ldelim()
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): ValidateUtxoRequest {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseValidateUtxoRequest } as ValidateUtxoRequest
    message.outpoints = []
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.outpoints.push(OutPoint.decode(reader, reader.uint32()))
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): ValidateUtxoRequest {
    const message = { ...baseValidateUtxoRequest } as ValidateUtxoRequest
    message.outpoints = (object.outpoints ?? []).map((e: any) =>
      OutPoint.fromJSON(e),
    )
    return message
  },

  toJSON(message: ValidateUtxoRequest): unknown {
    const obj: any = {}
    if (message.outpoints) {
      obj.outpoints = message.outpoints.map(e =>
        e ? OutPoint.toJSON(e) : undefined,
      )
    } else {
      obj.outpoints = []
    }
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<ValidateUtxoRequest>, I>>(
    object: I,
  ): ValidateUtxoRequest {
    const message = { ...baseValidateUtxoRequest } as ValidateUtxoRequest
    message.outpoints =
      object.outpoints?.map(e => OutPoint.fromPartial(e)) || []
    return message
  },
}

const baseValidateUtxoResponse: object = {}

export const ValidateUtxoResponse = {
  encode(
    message: ValidateUtxoResponse,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    for (const v of message.utxoStates) {
      UtxoState.encode(v!, writer.uint32(10).fork()).ldelim()
    }
    return writer
  },

  decode(
    input: _m0.Reader | Uint8Array,
    length?: number,
  ): ValidateUtxoResponse {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseValidateUtxoResponse } as ValidateUtxoResponse
    message.utxoStates = []
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.utxoStates.push(UtxoState.decode(reader, reader.uint32()))
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): ValidateUtxoResponse {
    const message = { ...baseValidateUtxoResponse } as ValidateUtxoResponse
    message.utxoStates = (object.utxoStates ?? []).map((e: any) =>
      UtxoState.fromJSON(e),
    )
    return message
  },

  toJSON(message: ValidateUtxoResponse): unknown {
    const obj: any = {}
    if (message.utxoStates) {
      obj.utxoStates = message.utxoStates.map(e =>
        e ? UtxoState.toJSON(e) : undefined,
      )
    } else {
      obj.utxoStates = []
    }
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<ValidateUtxoResponse>, I>>(
    object: I,
  ): ValidateUtxoResponse {
    const message = { ...baseValidateUtxoResponse } as ValidateUtxoResponse
    message.utxoStates =
      object.utxoStates?.map(e => UtxoState.fromPartial(e)) || []
    return message
  },
}

const baseBroadcastTxRequest: object = { skipSlpCheck: false }

export const BroadcastTxRequest = {
  encode(
    message: BroadcastTxRequest,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.rawTx.length !== 0) {
      writer.uint32(10).bytes(message.rawTx)
    }
    if (message.skipSlpCheck === true) {
      writer.uint32(16).bool(message.skipSlpCheck)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): BroadcastTxRequest {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseBroadcastTxRequest } as BroadcastTxRequest
    message.rawTx = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.rawTx = reader.bytes()
          break
        case 2:
          message.skipSlpCheck = reader.bool()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): BroadcastTxRequest {
    const message = { ...baseBroadcastTxRequest } as BroadcastTxRequest
    message.rawTx =
      object.rawTx !== undefined && object.rawTx !== null
        ? bytesFromBase64(object.rawTx)
        : new Uint8Array()
    message.skipSlpCheck =
      object.skipSlpCheck !== undefined && object.skipSlpCheck !== null
        ? Boolean(object.skipSlpCheck)
        : false
    return message
  },

  toJSON(message: BroadcastTxRequest): unknown {
    const obj: any = {}
    message.rawTx !== undefined &&
      (obj.rawTx = base64FromBytes(
        message.rawTx !== undefined ? message.rawTx : new Uint8Array(),
      ))
    message.skipSlpCheck !== undefined &&
      (obj.skipSlpCheck = message.skipSlpCheck)
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<BroadcastTxRequest>, I>>(
    object: I,
  ): BroadcastTxRequest {
    const message = { ...baseBroadcastTxRequest } as BroadcastTxRequest
    message.rawTx = object.rawTx ?? new Uint8Array()
    message.skipSlpCheck = object.skipSlpCheck ?? false
    return message
  },
}

const baseBroadcastTxResponse: object = {}

export const BroadcastTxResponse = {
  encode(
    message: BroadcastTxResponse,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.txid.length !== 0) {
      writer.uint32(10).bytes(message.txid)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): BroadcastTxResponse {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseBroadcastTxResponse } as BroadcastTxResponse
    message.txid = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.txid = reader.bytes()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): BroadcastTxResponse {
    const message = { ...baseBroadcastTxResponse } as BroadcastTxResponse
    message.txid =
      object.txid !== undefined && object.txid !== null
        ? bytesFromBase64(object.txid)
        : new Uint8Array()
    return message
  },

  toJSON(message: BroadcastTxResponse): unknown {
    const obj: any = {}
    message.txid !== undefined &&
      (obj.txid = base64FromBytes(
        message.txid !== undefined ? message.txid : new Uint8Array(),
      ))
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<BroadcastTxResponse>, I>>(
    object: I,
  ): BroadcastTxResponse {
    const message = { ...baseBroadcastTxResponse } as BroadcastTxResponse
    message.txid = object.txid ?? new Uint8Array()
    return message
  },
}

const baseTx: object = {
  version: 0,
  lockTime: 0,
  slpErrorMsg: "",
  timeFirstSeen: Long.ZERO,
  network: 0,
}

export const Tx = {
  encode(message: Tx, writer: _m0.Writer = _m0.Writer.create()): _m0.Writer {
    if (message.txid.length !== 0) {
      writer.uint32(10).bytes(message.txid)
    }
    if (message.version !== 0) {
      writer.uint32(16).int32(message.version)
    }
    for (const v of message.inputs) {
      TxInput.encode(v!, writer.uint32(26).fork()).ldelim()
    }
    for (const v of message.outputs) {
      TxOutput.encode(v!, writer.uint32(34).fork()).ldelim()
    }
    if (message.lockTime !== 0) {
      writer.uint32(40).uint32(message.lockTime)
    }
    if (message.slpTxData !== undefined) {
      SlpTxData.encode(message.slpTxData, writer.uint32(50).fork()).ldelim()
    }
    if (message.slpErrorMsg !== "") {
      writer.uint32(58).string(message.slpErrorMsg)
    }
    if (message.block !== undefined) {
      BlockMetadata.encode(message.block, writer.uint32(66).fork()).ldelim()
    }
    if (!message.timeFirstSeen.isZero()) {
      writer.uint32(72).int64(message.timeFirstSeen)
    }
    if (message.network !== 0) {
      writer.uint32(80).int32(message.network)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): Tx {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseTx } as Tx
    message.inputs = []
    message.outputs = []
    message.txid = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.txid = reader.bytes()
          break
        case 2:
          message.version = reader.int32()
          break
        case 3:
          message.inputs.push(TxInput.decode(reader, reader.uint32()))
          break
        case 4:
          message.outputs.push(TxOutput.decode(reader, reader.uint32()))
          break
        case 5:
          message.lockTime = reader.uint32()
          break
        case 6:
          message.slpTxData = SlpTxData.decode(reader, reader.uint32())
          break
        case 7:
          message.slpErrorMsg = reader.string()
          break
        case 8:
          message.block = BlockMetadata.decode(reader, reader.uint32())
          break
        case 9:
          message.timeFirstSeen = reader.int64() as Long
          break
        case 10:
          message.network = reader.int32() as any
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): Tx {
    const message = { ...baseTx } as Tx
    message.txid =
      object.txid !== undefined && object.txid !== null
        ? bytesFromBase64(object.txid)
        : new Uint8Array()
    message.version =
      object.version !== undefined && object.version !== null
        ? Number(object.version)
        : 0
    message.inputs = (object.inputs ?? []).map((e: any) => TxInput.fromJSON(e))
    message.outputs = (object.outputs ?? []).map((e: any) =>
      TxOutput.fromJSON(e),
    )
    message.lockTime =
      object.lockTime !== undefined && object.lockTime !== null
        ? Number(object.lockTime)
        : 0
    message.slpTxData =
      object.slpTxData !== undefined && object.slpTxData !== null
        ? SlpTxData.fromJSON(object.slpTxData)
        : undefined
    message.slpErrorMsg =
      object.slpErrorMsg !== undefined && object.slpErrorMsg !== null
        ? String(object.slpErrorMsg)
        : ""
    message.block =
      object.block !== undefined && object.block !== null
        ? BlockMetadata.fromJSON(object.block)
        : undefined
    message.timeFirstSeen =
      object.timeFirstSeen !== undefined && object.timeFirstSeen !== null
        ? Long.fromString(object.timeFirstSeen)
        : Long.ZERO
    message.network =
      object.network !== undefined && object.network !== null
        ? networkFromJSON(object.network)
        : 0
    return message
  },

  toJSON(message: Tx): unknown {
    const obj: any = {}
    message.txid !== undefined &&
      (obj.txid = base64FromBytes(
        message.txid !== undefined ? message.txid : new Uint8Array(),
      ))
    message.version !== undefined && (obj.version = message.version)
    if (message.inputs) {
      obj.inputs = message.inputs.map(e => (e ? TxInput.toJSON(e) : undefined))
    } else {
      obj.inputs = []
    }
    if (message.outputs) {
      obj.outputs = message.outputs.map(e =>
        e ? TxOutput.toJSON(e) : undefined,
      )
    } else {
      obj.outputs = []
    }
    message.lockTime !== undefined && (obj.lockTime = message.lockTime)
    message.slpTxData !== undefined &&
      (obj.slpTxData = message.slpTxData
        ? SlpTxData.toJSON(message.slpTxData)
        : undefined)
    message.slpErrorMsg !== undefined && (obj.slpErrorMsg = message.slpErrorMsg)
    message.block !== undefined &&
      (obj.block = message.block
        ? BlockMetadata.toJSON(message.block)
        : undefined)
    message.timeFirstSeen !== undefined &&
      (obj.timeFirstSeen = (message.timeFirstSeen || Long.ZERO).toString())
    message.network !== undefined &&
      (obj.network = networkToJSON(message.network))
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<Tx>, I>>(object: I): Tx {
    const message = { ...baseTx } as Tx
    message.txid = object.txid ?? new Uint8Array()
    message.version = object.version ?? 0
    message.inputs = object.inputs?.map(e => TxInput.fromPartial(e)) || []
    message.outputs = object.outputs?.map(e => TxOutput.fromPartial(e)) || []
    message.lockTime = object.lockTime ?? 0
    message.slpTxData =
      object.slpTxData !== undefined && object.slpTxData !== null
        ? SlpTxData.fromPartial(object.slpTxData)
        : undefined
    message.slpErrorMsg = object.slpErrorMsg ?? ""
    message.block =
      object.block !== undefined && object.block !== null
        ? BlockMetadata.fromPartial(object.block)
        : undefined
    message.timeFirstSeen =
      object.timeFirstSeen !== undefined && object.timeFirstSeen !== null
        ? Long.fromValue(object.timeFirstSeen)
        : Long.ZERO
    message.network = object.network ?? 0
    return message
  },
}

const baseUtxo: object = {
  blockHeight: 0,
  isCoinbase: false,
  value: Long.ZERO,
  network: 0,
}

export const Utxo = {
  encode(message: Utxo, writer: _m0.Writer = _m0.Writer.create()): _m0.Writer {
    if (message.outpoint !== undefined) {
      OutPoint.encode(message.outpoint, writer.uint32(10).fork()).ldelim()
    }
    if (message.blockHeight !== 0) {
      writer.uint32(16).int32(message.blockHeight)
    }
    if (message.isCoinbase === true) {
      writer.uint32(24).bool(message.isCoinbase)
    }
    if (!message.value.isZero()) {
      writer.uint32(40).int64(message.value)
    }
    if (message.slpMeta !== undefined) {
      SlpMeta.encode(message.slpMeta, writer.uint32(50).fork()).ldelim()
    }
    if (message.slpToken !== undefined) {
      SlpToken.encode(message.slpToken, writer.uint32(58).fork()).ldelim()
    }
    if (message.network !== 0) {
      writer.uint32(72).int32(message.network)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): Utxo {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseUtxo } as Utxo
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.outpoint = OutPoint.decode(reader, reader.uint32())
          break
        case 2:
          message.blockHeight = reader.int32()
          break
        case 3:
          message.isCoinbase = reader.bool()
          break
        case 5:
          message.value = reader.int64() as Long
          break
        case 6:
          message.slpMeta = SlpMeta.decode(reader, reader.uint32())
          break
        case 7:
          message.slpToken = SlpToken.decode(reader, reader.uint32())
          break
        case 9:
          message.network = reader.int32() as any
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): Utxo {
    const message = { ...baseUtxo } as Utxo
    message.outpoint =
      object.outpoint !== undefined && object.outpoint !== null
        ? OutPoint.fromJSON(object.outpoint)
        : undefined
    message.blockHeight =
      object.blockHeight !== undefined && object.blockHeight !== null
        ? Number(object.blockHeight)
        : 0
    message.isCoinbase =
      object.isCoinbase !== undefined && object.isCoinbase !== null
        ? Boolean(object.isCoinbase)
        : false
    message.value =
      object.value !== undefined && object.value !== null
        ? Long.fromString(object.value)
        : Long.ZERO
    message.slpMeta =
      object.slpMeta !== undefined && object.slpMeta !== null
        ? SlpMeta.fromJSON(object.slpMeta)
        : undefined
    message.slpToken =
      object.slpToken !== undefined && object.slpToken !== null
        ? SlpToken.fromJSON(object.slpToken)
        : undefined
    message.network =
      object.network !== undefined && object.network !== null
        ? networkFromJSON(object.network)
        : 0
    return message
  },

  toJSON(message: Utxo): unknown {
    const obj: any = {}
    message.outpoint !== undefined &&
      (obj.outpoint = message.outpoint
        ? OutPoint.toJSON(message.outpoint)
        : undefined)
    message.blockHeight !== undefined && (obj.blockHeight = message.blockHeight)
    message.isCoinbase !== undefined && (obj.isCoinbase = message.isCoinbase)
    message.value !== undefined &&
      (obj.value = (message.value || Long.ZERO).toString())
    message.slpMeta !== undefined &&
      (obj.slpMeta = message.slpMeta
        ? SlpMeta.toJSON(message.slpMeta)
        : undefined)
    message.slpToken !== undefined &&
      (obj.slpToken = message.slpToken
        ? SlpToken.toJSON(message.slpToken)
        : undefined)
    message.network !== undefined &&
      (obj.network = networkToJSON(message.network))
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<Utxo>, I>>(object: I): Utxo {
    const message = { ...baseUtxo } as Utxo
    message.outpoint =
      object.outpoint !== undefined && object.outpoint !== null
        ? OutPoint.fromPartial(object.outpoint)
        : undefined
    message.blockHeight = object.blockHeight ?? 0
    message.isCoinbase = object.isCoinbase ?? false
    message.value =
      object.value !== undefined && object.value !== null
        ? Long.fromValue(object.value)
        : Long.ZERO
    message.slpMeta =
      object.slpMeta !== undefined && object.slpMeta !== null
        ? SlpMeta.fromPartial(object.slpMeta)
        : undefined
    message.slpToken =
      object.slpToken !== undefined && object.slpToken !== null
        ? SlpToken.fromPartial(object.slpToken)
        : undefined
    message.network = object.network ?? 0
    return message
  },
}

const baseBlockInfo: object = {
  height: 0,
  nBits: 0,
  timestamp: Long.ZERO,
  blockSize: Long.UZERO,
  numTxs: Long.UZERO,
  numInputs: Long.UZERO,
  numOutputs: Long.UZERO,
  sumInputSats: Long.ZERO,
  sumCoinbaseOutputSats: Long.ZERO,
  sumNormalOutputSats: Long.ZERO,
  sumBurnedSats: Long.ZERO,
}

export const BlockInfo = {
  encode(
    message: BlockInfo,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.hash.length !== 0) {
      writer.uint32(10).bytes(message.hash)
    }
    if (message.prevHash.length !== 0) {
      writer.uint32(18).bytes(message.prevHash)
    }
    if (message.height !== 0) {
      writer.uint32(24).int32(message.height)
    }
    if (message.nBits !== 0) {
      writer.uint32(32).uint32(message.nBits)
    }
    if (!message.timestamp.isZero()) {
      writer.uint32(40).int64(message.timestamp)
    }
    if (!message.blockSize.isZero()) {
      writer.uint32(48).uint64(message.blockSize)
    }
    if (!message.numTxs.isZero()) {
      writer.uint32(56).uint64(message.numTxs)
    }
    if (!message.numInputs.isZero()) {
      writer.uint32(64).uint64(message.numInputs)
    }
    if (!message.numOutputs.isZero()) {
      writer.uint32(72).uint64(message.numOutputs)
    }
    if (!message.sumInputSats.isZero()) {
      writer.uint32(80).int64(message.sumInputSats)
    }
    if (!message.sumCoinbaseOutputSats.isZero()) {
      writer.uint32(88).int64(message.sumCoinbaseOutputSats)
    }
    if (!message.sumNormalOutputSats.isZero()) {
      writer.uint32(96).int64(message.sumNormalOutputSats)
    }
    if (!message.sumBurnedSats.isZero()) {
      writer.uint32(104).int64(message.sumBurnedSats)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): BlockInfo {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseBlockInfo } as BlockInfo
    message.hash = new Uint8Array()
    message.prevHash = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.hash = reader.bytes()
          break
        case 2:
          message.prevHash = reader.bytes()
          break
        case 3:
          message.height = reader.int32()
          break
        case 4:
          message.nBits = reader.uint32()
          break
        case 5:
          message.timestamp = reader.int64() as Long
          break
        case 6:
          message.blockSize = reader.uint64() as Long
          break
        case 7:
          message.numTxs = reader.uint64() as Long
          break
        case 8:
          message.numInputs = reader.uint64() as Long
          break
        case 9:
          message.numOutputs = reader.uint64() as Long
          break
        case 10:
          message.sumInputSats = reader.int64() as Long
          break
        case 11:
          message.sumCoinbaseOutputSats = reader.int64() as Long
          break
        case 12:
          message.sumNormalOutputSats = reader.int64() as Long
          break
        case 13:
          message.sumBurnedSats = reader.int64() as Long
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): BlockInfo {
    const message = { ...baseBlockInfo } as BlockInfo
    message.hash =
      object.hash !== undefined && object.hash !== null
        ? bytesFromBase64(object.hash)
        : new Uint8Array()
    message.prevHash =
      object.prevHash !== undefined && object.prevHash !== null
        ? bytesFromBase64(object.prevHash)
        : new Uint8Array()
    message.height =
      object.height !== undefined && object.height !== null
        ? Number(object.height)
        : 0
    message.nBits =
      object.nBits !== undefined && object.nBits !== null
        ? Number(object.nBits)
        : 0
    message.timestamp =
      object.timestamp !== undefined && object.timestamp !== null
        ? Long.fromString(object.timestamp)
        : Long.ZERO
    message.blockSize =
      object.blockSize !== undefined && object.blockSize !== null
        ? Long.fromString(object.blockSize)
        : Long.UZERO
    message.numTxs =
      object.numTxs !== undefined && object.numTxs !== null
        ? Long.fromString(object.numTxs)
        : Long.UZERO
    message.numInputs =
      object.numInputs !== undefined && object.numInputs !== null
        ? Long.fromString(object.numInputs)
        : Long.UZERO
    message.numOutputs =
      object.numOutputs !== undefined && object.numOutputs !== null
        ? Long.fromString(object.numOutputs)
        : Long.UZERO
    message.sumInputSats =
      object.sumInputSats !== undefined && object.sumInputSats !== null
        ? Long.fromString(object.sumInputSats)
        : Long.ZERO
    message.sumCoinbaseOutputSats =
      object.sumCoinbaseOutputSats !== undefined &&
      object.sumCoinbaseOutputSats !== null
        ? Long.fromString(object.sumCoinbaseOutputSats)
        : Long.ZERO
    message.sumNormalOutputSats =
      object.sumNormalOutputSats !== undefined &&
      object.sumNormalOutputSats !== null
        ? Long.fromString(object.sumNormalOutputSats)
        : Long.ZERO
    message.sumBurnedSats =
      object.sumBurnedSats !== undefined && object.sumBurnedSats !== null
        ? Long.fromString(object.sumBurnedSats)
        : Long.ZERO
    return message
  },

  toJSON(message: BlockInfo): unknown {
    const obj: any = {}
    message.hash !== undefined &&
      (obj.hash = base64FromBytes(
        message.hash !== undefined ? message.hash : new Uint8Array(),
      ))
    message.prevHash !== undefined &&
      (obj.prevHash = base64FromBytes(
        message.prevHash !== undefined ? message.prevHash : new Uint8Array(),
      ))
    message.height !== undefined && (obj.height = message.height)
    message.nBits !== undefined && (obj.nBits = message.nBits)
    message.timestamp !== undefined &&
      (obj.timestamp = (message.timestamp || Long.ZERO).toString())
    message.blockSize !== undefined &&
      (obj.blockSize = (message.blockSize || Long.UZERO).toString())
    message.numTxs !== undefined &&
      (obj.numTxs = (message.numTxs || Long.UZERO).toString())
    message.numInputs !== undefined &&
      (obj.numInputs = (message.numInputs || Long.UZERO).toString())
    message.numOutputs !== undefined &&
      (obj.numOutputs = (message.numOutputs || Long.UZERO).toString())
    message.sumInputSats !== undefined &&
      (obj.sumInputSats = (message.sumInputSats || Long.ZERO).toString())
    message.sumCoinbaseOutputSats !== undefined &&
      (obj.sumCoinbaseOutputSats = (
        message.sumCoinbaseOutputSats || Long.ZERO
      ).toString())
    message.sumNormalOutputSats !== undefined &&
      (obj.sumNormalOutputSats = (
        message.sumNormalOutputSats || Long.ZERO
      ).toString())
    message.sumBurnedSats !== undefined &&
      (obj.sumBurnedSats = (message.sumBurnedSats || Long.ZERO).toString())
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<BlockInfo>, I>>(
    object: I,
  ): BlockInfo {
    const message = { ...baseBlockInfo } as BlockInfo
    message.hash = object.hash ?? new Uint8Array()
    message.prevHash = object.prevHash ?? new Uint8Array()
    message.height = object.height ?? 0
    message.nBits = object.nBits ?? 0
    message.timestamp =
      object.timestamp !== undefined && object.timestamp !== null
        ? Long.fromValue(object.timestamp)
        : Long.ZERO
    message.blockSize =
      object.blockSize !== undefined && object.blockSize !== null
        ? Long.fromValue(object.blockSize)
        : Long.UZERO
    message.numTxs =
      object.numTxs !== undefined && object.numTxs !== null
        ? Long.fromValue(object.numTxs)
        : Long.UZERO
    message.numInputs =
      object.numInputs !== undefined && object.numInputs !== null
        ? Long.fromValue(object.numInputs)
        : Long.UZERO
    message.numOutputs =
      object.numOutputs !== undefined && object.numOutputs !== null
        ? Long.fromValue(object.numOutputs)
        : Long.UZERO
    message.sumInputSats =
      object.sumInputSats !== undefined && object.sumInputSats !== null
        ? Long.fromValue(object.sumInputSats)
        : Long.ZERO
    message.sumCoinbaseOutputSats =
      object.sumCoinbaseOutputSats !== undefined &&
      object.sumCoinbaseOutputSats !== null
        ? Long.fromValue(object.sumCoinbaseOutputSats)
        : Long.ZERO
    message.sumNormalOutputSats =
      object.sumNormalOutputSats !== undefined &&
      object.sumNormalOutputSats !== null
        ? Long.fromValue(object.sumNormalOutputSats)
        : Long.ZERO
    message.sumBurnedSats =
      object.sumBurnedSats !== undefined && object.sumBurnedSats !== null
        ? Long.fromValue(object.sumBurnedSats)
        : Long.ZERO
    return message
  },
}

const baseBlock: object = {}

export const Block = {
  encode(message: Block, writer: _m0.Writer = _m0.Writer.create()): _m0.Writer {
    if (message.blockInfo !== undefined) {
      BlockInfo.encode(message.blockInfo, writer.uint32(10).fork()).ldelim()
    }
    for (const v of message.txs) {
      Tx.encode(v!, writer.uint32(18).fork()).ldelim()
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): Block {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseBlock } as Block
    message.txs = []
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.blockInfo = BlockInfo.decode(reader, reader.uint32())
          break
        case 2:
          message.txs.push(Tx.decode(reader, reader.uint32()))
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): Block {
    const message = { ...baseBlock } as Block
    message.blockInfo =
      object.blockInfo !== undefined && object.blockInfo !== null
        ? BlockInfo.fromJSON(object.blockInfo)
        : undefined
    message.txs = (object.txs ?? []).map((e: any) => Tx.fromJSON(e))
    return message
  },

  toJSON(message: Block): unknown {
    const obj: any = {}
    message.blockInfo !== undefined &&
      (obj.blockInfo = message.blockInfo
        ? BlockInfo.toJSON(message.blockInfo)
        : undefined)
    if (message.txs) {
      obj.txs = message.txs.map(e => (e ? Tx.toJSON(e) : undefined))
    } else {
      obj.txs = []
    }
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<Block>, I>>(object: I): Block {
    const message = { ...baseBlock } as Block
    message.blockInfo =
      object.blockInfo !== undefined && object.blockInfo !== null
        ? BlockInfo.fromPartial(object.blockInfo)
        : undefined
    message.txs = object.txs?.map(e => Tx.fromPartial(e)) || []
    return message
  },
}

const baseScriptUtxos: object = {}

export const ScriptUtxos = {
  encode(
    message: ScriptUtxos,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.outputScript.length !== 0) {
      writer.uint32(10).bytes(message.outputScript)
    }
    for (const v of message.utxos) {
      Utxo.encode(v!, writer.uint32(18).fork()).ldelim()
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): ScriptUtxos {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseScriptUtxos } as ScriptUtxos
    message.utxos = []
    message.outputScript = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.outputScript = reader.bytes()
          break
        case 2:
          message.utxos.push(Utxo.decode(reader, reader.uint32()))
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): ScriptUtxos {
    const message = { ...baseScriptUtxos } as ScriptUtxos
    message.outputScript =
      object.outputScript !== undefined && object.outputScript !== null
        ? bytesFromBase64(object.outputScript)
        : new Uint8Array()
    message.utxos = (object.utxos ?? []).map((e: any) => Utxo.fromJSON(e))
    return message
  },

  toJSON(message: ScriptUtxos): unknown {
    const obj: any = {}
    message.outputScript !== undefined &&
      (obj.outputScript = base64FromBytes(
        message.outputScript !== undefined
          ? message.outputScript
          : new Uint8Array(),
      ))
    if (message.utxos) {
      obj.utxos = message.utxos.map(e => (e ? Utxo.toJSON(e) : undefined))
    } else {
      obj.utxos = []
    }
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<ScriptUtxos>, I>>(
    object: I,
  ): ScriptUtxos {
    const message = { ...baseScriptUtxos } as ScriptUtxos
    message.outputScript = object.outputScript ?? new Uint8Array()
    message.utxos = object.utxos?.map(e => Utxo.fromPartial(e)) || []
    return message
  },
}

const baseTxHistoryPage: object = { numPages: 0 }

export const TxHistoryPage = {
  encode(
    message: TxHistoryPage,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    for (const v of message.txs) {
      Tx.encode(v!, writer.uint32(10).fork()).ldelim()
    }
    if (message.numPages !== 0) {
      writer.uint32(16).uint32(message.numPages)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): TxHistoryPage {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseTxHistoryPage } as TxHistoryPage
    message.txs = []
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.txs.push(Tx.decode(reader, reader.uint32()))
          break
        case 2:
          message.numPages = reader.uint32()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): TxHistoryPage {
    const message = { ...baseTxHistoryPage } as TxHistoryPage
    message.txs = (object.txs ?? []).map((e: any) => Tx.fromJSON(e))
    message.numPages =
      object.numPages !== undefined && object.numPages !== null
        ? Number(object.numPages)
        : 0
    return message
  },

  toJSON(message: TxHistoryPage): unknown {
    const obj: any = {}
    if (message.txs) {
      obj.txs = message.txs.map(e => (e ? Tx.toJSON(e) : undefined))
    } else {
      obj.txs = []
    }
    message.numPages !== undefined && (obj.numPages = message.numPages)
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<TxHistoryPage>, I>>(
    object: I,
  ): TxHistoryPage {
    const message = { ...baseTxHistoryPage } as TxHistoryPage
    message.txs = object.txs?.map(e => Tx.fromPartial(e)) || []
    message.numPages = object.numPages ?? 0
    return message
  },
}

const baseUtxos: object = {}

export const Utxos = {
  encode(message: Utxos, writer: _m0.Writer = _m0.Writer.create()): _m0.Writer {
    for (const v of message.scriptUtxos) {
      ScriptUtxos.encode(v!, writer.uint32(10).fork()).ldelim()
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): Utxos {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseUtxos } as Utxos
    message.scriptUtxos = []
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.scriptUtxos.push(ScriptUtxos.decode(reader, reader.uint32()))
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): Utxos {
    const message = { ...baseUtxos } as Utxos
    message.scriptUtxos = (object.scriptUtxos ?? []).map((e: any) =>
      ScriptUtxos.fromJSON(e),
    )
    return message
  },

  toJSON(message: Utxos): unknown {
    const obj: any = {}
    if (message.scriptUtxos) {
      obj.scriptUtxos = message.scriptUtxos.map(e =>
        e ? ScriptUtxos.toJSON(e) : undefined,
      )
    } else {
      obj.scriptUtxos = []
    }
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<Utxos>, I>>(object: I): Utxos {
    const message = { ...baseUtxos } as Utxos
    message.scriptUtxos =
      object.scriptUtxos?.map(e => ScriptUtxos.fromPartial(e)) || []
    return message
  },
}

const baseBlocks: object = {}

export const Blocks = {
  encode(
    message: Blocks,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    for (const v of message.blocks) {
      BlockInfo.encode(v!, writer.uint32(10).fork()).ldelim()
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): Blocks {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseBlocks } as Blocks
    message.blocks = []
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.blocks.push(BlockInfo.decode(reader, reader.uint32()))
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): Blocks {
    const message = { ...baseBlocks } as Blocks
    message.blocks = (object.blocks ?? []).map((e: any) =>
      BlockInfo.fromJSON(e),
    )
    return message
  },

  toJSON(message: Blocks): unknown {
    const obj: any = {}
    if (message.blocks) {
      obj.blocks = message.blocks.map(e =>
        e ? BlockInfo.toJSON(e) : undefined,
      )
    } else {
      obj.blocks = []
    }
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<Blocks>, I>>(object: I): Blocks {
    const message = { ...baseBlocks } as Blocks
    message.blocks = object.blocks?.map(e => BlockInfo.fromPartial(e)) || []
    return message
  },
}

const baseSlpTxData: object = {}

export const SlpTxData = {
  encode(
    message: SlpTxData,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.slpMeta !== undefined) {
      SlpMeta.encode(message.slpMeta, writer.uint32(10).fork()).ldelim()
    }
    if (message.genesisInfo !== undefined) {
      SlpGenesisInfo.encode(
        message.genesisInfo,
        writer.uint32(18).fork(),
      ).ldelim()
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): SlpTxData {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseSlpTxData } as SlpTxData
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.slpMeta = SlpMeta.decode(reader, reader.uint32())
          break
        case 2:
          message.genesisInfo = SlpGenesisInfo.decode(reader, reader.uint32())
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): SlpTxData {
    const message = { ...baseSlpTxData } as SlpTxData
    message.slpMeta =
      object.slpMeta !== undefined && object.slpMeta !== null
        ? SlpMeta.fromJSON(object.slpMeta)
        : undefined
    message.genesisInfo =
      object.genesisInfo !== undefined && object.genesisInfo !== null
        ? SlpGenesisInfo.fromJSON(object.genesisInfo)
        : undefined
    return message
  },

  toJSON(message: SlpTxData): unknown {
    const obj: any = {}
    message.slpMeta !== undefined &&
      (obj.slpMeta = message.slpMeta
        ? SlpMeta.toJSON(message.slpMeta)
        : undefined)
    message.genesisInfo !== undefined &&
      (obj.genesisInfo = message.genesisInfo
        ? SlpGenesisInfo.toJSON(message.genesisInfo)
        : undefined)
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<SlpTxData>, I>>(
    object: I,
  ): SlpTxData {
    const message = { ...baseSlpTxData } as SlpTxData
    message.slpMeta =
      object.slpMeta !== undefined && object.slpMeta !== null
        ? SlpMeta.fromPartial(object.slpMeta)
        : undefined
    message.genesisInfo =
      object.genesisInfo !== undefined && object.genesisInfo !== null
        ? SlpGenesisInfo.fromPartial(object.genesisInfo)
        : undefined
    return message
  },
}

const baseSlpMeta: object = { tokenType: 0, txType: 0 }

export const SlpMeta = {
  encode(
    message: SlpMeta,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.tokenType !== 0) {
      writer.uint32(8).int32(message.tokenType)
    }
    if (message.txType !== 0) {
      writer.uint32(16).int32(message.txType)
    }
    if (message.tokenId.length !== 0) {
      writer.uint32(26).bytes(message.tokenId)
    }
    if (message.groupTokenId.length !== 0) {
      writer.uint32(34).bytes(message.groupTokenId)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): SlpMeta {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseSlpMeta } as SlpMeta
    message.tokenId = new Uint8Array()
    message.groupTokenId = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.tokenType = reader.int32() as any
          break
        case 2:
          message.txType = reader.int32() as any
          break
        case 3:
          message.tokenId = reader.bytes()
          break
        case 4:
          message.groupTokenId = reader.bytes()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): SlpMeta {
    const message = { ...baseSlpMeta } as SlpMeta
    message.tokenType =
      object.tokenType !== undefined && object.tokenType !== null
        ? slpTokenTypeFromJSON(object.tokenType)
        : 0
    message.txType =
      object.txType !== undefined && object.txType !== null
        ? slpTxTypeFromJSON(object.txType)
        : 0
    message.tokenId =
      object.tokenId !== undefined && object.tokenId !== null
        ? bytesFromBase64(object.tokenId)
        : new Uint8Array()
    message.groupTokenId =
      object.groupTokenId !== undefined && object.groupTokenId !== null
        ? bytesFromBase64(object.groupTokenId)
        : new Uint8Array()
    return message
  },

  toJSON(message: SlpMeta): unknown {
    const obj: any = {}
    message.tokenType !== undefined &&
      (obj.tokenType = slpTokenTypeToJSON(message.tokenType))
    message.txType !== undefined &&
      (obj.txType = slpTxTypeToJSON(message.txType))
    message.tokenId !== undefined &&
      (obj.tokenId = base64FromBytes(
        message.tokenId !== undefined ? message.tokenId : new Uint8Array(),
      ))
    message.groupTokenId !== undefined &&
      (obj.groupTokenId = base64FromBytes(
        message.groupTokenId !== undefined
          ? message.groupTokenId
          : new Uint8Array(),
      ))
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<SlpMeta>, I>>(object: I): SlpMeta {
    const message = { ...baseSlpMeta } as SlpMeta
    message.tokenType = object.tokenType ?? 0
    message.txType = object.txType ?? 0
    message.tokenId = object.tokenId ?? new Uint8Array()
    message.groupTokenId = object.groupTokenId ?? new Uint8Array()
    return message
  },
}

const baseTxInput: object = { value: Long.ZERO, sequenceNo: 0 }

export const TxInput = {
  encode(
    message: TxInput,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.prevOut !== undefined) {
      OutPoint.encode(message.prevOut, writer.uint32(10).fork()).ldelim()
    }
    if (message.inputScript.length !== 0) {
      writer.uint32(18).bytes(message.inputScript)
    }
    if (message.outputScript.length !== 0) {
      writer.uint32(26).bytes(message.outputScript)
    }
    if (!message.value.isZero()) {
      writer.uint32(32).int64(message.value)
    }
    if (message.sequenceNo !== 0) {
      writer.uint32(40).uint32(message.sequenceNo)
    }
    if (message.slpBurn !== undefined) {
      SlpBurn.encode(message.slpBurn, writer.uint32(50).fork()).ldelim()
    }
    if (message.slpToken !== undefined) {
      SlpToken.encode(message.slpToken, writer.uint32(58).fork()).ldelim()
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): TxInput {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseTxInput } as TxInput
    message.inputScript = new Uint8Array()
    message.outputScript = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.prevOut = OutPoint.decode(reader, reader.uint32())
          break
        case 2:
          message.inputScript = reader.bytes()
          break
        case 3:
          message.outputScript = reader.bytes()
          break
        case 4:
          message.value = reader.int64() as Long
          break
        case 5:
          message.sequenceNo = reader.uint32()
          break
        case 6:
          message.slpBurn = SlpBurn.decode(reader, reader.uint32())
          break
        case 7:
          message.slpToken = SlpToken.decode(reader, reader.uint32())
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): TxInput {
    const message = { ...baseTxInput } as TxInput
    message.prevOut =
      object.prevOut !== undefined && object.prevOut !== null
        ? OutPoint.fromJSON(object.prevOut)
        : undefined
    message.inputScript =
      object.inputScript !== undefined && object.inputScript !== null
        ? bytesFromBase64(object.inputScript)
        : new Uint8Array()
    message.outputScript =
      object.outputScript !== undefined && object.outputScript !== null
        ? bytesFromBase64(object.outputScript)
        : new Uint8Array()
    message.value =
      object.value !== undefined && object.value !== null
        ? Long.fromString(object.value)
        : Long.ZERO
    message.sequenceNo =
      object.sequenceNo !== undefined && object.sequenceNo !== null
        ? Number(object.sequenceNo)
        : 0
    message.slpBurn =
      object.slpBurn !== undefined && object.slpBurn !== null
        ? SlpBurn.fromJSON(object.slpBurn)
        : undefined
    message.slpToken =
      object.slpToken !== undefined && object.slpToken !== null
        ? SlpToken.fromJSON(object.slpToken)
        : undefined
    return message
  },

  toJSON(message: TxInput): unknown {
    const obj: any = {}
    message.prevOut !== undefined &&
      (obj.prevOut = message.prevOut
        ? OutPoint.toJSON(message.prevOut)
        : undefined)
    message.inputScript !== undefined &&
      (obj.inputScript = base64FromBytes(
        message.inputScript !== undefined
          ? message.inputScript
          : new Uint8Array(),
      ))
    message.outputScript !== undefined &&
      (obj.outputScript = base64FromBytes(
        message.outputScript !== undefined
          ? message.outputScript
          : new Uint8Array(),
      ))
    message.value !== undefined &&
      (obj.value = (message.value || Long.ZERO).toString())
    message.sequenceNo !== undefined && (obj.sequenceNo = message.sequenceNo)
    message.slpBurn !== undefined &&
      (obj.slpBurn = message.slpBurn
        ? SlpBurn.toJSON(message.slpBurn)
        : undefined)
    message.slpToken !== undefined &&
      (obj.slpToken = message.slpToken
        ? SlpToken.toJSON(message.slpToken)
        : undefined)
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<TxInput>, I>>(object: I): TxInput {
    const message = { ...baseTxInput } as TxInput
    message.prevOut =
      object.prevOut !== undefined && object.prevOut !== null
        ? OutPoint.fromPartial(object.prevOut)
        : undefined
    message.inputScript = object.inputScript ?? new Uint8Array()
    message.outputScript = object.outputScript ?? new Uint8Array()
    message.value =
      object.value !== undefined && object.value !== null
        ? Long.fromValue(object.value)
        : Long.ZERO
    message.sequenceNo = object.sequenceNo ?? 0
    message.slpBurn =
      object.slpBurn !== undefined && object.slpBurn !== null
        ? SlpBurn.fromPartial(object.slpBurn)
        : undefined
    message.slpToken =
      object.slpToken !== undefined && object.slpToken !== null
        ? SlpToken.fromPartial(object.slpToken)
        : undefined
    return message
  },
}

const baseTxOutput: object = { value: Long.ZERO }

export const TxOutput = {
  encode(
    message: TxOutput,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (!message.value.isZero()) {
      writer.uint32(8).int64(message.value)
    }
    if (message.outputScript.length !== 0) {
      writer.uint32(18).bytes(message.outputScript)
    }
    if (message.slpToken !== undefined) {
      SlpToken.encode(message.slpToken, writer.uint32(26).fork()).ldelim()
    }
    if (message.spentBy !== undefined) {
      OutPoint.encode(message.spentBy, writer.uint32(34).fork()).ldelim()
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): TxOutput {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseTxOutput } as TxOutput
    message.outputScript = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.value = reader.int64() as Long
          break
        case 2:
          message.outputScript = reader.bytes()
          break
        case 3:
          message.slpToken = SlpToken.decode(reader, reader.uint32())
          break
        case 4:
          message.spentBy = OutPoint.decode(reader, reader.uint32())
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): TxOutput {
    const message = { ...baseTxOutput } as TxOutput
    message.value =
      object.value !== undefined && object.value !== null
        ? Long.fromString(object.value)
        : Long.ZERO
    message.outputScript =
      object.outputScript !== undefined && object.outputScript !== null
        ? bytesFromBase64(object.outputScript)
        : new Uint8Array()
    message.slpToken =
      object.slpToken !== undefined && object.slpToken !== null
        ? SlpToken.fromJSON(object.slpToken)
        : undefined
    message.spentBy =
      object.spentBy !== undefined && object.spentBy !== null
        ? OutPoint.fromJSON(object.spentBy)
        : undefined
    return message
  },

  toJSON(message: TxOutput): unknown {
    const obj: any = {}
    message.value !== undefined &&
      (obj.value = (message.value || Long.ZERO).toString())
    message.outputScript !== undefined &&
      (obj.outputScript = base64FromBytes(
        message.outputScript !== undefined
          ? message.outputScript
          : new Uint8Array(),
      ))
    message.slpToken !== undefined &&
      (obj.slpToken = message.slpToken
        ? SlpToken.toJSON(message.slpToken)
        : undefined)
    message.spentBy !== undefined &&
      (obj.spentBy = message.spentBy
        ? OutPoint.toJSON(message.spentBy)
        : undefined)
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<TxOutput>, I>>(object: I): TxOutput {
    const message = { ...baseTxOutput } as TxOutput
    message.value =
      object.value !== undefined && object.value !== null
        ? Long.fromValue(object.value)
        : Long.ZERO
    message.outputScript = object.outputScript ?? new Uint8Array()
    message.slpToken =
      object.slpToken !== undefined && object.slpToken !== null
        ? SlpToken.fromPartial(object.slpToken)
        : undefined
    message.spentBy =
      object.spentBy !== undefined && object.spentBy !== null
        ? OutPoint.fromPartial(object.spentBy)
        : undefined
    return message
  },
}

const baseBlockMetadata: object = { height: 0, timestamp: Long.ZERO }

export const BlockMetadata = {
  encode(
    message: BlockMetadata,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.height !== 0) {
      writer.uint32(8).int32(message.height)
    }
    if (message.hash.length !== 0) {
      writer.uint32(18).bytes(message.hash)
    }
    if (!message.timestamp.isZero()) {
      writer.uint32(24).int64(message.timestamp)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): BlockMetadata {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseBlockMetadata } as BlockMetadata
    message.hash = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.height = reader.int32()
          break
        case 2:
          message.hash = reader.bytes()
          break
        case 3:
          message.timestamp = reader.int64() as Long
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): BlockMetadata {
    const message = { ...baseBlockMetadata } as BlockMetadata
    message.height =
      object.height !== undefined && object.height !== null
        ? Number(object.height)
        : 0
    message.hash =
      object.hash !== undefined && object.hash !== null
        ? bytesFromBase64(object.hash)
        : new Uint8Array()
    message.timestamp =
      object.timestamp !== undefined && object.timestamp !== null
        ? Long.fromString(object.timestamp)
        : Long.ZERO
    return message
  },

  toJSON(message: BlockMetadata): unknown {
    const obj: any = {}
    message.height !== undefined && (obj.height = message.height)
    message.hash !== undefined &&
      (obj.hash = base64FromBytes(
        message.hash !== undefined ? message.hash : new Uint8Array(),
      ))
    message.timestamp !== undefined &&
      (obj.timestamp = (message.timestamp || Long.ZERO).toString())
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<BlockMetadata>, I>>(
    object: I,
  ): BlockMetadata {
    const message = { ...baseBlockMetadata } as BlockMetadata
    message.height = object.height ?? 0
    message.hash = object.hash ?? new Uint8Array()
    message.timestamp =
      object.timestamp !== undefined && object.timestamp !== null
        ? Long.fromValue(object.timestamp)
        : Long.ZERO
    return message
  },
}

const baseOutPoint: object = { outIdx: 0 }

export const OutPoint = {
  encode(
    message: OutPoint,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.txid.length !== 0) {
      writer.uint32(10).bytes(message.txid)
    }
    if (message.outIdx !== 0) {
      writer.uint32(16).uint32(message.outIdx)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): OutPoint {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseOutPoint } as OutPoint
    message.txid = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.txid = reader.bytes()
          break
        case 2:
          message.outIdx = reader.uint32()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): OutPoint {
    const message = { ...baseOutPoint } as OutPoint
    message.txid =
      object.txid !== undefined && object.txid !== null
        ? bytesFromBase64(object.txid)
        : new Uint8Array()
    message.outIdx =
      object.outIdx !== undefined && object.outIdx !== null
        ? Number(object.outIdx)
        : 0
    return message
  },

  toJSON(message: OutPoint): unknown {
    const obj: any = {}
    message.txid !== undefined &&
      (obj.txid = base64FromBytes(
        message.txid !== undefined ? message.txid : new Uint8Array(),
      ))
    message.outIdx !== undefined && (obj.outIdx = message.outIdx)
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<OutPoint>, I>>(object: I): OutPoint {
    const message = { ...baseOutPoint } as OutPoint
    message.txid = object.txid ?? new Uint8Array()
    message.outIdx = object.outIdx ?? 0
    return message
  },
}

const baseSlpToken: object = { amount: Long.UZERO, isMintBaton: false }

export const SlpToken = {
  encode(
    message: SlpToken,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (!message.amount.isZero()) {
      writer.uint32(8).uint64(message.amount)
    }
    if (message.isMintBaton === true) {
      writer.uint32(16).bool(message.isMintBaton)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): SlpToken {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseSlpToken } as SlpToken
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.amount = reader.uint64() as Long
          break
        case 2:
          message.isMintBaton = reader.bool()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): SlpToken {
    const message = { ...baseSlpToken } as SlpToken
    message.amount =
      object.amount !== undefined && object.amount !== null
        ? Long.fromString(object.amount)
        : Long.UZERO
    message.isMintBaton =
      object.isMintBaton !== undefined && object.isMintBaton !== null
        ? Boolean(object.isMintBaton)
        : false
    return message
  },

  toJSON(message: SlpToken): unknown {
    const obj: any = {}
    message.amount !== undefined &&
      (obj.amount = (message.amount || Long.UZERO).toString())
    message.isMintBaton !== undefined && (obj.isMintBaton = message.isMintBaton)
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<SlpToken>, I>>(object: I): SlpToken {
    const message = { ...baseSlpToken } as SlpToken
    message.amount =
      object.amount !== undefined && object.amount !== null
        ? Long.fromValue(object.amount)
        : Long.UZERO
    message.isMintBaton = object.isMintBaton ?? false
    return message
  },
}

const baseSlpBurn: object = {}

export const SlpBurn = {
  encode(
    message: SlpBurn,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.token !== undefined) {
      SlpToken.encode(message.token, writer.uint32(10).fork()).ldelim()
    }
    if (message.tokenId.length !== 0) {
      writer.uint32(18).bytes(message.tokenId)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): SlpBurn {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseSlpBurn } as SlpBurn
    message.tokenId = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.token = SlpToken.decode(reader, reader.uint32())
          break
        case 2:
          message.tokenId = reader.bytes()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): SlpBurn {
    const message = { ...baseSlpBurn } as SlpBurn
    message.token =
      object.token !== undefined && object.token !== null
        ? SlpToken.fromJSON(object.token)
        : undefined
    message.tokenId =
      object.tokenId !== undefined && object.tokenId !== null
        ? bytesFromBase64(object.tokenId)
        : new Uint8Array()
    return message
  },

  toJSON(message: SlpBurn): unknown {
    const obj: any = {}
    message.token !== undefined &&
      (obj.token = message.token ? SlpToken.toJSON(message.token) : undefined)
    message.tokenId !== undefined &&
      (obj.tokenId = base64FromBytes(
        message.tokenId !== undefined ? message.tokenId : new Uint8Array(),
      ))
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<SlpBurn>, I>>(object: I): SlpBurn {
    const message = { ...baseSlpBurn } as SlpBurn
    message.token =
      object.token !== undefined && object.token !== null
        ? SlpToken.fromPartial(object.token)
        : undefined
    message.tokenId = object.tokenId ?? new Uint8Array()
    return message
  },
}

const baseSlpGenesisInfo: object = { decimals: 0 }

export const SlpGenesisInfo = {
  encode(
    message: SlpGenesisInfo,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.tokenTicker.length !== 0) {
      writer.uint32(10).bytes(message.tokenTicker)
    }
    if (message.tokenName.length !== 0) {
      writer.uint32(18).bytes(message.tokenName)
    }
    if (message.tokenDocumentUrl.length !== 0) {
      writer.uint32(26).bytes(message.tokenDocumentUrl)
    }
    if (message.tokenDocumentHash.length !== 0) {
      writer.uint32(34).bytes(message.tokenDocumentHash)
    }
    if (message.decimals !== 0) {
      writer.uint32(40).uint32(message.decimals)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): SlpGenesisInfo {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseSlpGenesisInfo } as SlpGenesisInfo
    message.tokenTicker = new Uint8Array()
    message.tokenName = new Uint8Array()
    message.tokenDocumentUrl = new Uint8Array()
    message.tokenDocumentHash = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.tokenTicker = reader.bytes()
          break
        case 2:
          message.tokenName = reader.bytes()
          break
        case 3:
          message.tokenDocumentUrl = reader.bytes()
          break
        case 4:
          message.tokenDocumentHash = reader.bytes()
          break
        case 5:
          message.decimals = reader.uint32()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): SlpGenesisInfo {
    const message = { ...baseSlpGenesisInfo } as SlpGenesisInfo
    message.tokenTicker =
      object.tokenTicker !== undefined && object.tokenTicker !== null
        ? bytesFromBase64(object.tokenTicker)
        : new Uint8Array()
    message.tokenName =
      object.tokenName !== undefined && object.tokenName !== null
        ? bytesFromBase64(object.tokenName)
        : new Uint8Array()
    message.tokenDocumentUrl =
      object.tokenDocumentUrl !== undefined && object.tokenDocumentUrl !== null
        ? bytesFromBase64(object.tokenDocumentUrl)
        : new Uint8Array()
    message.tokenDocumentHash =
      object.tokenDocumentHash !== undefined &&
      object.tokenDocumentHash !== null
        ? bytesFromBase64(object.tokenDocumentHash)
        : new Uint8Array()
    message.decimals =
      object.decimals !== undefined && object.decimals !== null
        ? Number(object.decimals)
        : 0
    return message
  },

  toJSON(message: SlpGenesisInfo): unknown {
    const obj: any = {}
    message.tokenTicker !== undefined &&
      (obj.tokenTicker = base64FromBytes(
        message.tokenTicker !== undefined
          ? message.tokenTicker
          : new Uint8Array(),
      ))
    message.tokenName !== undefined &&
      (obj.tokenName = base64FromBytes(
        message.tokenName !== undefined ? message.tokenName : new Uint8Array(),
      ))
    message.tokenDocumentUrl !== undefined &&
      (obj.tokenDocumentUrl = base64FromBytes(
        message.tokenDocumentUrl !== undefined
          ? message.tokenDocumentUrl
          : new Uint8Array(),
      ))
    message.tokenDocumentHash !== undefined &&
      (obj.tokenDocumentHash = base64FromBytes(
        message.tokenDocumentHash !== undefined
          ? message.tokenDocumentHash
          : new Uint8Array(),
      ))
    message.decimals !== undefined && (obj.decimals = message.decimals)
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<SlpGenesisInfo>, I>>(
    object: I,
  ): SlpGenesisInfo {
    const message = { ...baseSlpGenesisInfo } as SlpGenesisInfo
    message.tokenTicker = object.tokenTicker ?? new Uint8Array()
    message.tokenName = object.tokenName ?? new Uint8Array()
    message.tokenDocumentUrl = object.tokenDocumentUrl ?? new Uint8Array()
    message.tokenDocumentHash = object.tokenDocumentHash ?? new Uint8Array()
    message.decimals = object.decimals ?? 0
    return message
  },
}

const baseUtxoState: object = { height: 0, isConfirmed: false, state: 0 }

export const UtxoState = {
  encode(
    message: UtxoState,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.height !== 0) {
      writer.uint32(8).int32(message.height)
    }
    if (message.isConfirmed === true) {
      writer.uint32(16).bool(message.isConfirmed)
    }
    if (message.state !== 0) {
      writer.uint32(24).int32(message.state)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): UtxoState {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseUtxoState } as UtxoState
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.height = reader.int32()
          break
        case 2:
          message.isConfirmed = reader.bool()
          break
        case 3:
          message.state = reader.int32() as any
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): UtxoState {
    const message = { ...baseUtxoState } as UtxoState
    message.height =
      object.height !== undefined && object.height !== null
        ? Number(object.height)
        : 0
    message.isConfirmed =
      object.isConfirmed !== undefined && object.isConfirmed !== null
        ? Boolean(object.isConfirmed)
        : false
    message.state =
      object.state !== undefined && object.state !== null
        ? utxoStateVariantFromJSON(object.state)
        : 0
    return message
  },

  toJSON(message: UtxoState): unknown {
    const obj: any = {}
    message.height !== undefined && (obj.height = message.height)
    message.isConfirmed !== undefined && (obj.isConfirmed = message.isConfirmed)
    message.state !== undefined &&
      (obj.state = utxoStateVariantToJSON(message.state))
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<UtxoState>, I>>(
    object: I,
  ): UtxoState {
    const message = { ...baseUtxoState } as UtxoState
    message.height = object.height ?? 0
    message.isConfirmed = object.isConfirmed ?? false
    message.state = object.state ?? 0
    return message
  },
}

const baseSubscription: object = { scriptType: "", isSubscribe: false }

export const Subscription = {
  encode(
    message: Subscription,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.scriptType !== "") {
      writer.uint32(10).string(message.scriptType)
    }
    if (message.payload.length !== 0) {
      writer.uint32(18).bytes(message.payload)
    }
    if (message.isSubscribe === true) {
      writer.uint32(24).bool(message.isSubscribe)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): Subscription {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseSubscription } as Subscription
    message.payload = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.scriptType = reader.string()
          break
        case 2:
          message.payload = reader.bytes()
          break
        case 3:
          message.isSubscribe = reader.bool()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): Subscription {
    const message = { ...baseSubscription } as Subscription
    message.scriptType =
      object.scriptType !== undefined && object.scriptType !== null
        ? String(object.scriptType)
        : ""
    message.payload =
      object.payload !== undefined && object.payload !== null
        ? bytesFromBase64(object.payload)
        : new Uint8Array()
    message.isSubscribe =
      object.isSubscribe !== undefined && object.isSubscribe !== null
        ? Boolean(object.isSubscribe)
        : false
    return message
  },

  toJSON(message: Subscription): unknown {
    const obj: any = {}
    message.scriptType !== undefined && (obj.scriptType = message.scriptType)
    message.payload !== undefined &&
      (obj.payload = base64FromBytes(
        message.payload !== undefined ? message.payload : new Uint8Array(),
      ))
    message.isSubscribe !== undefined && (obj.isSubscribe = message.isSubscribe)
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<Subscription>, I>>(
    object: I,
  ): Subscription {
    const message = { ...baseSubscription } as Subscription
    message.scriptType = object.scriptType ?? ""
    message.payload = object.payload ?? new Uint8Array()
    message.isSubscribe = object.isSubscribe ?? false
    return message
  },
}

const baseSubscribeMsg: object = {}

export const SubscribeMsg = {
  encode(
    message: SubscribeMsg,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.error !== undefined) {
      Error.encode(message.error, writer.uint32(10).fork()).ldelim()
    }
    if (message.AddedToMempool !== undefined) {
      MsgAddedToMempool.encode(
        message.AddedToMempool,
        writer.uint32(18).fork(),
      ).ldelim()
    }
    if (message.RemovedFromMempool !== undefined) {
      MsgRemovedFromMempool.encode(
        message.RemovedFromMempool,
        writer.uint32(26).fork(),
      ).ldelim()
    }
    if (message.Confirmed !== undefined) {
      MsgConfirmed.encode(message.Confirmed, writer.uint32(34).fork()).ldelim()
    }
    if (message.Reorg !== undefined) {
      MsgReorg.encode(message.Reorg, writer.uint32(42).fork()).ldelim()
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): SubscribeMsg {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseSubscribeMsg } as SubscribeMsg
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.error = Error.decode(reader, reader.uint32())
          break
        case 2:
          message.AddedToMempool = MsgAddedToMempool.decode(
            reader,
            reader.uint32(),
          )
          break
        case 3:
          message.RemovedFromMempool = MsgRemovedFromMempool.decode(
            reader,
            reader.uint32(),
          )
          break
        case 4:
          message.Confirmed = MsgConfirmed.decode(reader, reader.uint32())
          break
        case 5:
          message.Reorg = MsgReorg.decode(reader, reader.uint32())
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): SubscribeMsg {
    const message = { ...baseSubscribeMsg } as SubscribeMsg
    message.error =
      object.error !== undefined && object.error !== null
        ? Error.fromJSON(object.error)
        : undefined
    message.AddedToMempool =
      object.AddedToMempool !== undefined && object.AddedToMempool !== null
        ? MsgAddedToMempool.fromJSON(object.AddedToMempool)
        : undefined
    message.RemovedFromMempool =
      object.RemovedFromMempool !== undefined &&
      object.RemovedFromMempool !== null
        ? MsgRemovedFromMempool.fromJSON(object.RemovedFromMempool)
        : undefined
    message.Confirmed =
      object.Confirmed !== undefined && object.Confirmed !== null
        ? MsgConfirmed.fromJSON(object.Confirmed)
        : undefined
    message.Reorg =
      object.Reorg !== undefined && object.Reorg !== null
        ? MsgReorg.fromJSON(object.Reorg)
        : undefined
    return message
  },

  toJSON(message: SubscribeMsg): unknown {
    const obj: any = {}
    message.error !== undefined &&
      (obj.error = message.error ? Error.toJSON(message.error) : undefined)
    message.AddedToMempool !== undefined &&
      (obj.AddedToMempool = message.AddedToMempool
        ? MsgAddedToMempool.toJSON(message.AddedToMempool)
        : undefined)
    message.RemovedFromMempool !== undefined &&
      (obj.RemovedFromMempool = message.RemovedFromMempool
        ? MsgRemovedFromMempool.toJSON(message.RemovedFromMempool)
        : undefined)
    message.Confirmed !== undefined &&
      (obj.Confirmed = message.Confirmed
        ? MsgConfirmed.toJSON(message.Confirmed)
        : undefined)
    message.Reorg !== undefined &&
      (obj.Reorg = message.Reorg ? MsgReorg.toJSON(message.Reorg) : undefined)
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<SubscribeMsg>, I>>(
    object: I,
  ): SubscribeMsg {
    const message = { ...baseSubscribeMsg } as SubscribeMsg
    message.error =
      object.error !== undefined && object.error !== null
        ? Error.fromPartial(object.error)
        : undefined
    message.AddedToMempool =
      object.AddedToMempool !== undefined && object.AddedToMempool !== null
        ? MsgAddedToMempool.fromPartial(object.AddedToMempool)
        : undefined
    message.RemovedFromMempool =
      object.RemovedFromMempool !== undefined &&
      object.RemovedFromMempool !== null
        ? MsgRemovedFromMempool.fromPartial(object.RemovedFromMempool)
        : undefined
    message.Confirmed =
      object.Confirmed !== undefined && object.Confirmed !== null
        ? MsgConfirmed.fromPartial(object.Confirmed)
        : undefined
    message.Reorg =
      object.Reorg !== undefined && object.Reorg !== null
        ? MsgReorg.fromPartial(object.Reorg)
        : undefined
    return message
  },
}

const baseMsgAddedToMempool: object = {}

export const MsgAddedToMempool = {
  encode(
    message: MsgAddedToMempool,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.txid.length !== 0) {
      writer.uint32(10).bytes(message.txid)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): MsgAddedToMempool {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseMsgAddedToMempool } as MsgAddedToMempool
    message.txid = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.txid = reader.bytes()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): MsgAddedToMempool {
    const message = { ...baseMsgAddedToMempool } as MsgAddedToMempool
    message.txid =
      object.txid !== undefined && object.txid !== null
        ? bytesFromBase64(object.txid)
        : new Uint8Array()
    return message
  },

  toJSON(message: MsgAddedToMempool): unknown {
    const obj: any = {}
    message.txid !== undefined &&
      (obj.txid = base64FromBytes(
        message.txid !== undefined ? message.txid : new Uint8Array(),
      ))
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<MsgAddedToMempool>, I>>(
    object: I,
  ): MsgAddedToMempool {
    const message = { ...baseMsgAddedToMempool } as MsgAddedToMempool
    message.txid = object.txid ?? new Uint8Array()
    return message
  },
}

const baseMsgRemovedFromMempool: object = {}

export const MsgRemovedFromMempool = {
  encode(
    message: MsgRemovedFromMempool,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.txid.length !== 0) {
      writer.uint32(10).bytes(message.txid)
    }
    return writer
  },

  decode(
    input: _m0.Reader | Uint8Array,
    length?: number,
  ): MsgRemovedFromMempool {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseMsgRemovedFromMempool } as MsgRemovedFromMempool
    message.txid = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.txid = reader.bytes()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): MsgRemovedFromMempool {
    const message = { ...baseMsgRemovedFromMempool } as MsgRemovedFromMempool
    message.txid =
      object.txid !== undefined && object.txid !== null
        ? bytesFromBase64(object.txid)
        : new Uint8Array()
    return message
  },

  toJSON(message: MsgRemovedFromMempool): unknown {
    const obj: any = {}
    message.txid !== undefined &&
      (obj.txid = base64FromBytes(
        message.txid !== undefined ? message.txid : new Uint8Array(),
      ))
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<MsgRemovedFromMempool>, I>>(
    object: I,
  ): MsgRemovedFromMempool {
    const message = { ...baseMsgRemovedFromMempool } as MsgRemovedFromMempool
    message.txid = object.txid ?? new Uint8Array()
    return message
  },
}

const baseMsgConfirmed: object = {}

export const MsgConfirmed = {
  encode(
    message: MsgConfirmed,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.txid.length !== 0) {
      writer.uint32(10).bytes(message.txid)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): MsgConfirmed {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseMsgConfirmed } as MsgConfirmed
    message.txid = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.txid = reader.bytes()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): MsgConfirmed {
    const message = { ...baseMsgConfirmed } as MsgConfirmed
    message.txid =
      object.txid !== undefined && object.txid !== null
        ? bytesFromBase64(object.txid)
        : new Uint8Array()
    return message
  },

  toJSON(message: MsgConfirmed): unknown {
    const obj: any = {}
    message.txid !== undefined &&
      (obj.txid = base64FromBytes(
        message.txid !== undefined ? message.txid : new Uint8Array(),
      ))
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<MsgConfirmed>, I>>(
    object: I,
  ): MsgConfirmed {
    const message = { ...baseMsgConfirmed } as MsgConfirmed
    message.txid = object.txid ?? new Uint8Array()
    return message
  },
}

const baseMsgReorg: object = {}

export const MsgReorg = {
  encode(
    message: MsgReorg,
    writer: _m0.Writer = _m0.Writer.create(),
  ): _m0.Writer {
    if (message.txid.length !== 0) {
      writer.uint32(10).bytes(message.txid)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): MsgReorg {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseMsgReorg } as MsgReorg
    message.txid = new Uint8Array()
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.txid = reader.bytes()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): MsgReorg {
    const message = { ...baseMsgReorg } as MsgReorg
    message.txid =
      object.txid !== undefined && object.txid !== null
        ? bytesFromBase64(object.txid)
        : new Uint8Array()
    return message
  },

  toJSON(message: MsgReorg): unknown {
    const obj: any = {}
    message.txid !== undefined &&
      (obj.txid = base64FromBytes(
        message.txid !== undefined ? message.txid : new Uint8Array(),
      ))
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<MsgReorg>, I>>(object: I): MsgReorg {
    const message = { ...baseMsgReorg } as MsgReorg
    message.txid = object.txid ?? new Uint8Array()
    return message
  },
}

const baseError: object = { errorCode: "", msg: "", isUserError: false }

export const Error = {
  encode(message: Error, writer: _m0.Writer = _m0.Writer.create()): _m0.Writer {
    if (message.errorCode !== "") {
      writer.uint32(10).string(message.errorCode)
    }
    if (message.msg !== "") {
      writer.uint32(18).string(message.msg)
    }
    if (message.isUserError === true) {
      writer.uint32(24).bool(message.isUserError)
    }
    return writer
  },

  decode(input: _m0.Reader | Uint8Array, length?: number): Error {
    const reader = input instanceof _m0.Reader ? input : new _m0.Reader(input)
    let end = length === undefined ? reader.len : reader.pos + length
    const message = { ...baseError } as Error
    while (reader.pos < end) {
      const tag = reader.uint32()
      switch (tag >>> 3) {
        case 1:
          message.errorCode = reader.string()
          break
        case 2:
          message.msg = reader.string()
          break
        case 3:
          message.isUserError = reader.bool()
          break
        default:
          reader.skipType(tag & 7)
          break
      }
    }
    return message
  },

  fromJSON(object: any): Error {
    const message = { ...baseError } as Error
    message.errorCode =
      object.errorCode !== undefined && object.errorCode !== null
        ? String(object.errorCode)
        : ""
    message.msg =
      object.msg !== undefined && object.msg !== null ? String(object.msg) : ""
    message.isUserError =
      object.isUserError !== undefined && object.isUserError !== null
        ? Boolean(object.isUserError)
        : false
    return message
  },

  toJSON(message: Error): unknown {
    const obj: any = {}
    message.errorCode !== undefined && (obj.errorCode = message.errorCode)
    message.msg !== undefined && (obj.msg = message.msg)
    message.isUserError !== undefined && (obj.isUserError = message.isUserError)
    return obj
  },

  fromPartial<I extends Exact<DeepPartial<Error>, I>>(object: I): Error {
    const message = { ...baseError } as Error
    message.errorCode = object.errorCode ?? ""
    message.msg = object.msg ?? ""
    message.isUserError = object.isUserError ?? false
    return message
  },
}

declare var self: any | undefined
declare var window: any | undefined
declare var global: any | undefined
var globalThis: any = (() => {
  if (typeof globalThis !== "undefined") return globalThis
  if (typeof self !== "undefined") return self
  if (typeof window !== "undefined") return window
  if (typeof global !== "undefined") return global
  throw "Unable to locate global object"
})()

const atob: (b64: string) => string =
  globalThis.atob ||
  (b64 => globalThis.Buffer.from(b64, "base64").toString("binary"))
function bytesFromBase64(b64: string): Uint8Array {
  const bin = atob(b64)
  const arr = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; ++i) {
    arr[i] = bin.charCodeAt(i)
  }
  return arr
}

const btoa: (bin: string) => string =
  globalThis.btoa ||
  (bin => globalThis.Buffer.from(bin, "binary").toString("base64"))
function base64FromBytes(arr: Uint8Array): string {
  const bin: string[] = []
  for (const byte of arr) {
    bin.push(String.fromCharCode(byte))
  }
  return btoa(bin.join(""))
}

type Builtin =
  | Date
  | Function
  | Uint8Array
  | string
  | number
  | boolean
  | undefined

export type DeepPartial<T> = T extends Builtin
  ? T
  : T extends Long
  ? string | number | Long
  : T extends Array<infer U>
  ? Array<DeepPartial<U>>
  : T extends ReadonlyArray<infer U>
  ? ReadonlyArray<DeepPartial<U>>
  : T extends {}
  ? { [K in keyof T]?: DeepPartial<T[K]> }
  : Partial<T>

type KeysOfUnion<T> = T extends T ? keyof T : never
export type Exact<P, I extends P> = P extends Builtin
  ? P
  : P & { [K in keyof P]: Exact<P[K], I[K]> } & Record<
        Exclude<keyof I, KeysOfUnion<P>>,
        never
      >

if (_m0.util.Long !== Long) {
  _m0.util.Long = Long as any
  _m0.configure()
}

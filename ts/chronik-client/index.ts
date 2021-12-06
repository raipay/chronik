import axios, { AxiosResponse } from "axios"
import WebSocket from "isomorphic-ws"
import Long from "long"
import * as ws from "ws"
import * as proto from "./chronik"
import { fromHex, fromHexRev, toHex, toHexRev } from "./hex"

type MessageEvent = ws.MessageEvent | { data: Blob }

export class ChronikClient {
  private _url: string
  private _wsUrl: string

  constructor(url: string) {
    this._url = url
    if (url.endsWith("/")) {
      throw new Error("`url` cannot end with '/', got: " + url)
    }
    if (url.startsWith("https://")) {
      this._wsUrl = "wss://" + url.substr("https://".length)
    } else if (url.startsWith("http://")) {
      this._wsUrl = "ws://" + url.substr("http://".length)
    } else {
      throw new Error(
        "`url` must start with 'https://' or 'http://', got: " + url,
      )
    }
  }

  public async block(hashOrHeight: string | number): Promise<Block> {
    const data = await _get(this._url, `/block/${hashOrHeight}`)
    const block = proto.Block.decode(data)
    return convertToBlock(block)
  }

  public async blocks(
    startHeight: number,
    endHeight: number,
  ): Promise<BlockInfo[]> {
    const data = await _get(this._url, `/blocks/${startHeight}/${endHeight}`)
    const blocks = proto.Blocks.decode(data)
    return blocks.blocks.map(convertToBlockInfo)
  }

  public async tx(txid: string): Promise<Tx> {
    const data = await _get(this._url, `/tx/${txid}`)
    const tx = proto.Tx.decode(data)
    return convertToTx(tx)
  }

  public async validateUtxos(outpoints: OutPoint[]): Promise<UtxoState[]> {
    const request = proto.ValidateUtxoRequest.encode({
      outpoints: outpoints.map(outpoint => ({
        txid: fromHexRev(outpoint.txid),
        outIdx: outpoint.outIdx,
      })),
    }).finish()
    const data = await _post(this._url, "/validate-utxos", request)
    const validationStates = proto.ValidateUtxoResponse.decode(data)
    return validationStates.utxoStates.map(state => ({
      height: state.height,
      isConfirmed: state.isConfirmed,
      state: convertToUtxoStateVariant(state.state),
    }))
  }

  public script(scriptType: ScriptType, scriptPayload: string): ScriptEndpoint {
    return new ScriptEndpoint(this._url, scriptType, scriptPayload)
  }

  public ws(config: WsConfig): WsEndpoint {
    return new WsEndpoint(`${this._wsUrl}/ws`, config)
  }
}

export class ScriptEndpoint {
  private _url: string
  private _scriptType: string
  private _scriptPayload: string

  constructor(url: string, scriptType: string, scriptPayload: string) {
    this._url = url
    this._scriptType = scriptType
    this._scriptPayload = scriptPayload
  }

  public async history(
    page?: number,
    pageSize?: number,
  ): Promise<TxHistoryPage> {
    const query =
      page !== undefined && pageSize !== undefined
        ? `?page=${page}&page_size=${pageSize}`
        : page !== undefined
        ? `?page=${page}`
        : pageSize !== undefined
        ? `?page_size=${pageSize}`
        : ""
    const data = await _get(
      this._url,
      `/script/${this._scriptType}/${this._scriptPayload}/history${query}`,
    )
    const historyPage = proto.TxHistoryPage.decode(data)
    return {
      txs: historyPage.txs.map(convertToTx),
      numPages: historyPage.numPages,
    }
  }

  public async utxos(): Promise<ScriptUtxos[]> {
    const data = await _get(
      this._url,
      `/script/${this._scriptType}/${this._scriptPayload}/utxos`,
    )
    const utxos = proto.Utxos.decode(data)
    return utxos.scriptUtxos.map(scriptUtxos => ({
      outputScript: toHex(scriptUtxos.outputScript),
      utxos: scriptUtxos.utxos.map(convertToUtxo),
    }))
  }
}

export interface WsConfig {
  onMessage?: (msg: SubscribeMsg) => void
  onReconnect?: (e: ws.Event) => void
}

export class WsEndpoint {
  private _ws: ws.WebSocket | undefined
  private _wsUrl: string
  private _connected: Promise<ws.Event> | undefined
  private _config: WsConfig
  private _closed: boolean
  private _subs: { scriptType: ScriptType; scriptPayload: string }[]

  constructor(wsUrl: string, config: WsConfig) {
    this._closed = false
    this._subs = []
    this._config = config
    this._wsUrl = wsUrl
    this._connect()
  }

  public async waitForOpen() {
    await this._connected
  }

  public subscribe(scriptType: ScriptType, scriptPayload: string) {
    this._subs.push({ scriptType, scriptPayload })
    this._subUnsub(true, scriptType, scriptPayload)
  }

  public unsubscribe(scriptType: ScriptType, scriptPayload: string) {
    this._subs = this._subs.filter(
      sub =>
        sub.scriptType !== scriptType || sub.scriptPayload !== scriptPayload,
    )
    this._subUnsub(false, scriptType, scriptPayload)
  }

  public close() {
    this._closed = true
    this._ws?.close()
  }

  private _connect() {
    const ws: ws.WebSocket = new WebSocket(this._wsUrl)
    this._ws = ws
    this._connected = new Promise(resolved => {
      ws.onopen = msg => {
        this._subs.forEach(sub =>
          this._subUnsub(true, sub.scriptType, sub.scriptPayload),
        )
        resolved(msg)
      }
    })
    ws.onmessage = e => this._handleMsg(e as MessageEvent)
    ws.onclose = e => {
      if (this._closed) return
      if (this._config.onReconnect !== undefined) {
        this._config.onReconnect(e)
      }
      this._connect()
    }
  }

  private _subUnsub(
    isSubscribe: boolean,
    scriptType: ScriptType,
    scriptPayload: string,
  ) {
    const encodedSubscription = proto.Subscription.encode({
      isSubscribe,
      scriptType,
      payload: fromHex(scriptPayload),
    }).finish()
    if (this._ws === undefined)
      throw new Error("Invalid state; _ws is undefined")
    this._ws.send(encodedSubscription)
  }

  private async _handleMsg(wsMsg: MessageEvent) {
    if (this._config.onMessage === undefined) return
    const data =
      wsMsg.data instanceof Buffer
        ? (wsMsg.data as Uint8Array)
        : new Uint8Array(await (wsMsg.data as Blob).arrayBuffer())
    const msg = proto.SubscribeMsg.decode(data)
    if (msg.error) {
      this._config.onMessage({
        type: "Error",
        ...msg.error,
      })
    } else if (msg.AddedToMempool) {
      this._config.onMessage({
        type: "AddedToMempool",
        txid: toHexRev(msg.AddedToMempool.txid),
      })
    } else if (msg.RemovedFromMempool) {
      this._config.onMessage({
        type: "RemovedFromMempool",
        txid: toHexRev(msg.RemovedFromMempool.txid),
      })
    } else if (msg.Confirmed) {
      this._config.onMessage({
        type: "Confirmed",
        txid: toHexRev(msg.Confirmed.txid),
      })
    } else if (msg.Reorg) {
      this._config.onMessage({
        type: "Reorg",
        txid: toHexRev(msg.Reorg.txid),
      })
    } else {
      throw new Error(`Unknown message: ${msg}`)
    }
  }
}

async function _get(url: string, path: string): Promise<Uint8Array> {
  const response = await axios.get(`${url}${path}`, {
    responseType: "arraybuffer",
    validateStatus: undefined,
  })
  ensureResponseErrorThrown(response, path)
  return new Uint8Array(response.data)
}

async function _post(
  url: string,
  path: string,
  data: Uint8Array,
): Promise<Uint8Array> {
  const response = await axios.post(`${url}${path}`, data, {
    responseType: "arraybuffer",
    validateStatus: undefined,
    headers: {
      "Content-Type": "application/x-protobuf",
    },
  })
  ensureResponseErrorThrown(response, path)
  return new Uint8Array(response.data)
}

function ensureResponseErrorThrown(response: AxiosResponse, path: string) {
  if (response.status != 200) {
    const error = proto.Error.decode(new Uint8Array(response.data))
    throw new Error(`Failed getting ${path} (${error.errorCode}): ${error.msg}`)
  }
}

function convertToBlock(block: proto.Block): Block {
  if (block.blockInfo === undefined) {
    throw new Error("Block has no blockInfo")
  }
  return {
    blockInfo: convertToBlockInfo(block.blockInfo),
    txs: block.txs.map(convertToTx),
  }
}

function convertToTx(tx: proto.Tx): Tx {
  return {
    txid: toHexRev(tx.txid),
    version: tx.version,
    inputs: tx.inputs.map(convertToTxInput),
    outputs: tx.outputs.map(convertToTxOutput),
    lockTime: tx.lockTime,
    slpTxData: tx.slpTxData ? convertToSlpTxData(tx.slpTxData) : undefined,
    slpErrorMsg: tx.slpErrorMsg.length !== 0 ? tx.slpErrorMsg : undefined,
    block: tx.block !== undefined ? convertToBlockMeta(tx.block) : undefined,
    timeFirstSeen: tx.timeFirstSeen,
    network: convertToNetwork(tx.network),
  }
}

function convertToUtxo(utxo: proto.Utxo): Utxo {
  if (utxo.outpoint === undefined) {
    throw new Error("UTXO outpoint is undefined")
  }
  return {
    outpoint: {
      txid: toHexRev(utxo.outpoint.txid),
      outIdx: utxo.outpoint.outIdx,
    },
    blockHeight: utxo.blockHeight,
    isCoinbase: utxo.isCoinbase,
    value: utxo.value,
    slpMeta:
      utxo.slpMeta !== undefined ? convertToSlpMeta(utxo.slpMeta) : undefined,
    slpToken:
      utxo.slpToken !== undefined
        ? convertToSlpToken(utxo.slpToken)
        : undefined,
    network: convertToNetwork(utxo.network),
  }
}

function convertToTxInput(input: proto.TxInput): TxInput {
  if (input.prevOut === undefined) {
    throw new Error("Invalid proto, no prevOut")
  }
  return {
    prevOut: {
      txid: toHexRev(input.prevOut.txid),
      outIdx: input.prevOut.outIdx,
    },
    inputScript: toHex(input.inputScript),
    outputScript:
      input.outputScript.length > 0 ? toHex(input.outputScript) : undefined,
    value: input.value,
    sequenceNo: input.sequenceNo,
    slpBurn:
      input.slpBurn !== undefined ? convertToSlpBurn(input.slpBurn) : undefined,
    slpToken:
      input.slpToken !== undefined
        ? convertToSlpToken(input.slpToken)
        : undefined,
  }
}

function convertToTxOutput(output: proto.TxOutput): TxOutput {
  return {
    value: output.value,
    outputScript: toHex(output.outputScript),
    slpToken:
      output.slpToken !== undefined
        ? convertToSlpToken(output.slpToken)
        : undefined,
    spentBy:
      output.spentBy !== undefined
        ? {
            txid: toHex(output.spentBy.txid),
            outIdx: output.spentBy.outIdx,
          }
        : undefined,
  }
}

function convertToSlpTxData(slpTxData: proto.SlpTxData): SlpTxData {
  if (slpTxData.slpMeta === undefined) {
    throw new Error("Invalid slpTxData: slpMeta is undefined")
  }
  return {
    slpMeta: convertToSlpMeta(slpTxData.slpMeta),
    genesisInfo:
      slpTxData.genesisInfo !== undefined
        ? convertToSlpGenesisInfo(slpTxData.genesisInfo)
        : undefined,
  }
}

function convertToSlpMeta(slpMeta: proto.SlpMeta): SlpMeta {
  let tokenType: SlpTokenType
  switch (slpMeta.tokenType) {
    case proto.SlpTokenType.FUNGIBLE:
      tokenType = "FUNGIBLE"
      break
    case proto.SlpTokenType.NFT1_GROUP:
      tokenType = "NFT1_GROUP"
      break
    case proto.SlpTokenType.NFT1_CHILD:
      tokenType = "NFT1_CHILD"
      break
    case proto.SlpTokenType.UNKNOWN_TOKEN_TYPE:
      tokenType = "UNKNOWN_TOKEN_TYPE"
      break
    default:
      throw new Error(`Invalid token type: ${slpMeta.tokenType}`)
  }
  let txType: SlpTxType
  switch (slpMeta.txType) {
    case proto.SlpTxType.GENESIS:
      txType = "GENESIS"
      break
    case proto.SlpTxType.SEND:
      txType = "SEND"
      break
    case proto.SlpTxType.MINT:
      txType = "MINT"
      break
    case proto.SlpTxType.UNKNOWN_TX_TYPE:
      txType = "UNKNOWN_TX_TYPE"
      break
    default:
      throw new Error(`Invalid token type: ${slpMeta.txType}`)
  }
  return {
    tokenType,
    txType,
    tokenId: toHex(slpMeta.tokenId),
    groupTokenId:
      slpMeta.groupTokenId.length == 32
        ? toHex(slpMeta.groupTokenId)
        : undefined,
  }
}

function convertToSlpGenesisInfo(info: proto.SlpGenesisInfo): SlpGenesisInfo {
  const decoder = new TextDecoder()
  return {
    tokenTicker: decoder.decode(info.tokenTicker),
    tokenName: decoder.decode(info.tokenName),
    tokenDocumentUrl: decoder.decode(info.tokenDocumentUrl),
    tokenDocumentHash: toHex(info.tokenDocumentHash),
    decimals: info.decimals,
  }
}

function convertToBlockMeta(block: proto.BlockMetadata): BlockMetadata {
  return {
    height: block.height,
    hash: toHexRev(block.hash),
    timestamp: block.timestamp,
  }
}

function convertToBlockInfo(block: proto.BlockInfo): BlockInfo {
  return {
    ...block,
    hash: toHexRev(block.hash),
    prevHash: toHexRev(block.prevHash),
  }
}

function convertToSlpBurn(burn: proto.SlpBurn): SlpBurn {
  if (burn.token === undefined) {
    throw new Error("Invalid burn: token is undefined")
  }
  return {
    token: convertToSlpToken(burn.token),
    tokenId: toHex(burn.tokenId),
  }
}

function convertToSlpToken(token: proto.SlpToken): SlpToken {
  return {
    amount: token.amount,
    isMintBaton: token.isMintBaton,
  }
}

function convertToNetwork(network: proto.Network): Network {
  switch (network) {
    case proto.Network.BCH:
      return "BCH"
    case proto.Network.XEC:
      return "XEC"
    case proto.Network.XPI:
      return "XPI"
    case proto.Network.XRG:
      return "XRG"
    default:
      throw new Error(`Unknown network: ${network}`)
  }
}

function convertToUtxoStateVariant(
  variant: proto.UtxoStateVariant,
): UtxoStateVariant {
  switch (variant) {
    case proto.UtxoStateVariant.UNSPENT:
      return "UNSPENT"
    case proto.UtxoStateVariant.SPENT:
      return "SPENT"
    case proto.UtxoStateVariant.NO_SUCH_TX:
      return "NO_SUCH_TX"
    case proto.UtxoStateVariant.NO_SUCH_OUTPUT:
      return "NO_SUCH_OUTPUT"
    default:
      throw new Error(`Unknown UtxoStateVariant: ${variant}`)
  }
}

export interface Tx {
  txid: string
  version: number
  inputs: TxInput[]
  outputs: TxOutput[]
  lockTime: number
  slpTxData: SlpTxData | undefined
  slpErrorMsg: string | undefined
  block: BlockMetadata | undefined
  timeFirstSeen: Long
  network: Network
}

export interface Utxo {
  outpoint: OutPoint
  blockHeight: number
  isCoinbase: boolean
  value: Long
  slpMeta: SlpMeta | undefined
  slpToken: SlpToken | undefined
  network: Network
}

export interface BlockInfo {
  hash: string
  prevHash: string
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
  blockInfo: BlockInfo
  txs: Tx[]
}

export interface ScriptUtxos {
  outputScript: string
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
  slpMeta: SlpMeta
  genesisInfo: SlpGenesisInfo | undefined
}

export interface SlpMeta {
  tokenType: SlpTokenType
  txType: SlpTxType
  tokenId: string
  groupTokenId: string | undefined
}

export interface TxInput {
  prevOut: OutPoint
  inputScript: string
  outputScript: string | undefined
  value: Long
  sequenceNo: number
  slpBurn: SlpBurn | undefined
  slpToken: SlpToken | undefined
}

export interface TxOutput {
  value: Long
  outputScript: string
  slpToken: SlpToken | undefined
  spentBy: OutPoint | undefined
}

export interface BlockMetadata {
  height: number
  hash: string
  timestamp: Long
}

export interface OutPoint {
  txid: string
  outIdx: number
}

export interface SlpToken {
  amount: Long
  isMintBaton: boolean
}

export interface SlpBurn {
  token: SlpToken
  tokenId: string
}

export interface SlpGenesisInfo {
  tokenTicker: string
  tokenName: string
  tokenDocumentUrl: string
  tokenDocumentHash: string
  decimals: number
}

export interface UtxoState {
  height: number
  isConfirmed: boolean
  state: UtxoStateVariant
}

export interface Subscription {
  scriptType: string
  payload: string
  isSubscribe: boolean
}

export type SubscribeMsg =
  | Error
  | MsgAddedToMempool
  | MsgRemovedFromMempool
  | MsgConfirmed
  | MsgReorg

export interface MsgAddedToMempool {
  type: "AddedToMempool"
  txid: string
}

export interface MsgRemovedFromMempool {
  type: "RemovedFromMempool"
  txid: string
}

export interface MsgConfirmed {
  type: "Confirmed"
  txid: string
}

export interface MsgReorg {
  type: "Reorg"
  txid: string
}

export interface Error {
  type: "Error"
  errorCode: string
  msg: string
  isUserError: boolean
}

export type Network = "BCH" | "XEC" | "XPI" | "XRG"

export type SlpTxType = "GENESIS" | "SEND" | "MINT" | "UNKNOWN_TX_TYPE"

export type SlpTokenType =
  | "FUNGIBLE"
  | "NFT1_GROUP"
  | "NFT1_CHILD"
  | "UNKNOWN_TOKEN_TYPE"

export type UtxoStateVariant =
  | "UNSPENT"
  | "SPENT"
  | "NO_SUCH_TX"
  | "NO_SUCH_OUTPUT"

export type ScriptType =
  | "other"
  | "p2pk"
  | "p2pkh"
  | "p2sh"
  | "p2tr-commitment"
  | "p2tr-state"

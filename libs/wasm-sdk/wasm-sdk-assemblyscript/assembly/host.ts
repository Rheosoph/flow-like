import { parseExecutionInputJson } from "./json";
import { ABI_VERSION, type ExecutionInput, type ExecutionResult, type LogLevel, type NodeDefinition } from "./types";

// -- flowlike_log --

@external("flowlike_log", "trace")
declare function _log_trace(ptr: i32, len: i32): void;
@external("flowlike_log", "debug")
declare function _log_debug(ptr: i32, len: i32): void;
@external("flowlike_log", "info")
declare function _log_info(ptr: i32, len: i32): void;
@external("flowlike_log", "warn")
declare function _log_warn(ptr: i32, len: i32): void;
@external("flowlike_log", "error")
declare function _log_error(ptr: i32, len: i32): void;
@external("flowlike_log", "log_json")
declare function _log_json(level: i32, msg_ptr: i32, msg_len: i32, data_ptr: i32, data_len: i32): void;

// -- flowlike_pins --

@external("flowlike_pins", "get_input")
declare function _get_input(name_ptr: i32, name_len: i32): i64;
@external("flowlike_pins", "set_output")
declare function _set_output(name_ptr: i32, name_len: i32, value_ptr: i32, value_len: i32): void;
@external("flowlike_pins", "activate_exec")
declare function _activate_exec(name_ptr: i32, name_len: i32): void;

// -- flowlike_vars --

@external("flowlike_vars", "get")
declare function _var_get(name_ptr: i32, name_len: i32): i64;
@external("flowlike_vars", "set")
declare function _var_set(name_ptr: i32, name_len: i32, value_ptr: i32, value_len: i32): void;
@external("flowlike_vars", "delete")
declare function _var_delete(name_ptr: i32, name_len: i32): void;
@external("flowlike_vars", "has")
declare function _var_has(name_ptr: i32, name_len: i32): i32;

// -- flowlike_cache --

@external("flowlike_cache", "get")
declare function _cache_get(key_ptr: i32, key_len: i32): i64;
@external("flowlike_cache", "set")
declare function _cache_set(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32): void;
@external("flowlike_cache", "delete")
declare function _cache_delete(key_ptr: i32, key_len: i32): void;
@external("flowlike_cache", "has")
declare function _cache_has(key_ptr: i32, key_len: i32): i32;

// -- flowlike_meta --

@external("flowlike_meta", "get_node_id")
declare function _get_node_id(): i64;
@external("flowlike_meta", "get_run_id")
declare function _get_run_id(): i64;
@external("flowlike_meta", "get_app_id")
declare function _get_app_id(): i64;
@external("flowlike_meta", "get_board_id")
declare function _get_board_id(): i64;
@external("flowlike_meta", "get_user_id")
declare function _get_user_id(): i64;
@external("flowlike_meta", "is_streaming")
declare function _is_streaming(): i32;
@external("flowlike_meta", "get_log_level")
declare function _get_log_level(): i32;
@external("flowlike_meta", "time_now")
declare function _time_now(): i64;
@external("flowlike_meta", "random")
declare function _random(): i64;

// -- flowlike_storage --

@external("flowlike_storage", "read_request")
declare function _storage_read(path_ptr: i32, path_len: i32): i64;
@external("flowlike_storage", "write_request")
declare function _storage_write(path_ptr: i32, path_len: i32, data_ptr: i32, data_len: i32): i32;
@external("flowlike_storage", "storage_dir")
declare function _storage_dir(node_scoped: i32): i64;
@external("flowlike_storage", "upload_dir")
declare function _upload_dir(): i64;
@external("flowlike_storage", "cache_dir")
declare function _cache_dir(node_scoped: i32, user_scoped: i32): i64;
@external("flowlike_storage", "user_dir")
declare function _user_dir(node_scoped: i32): i64;
@external("flowlike_storage", "list_request")
declare function _storage_list(path_ptr: i32, path_len: i32): i64;

// -- flowlike_models --

@external("flowlike_models", "embed_text")
declare function _embed_text(bit_ptr: i32, bit_len: i32, texts_ptr: i32, texts_len: i32): i64;

// -- flowlike_http --

@external("flowlike_http", "request")
declare function _http_request(method: i32, url_ptr: i32, url_len: i32, headers_ptr: i32, headers_len: i32, body_ptr: i32, body_len: i32): i32;

// -- flowlike_stream --

@external("flowlike_stream", "emit")
declare function _stream_emit(event_ptr: i32, event_len: i32, data_ptr: i32, data_len: i32): void;
@external("flowlike_stream", "text")
declare function _stream_text(text_ptr: i32, text_len: i32): void;

// -- flowlike_auth --

@external("flowlike_auth", "get_oauth_token")
declare function _get_oauth_token(provider_ptr: i32, provider_len: i32): i64;
@external("flowlike_auth", "has_oauth_token")
declare function _has_oauth_token(provider_ptr: i32, provider_len: i32): i32;

// ============================================================================
// Unpacking helpers for host-returned packed u64 (ptr|len)
// ============================================================================

function unpackString(packed: i64): string | null {
  if (packed == 0) return null;
  const ptr = i32(packed >> 32);
  const len = i32(packed & 0xFFFFFFFF);
  if (ptr == 0 || len == 0) return null;
  const buf = new Uint8Array(len);
  memory.copy(changetype<i32>(buf.buffer), ptr, len);
  return String.UTF8.decode(buf.buffer);
}

function unpackBytes(packed: i64): ArrayBuffer | null {
  if (packed == 0) return null;
  const ptr = i32(packed >> 32);
  const len = i32(packed & 0xFFFFFFFF);
  if (ptr == 0 || len == 0) return null;
  const buf = new ArrayBuffer(len);
  memory.copy(changetype<i32>(buf), ptr, len);
  return buf;
}

// ============================================================================
// Logging
// ============================================================================

export function trace(message: string): void {
  const buf = String.UTF8.encode(message);
  _log_trace(changetype<i32>(buf), buf.byteLength);
}

export function debug(message: string): void {
  const buf = String.UTF8.encode(message);
  _log_debug(changetype<i32>(buf), buf.byteLength);
}

export function info(message: string): void {
  const buf = String.UTF8.encode(message);
  _log_info(changetype<i32>(buf), buf.byteLength);
}

export function warn(message: string): void {
  const buf = String.UTF8.encode(message);
  _log_warn(changetype<i32>(buf), buf.byteLength);
}

export function error(message: string): void {
  const buf = String.UTF8.encode(message);
  _log_error(changetype<i32>(buf), buf.byteLength);
}

export function logJson(level: LogLevel, message: string, data: string): void {
  const msgBuf = String.UTF8.encode(message);
  const dataBuf = String.UTF8.encode(data);
  _log_json(level, changetype<i32>(msgBuf), msgBuf.byteLength, changetype<i32>(dataBuf), dataBuf.byteLength);
}

// ============================================================================
// Pins
// ============================================================================

export function getInput(name: string): string | null {
  const buf = String.UTF8.encode(name);
  const result = _get_input(changetype<i32>(buf), buf.byteLength);
  return unpackString(result);
}

export function setOutput(name: string, value: string): void {
  const nameBuf = String.UTF8.encode(name);
  const valueBuf = String.UTF8.encode(value);
  _set_output(changetype<i32>(nameBuf), nameBuf.byteLength, changetype<i32>(valueBuf), valueBuf.byteLength);
}

export function activateExec(name: string): void {
  const buf = String.UTF8.encode(name);
  _activate_exec(changetype<i32>(buf), buf.byteLength);
}

// ============================================================================
// Variables
// ============================================================================

export function getVariable(name: string): string | null {
  const buf = String.UTF8.encode(name);
  const result = _var_get(changetype<i32>(buf), buf.byteLength);
  return unpackString(result);
}

export function setVariable(name: string, value: string): void {
  const nameBuf = String.UTF8.encode(name);
  const valueBuf = String.UTF8.encode(value);
  _var_set(changetype<i32>(nameBuf), nameBuf.byteLength, changetype<i32>(valueBuf), valueBuf.byteLength);
}

export function deleteVariable(name: string): void {
  const buf = String.UTF8.encode(name);
  _var_delete(changetype<i32>(buf), buf.byteLength);
}

export function hasVariable(name: string): bool {
  const buf = String.UTF8.encode(name);
  return _var_has(changetype<i32>(buf), buf.byteLength) != 0;
}

// ============================================================================
// Cache
// ============================================================================

export function cacheGet(key: string): string | null {
  const buf = String.UTF8.encode(key);
  const result = _cache_get(changetype<i32>(buf), buf.byteLength);
  return unpackString(result);
}

export function cacheSet(key: string, value: string): void {
  const keyBuf = String.UTF8.encode(key);
  const valBuf = String.UTF8.encode(value);
  _cache_set(changetype<i32>(keyBuf), keyBuf.byteLength, changetype<i32>(valBuf), valBuf.byteLength);
}

export function cacheDelete(key: string): void {
  const buf = String.UTF8.encode(key);
  _cache_delete(changetype<i32>(buf), buf.byteLength);
}

export function cacheHas(key: string): bool {
  const buf = String.UTF8.encode(key);
  return _cache_has(changetype<i32>(buf), buf.byteLength) != 0;
}

// ============================================================================
// Metadata
// ============================================================================

export function getNodeId(): string {
  const result = unpackString(_get_node_id());
  return result !== null ? result : "";
}

export function getRunId(): string {
  const result = unpackString(_get_run_id());
  return result !== null ? result : "";
}

export function getAppId(): string {
  const result = unpackString(_get_app_id());
  return result !== null ? result : "";
}

export function getBoardId(): string {
  const result = unpackString(_get_board_id());
  return result !== null ? result : "";
}

export function getUserId(): string {
  const result = unpackString(_get_user_id());
  return result !== null ? result : "";
}

export function isStreaming(): bool {
  return _is_streaming() != 0;
}

export function getLogLevel(): i32 {
  return _get_log_level();
}

export function now(): i64 {
  return _time_now();
}

export function random(): i64 {
  return _random();
}

// ============================================================================
// Storage
// ============================================================================

export function storageRead(path: string): ArrayBuffer | null {
  const buf = String.UTF8.encode(path);
  const result = _storage_read(changetype<i32>(buf), buf.byteLength);
  return unpackBytes(result);
}

export function storageWrite(path: string, data: ArrayBuffer): bool {
  const pathBuf = String.UTF8.encode(path);
  return _storage_write(changetype<i32>(pathBuf), pathBuf.byteLength, changetype<i32>(data), data.byteLength) != 0;
}

export function storageDir(nodeScoped: bool): string | null {
  return unpackString(_storage_dir(nodeScoped ? 1 : 0));
}

export function uploadDir(): string | null {
  return unpackString(_upload_dir());
}

export function cacheDir(nodeScoped: bool, userScoped: bool): string | null {
  return unpackString(_cache_dir(nodeScoped ? 1 : 0, userScoped ? 1 : 0));
}

export function userDir(nodeScoped: bool): string | null {
  return unpackString(_user_dir(nodeScoped ? 1 : 0));
}

export function storageList(flowPathJson: string): string | null {
  const buf = String.UTF8.encode(flowPathJson);
  const result = _storage_list(changetype<i32>(buf), buf.byteLength);
  return unpackString(result);
}

// ============================================================================
// Models
// ============================================================================

export function embedText(bitJson: string, textsJson: string): string | null {
  const bitBuf = String.UTF8.encode(bitJson);
  const textsBuf = String.UTF8.encode(textsJson);
  const result = _embed_text(
    changetype<i32>(bitBuf), bitBuf.byteLength,
    changetype<i32>(textsBuf), textsBuf.byteLength
  );
  return unpackString(result);
}

// ============================================================================
// HTTP
// ============================================================================

export function httpRequest(method: i32, url: string, headers: string, body: string): bool {
  const urlBuf = String.UTF8.encode(url);
  const headersBuf = String.UTF8.encode(headers);
  const bodyBuf = String.UTF8.encode(body);
  return _http_request(
    method,
    changetype<i32>(urlBuf), urlBuf.byteLength,
    changetype<i32>(headersBuf), headersBuf.byteLength,
    changetype<i32>(bodyBuf), bodyBuf.byteLength
  ) != 0;
}

// ============================================================================
// Streaming
// ============================================================================

export function stream(eventType: string, data: string): void {
  const typeBuf = String.UTF8.encode(eventType);
  const dataBuf = String.UTF8.encode(data);
  _stream_emit(changetype<i32>(typeBuf), typeBuf.byteLength, changetype<i32>(dataBuf), dataBuf.byteLength);
}

export function streamText(text: string): void {
  const buf = String.UTF8.encode(text);
  _stream_text(changetype<i32>(buf), buf.byteLength);
}

export function streamProgress(progress: f32, message: string): void {
  stream("progress", `{"progress":${progress},"message":"${message}"}`);
}

// ============================================================================
// Auth
// ============================================================================

export function getOAuthToken(provider: string): string | null {
  const buf = String.UTF8.encode(provider);
  const result = _get_oauth_token(changetype<i32>(buf), buf.byteLength);
  return unpackString(result);
}

export function hasOAuthToken(provider: string): bool {
  const buf = String.UTF8.encode(provider);
  return _has_oauth_token(changetype<i32>(buf), buf.byteLength) != 0;
}

// ============================================================================
// Memory Exports
// ============================================================================

let resultBuffer: ArrayBuffer = new ArrayBuffer(0);

export function alloc(size: i32): i32 {
  const buf = new ArrayBuffer(size);
  return changetype<i32>(buf);
}

export function dealloc(_ptr: i32, _size: i32): void {}

export function get_abi_version(): i32 {
  return ABI_VERSION;
}

export function packResult(json: string): i64 {
  const buf = String.UTF8.encode(json);
  resultBuffer = buf;
  const ptr = changetype<i32>(buf);
  const len = buf.byteLength;
  return (i64(ptr) << 32) | i64(len);
}

export function serializeDefinition(def: NodeDefinition): i64 {
  return packResult(def.toJSON());
}

export function serializeResult(result: ExecutionResult): i64 {
  return packResult(result.toJSON());
}

export function parseInput(ptr: i32, len: i32): ExecutionInput {
  const buf = new Uint8Array(len);
  memory.copy(changetype<i32>(buf.buffer), ptr, len);
  const json = String.UTF8.decode(buf.buffer);
  return parseExecutionInputJson(json);
}

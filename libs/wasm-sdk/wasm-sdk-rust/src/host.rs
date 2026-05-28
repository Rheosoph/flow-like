//! Host function bindings for WASM nodes (Component Model)
//!
//! Thin wrappers around WIT-generated imports. During tests, these delegate
//! to the `wit_stub` module which provides no-op implementations.

// In production builds, the WIT bindings live under `crate::flow_like::node::*`.
// In test builds they come from `crate::wit_stub::flow_like::node::*`.
#[cfg(target_arch = "wasm32")]
use crate::flow_like::node::{
    auth, cache, db, http, image, logging, metadata, models, schema, storage, streaming, variables,
    websocket,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::wit_stub::flow_like::node::{
    auth, cache, db, http, image, logging, metadata, models, schema, storage, streaming, variables,
    websocket,
};

// ============================================================================
// Logging
// ============================================================================

pub fn debug(message: &str) {
    logging::log(0, message);
}

pub fn info(message: &str) {
    logging::log(1, message);
}

pub fn warn(message: &str) {
    logging::log(2, message);
}

pub fn error(message: &str) {
    logging::log(3, message);
}

pub fn fatal(message: &str) {
    logging::log(4, message);
}

pub fn log_json(level: u8, message: &str, data: &serde_json::Value) {
    let combined = serde_json::json!({
        "message": message,
        "data": data
    });
    let json_str = serde_json::to_string(&combined).unwrap_or_default();
    logging::log(level, &json_str);
}

// ============================================================================
// Streaming
// ============================================================================

pub fn stream(event_type: &str, data: &str) {
    streaming::emit(event_type, data);
}

pub fn stream_text(text: &str) {
    stream("text", text);
}

pub fn stream_text_raw(text: &str) {
    streaming::text(text);
}

pub fn stream_json<T: serde::Serialize>(data: &T) {
    if let Ok(json) = serde_json::to_string(data) {
        stream("json", &json);
    }
}

pub fn stream_progress(progress: f32, message: &str) {
    let data = serde_json::json!({
        "progress": progress,
        "message": message
    });
    stream("progress", &data.to_string());
}

// ============================================================================
// Variables
// ============================================================================

pub fn get_variable(name: &str) -> Option<serde_json::Value> {
    variables::get_var(name).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn set_variable(name: &str, value: &serde_json::Value) -> bool {
    let json = serde_json::to_string(value).unwrap_or_default();
    variables::set_var(name, &json);
    true
}

pub fn delete_variable(name: &str) {
    variables::delete_var(name);
}

pub fn has_variable(name: &str) -> bool {
    variables::has_var(name)
}

// ============================================================================
// Cache
// ============================================================================

pub fn cache_get(key: &str) -> Option<serde_json::Value> {
    cache::cache_get(key).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn cache_set(key: &str, value: &serde_json::Value) {
    let json = serde_json::to_string(value).unwrap_or_default();
    cache::cache_set(key, &json);
}

pub fn cache_delete(key: &str) {
    cache::cache_delete(key);
}

pub fn cache_has(key: &str) -> bool {
    cache::cache_has(key)
}

// ============================================================================
// Metadata
// ============================================================================

pub fn get_node_id_from_host() -> Option<String> {
    Some(metadata::get_node_id())
}

pub fn get_run_id_from_host() -> Option<String> {
    Some(metadata::get_run_id())
}

pub fn get_app_id_from_host() -> Option<String> {
    Some(metadata::get_app_id())
}

pub fn get_board_id_from_host() -> Option<String> {
    Some(metadata::get_board_id())
}

pub fn get_user_id_from_host() -> Option<String> {
    Some(metadata::get_user_id())
}

pub fn is_streaming_from_host() -> bool {
    metadata::is_streaming()
}

pub fn get_log_level_from_host() -> u8 {
    metadata::get_log_level()
}

// ============================================================================
// Storage
// ============================================================================

pub fn storage_dir(node_scoped: bool) -> Option<serde_json::Value> {
    storage::storage_dir(node_scoped).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn upload_dir() -> Option<serde_json::Value> {
    storage::upload_dir().and_then(|s| serde_json::from_str(&s).ok())
}

pub fn cache_dir(node_scoped: bool, user_scoped: bool) -> Option<serde_json::Value> {
    storage::cache_dir(node_scoped, user_scoped).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn user_dir(node_scoped: bool) -> Option<serde_json::Value> {
    storage::user_dir(node_scoped).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn storage_read(flow_path_json: &str) -> Option<Vec<u8>> {
    storage::read_file(flow_path_json)
}

pub fn storage_write(flow_path_json: &str, data: &[u8]) -> bool {
    storage::write_file(flow_path_json, data)
}

pub fn storage_write_start(flow_path_json: &str, total_size: u64) -> Option<String> {
    storage::write_file_start(flow_path_json, total_size)
}

pub fn storage_write_chunk(write_id: &str, data: &[u8]) -> bool {
    storage::write_file_chunk(write_id, data)
}

pub fn storage_write_finish(write_id: &str) -> bool {
    storage::write_file_finish(write_id)
}

pub fn storage_list(flow_path_json: &str) -> Option<Vec<serde_json::Value>> {
    storage::list_files(flow_path_json).and_then(|s| serde_json::from_str(&s).ok())
}

// ============================================================================
// Models
// ============================================================================

pub fn embed_text(bit_json: &str, texts: &[String]) -> Option<Vec<Vec<f32>>> {
    let texts_json = serde_json::to_string(texts).ok()?;
    models::embed_text(bit_json, &texts_json).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn embed_text_query(model_json: &str, texts: &[String]) -> Option<Vec<Vec<f32>>> {
    let texts_json = serde_json::to_string(texts).ok()?;
    models::embed_text_query(model_json, &texts_json).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn embed_text_document(model_json: &str, texts: &[String]) -> Option<Vec<Vec<f32>>> {
    let texts_json = serde_json::to_string(texts).ok()?;
    models::embed_text_document(model_json, &texts_json).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn embed_image(model_json: &str, image_json: &str) -> Option<Vec<f32>> {
    models::embed_image(model_json, image_json.as_bytes())
        .and_then(|s| serde_json::from_str(&s).ok())
}

pub fn llm_prompt(bit_json: &str, messages_json: &str, do_stream: bool) -> Option<String> {
    models::llm_prompt(bit_json, messages_json, do_stream)
}

pub fn llm_prompt_stream(bit_json: &str, request_json: &str) -> Option<String> {
    models::llm_prompt_stream(bit_json, request_json)
}

// ============================================================================
// Schema
// ============================================================================

pub fn get_type_schema(type_name: &str) -> Option<String> {
    schema::get_type_schema(type_name)
}

pub fn list_types() -> Option<Vec<String>> {
    schema::list_types().and_then(|s| serde_json::from_str(&s).ok())
}

// ============================================================================
// Image
// ============================================================================

pub fn image_from_bytes(data: &[u8], format: &str) -> Option<String> {
    image::from_bytes(data, format)
}

pub fn image_to_bytes(image_ref_json: &str, format: &str) -> Option<Vec<u8>> {
    image::to_bytes(image_ref_json, format)
}

// ============================================================================
// Database
// ============================================================================

const DB_OP_VECTOR_SEARCH: u32 = 1;
const DB_OP_FTS_SEARCH: u32 = 2;
const DB_OP_HYBRID_SEARCH: u32 = 3;
const DB_OP_INSERT: u32 = 4;
const DB_OP_UPSERT: u32 = 5;
const DB_OP_DELETE: u32 = 6;
const DB_OP_LIST: u32 = 7;
const DB_OP_COUNT: u32 = 8;

fn db_call(op: u32, conn_json: &str, payload_json: &str) -> Option<String> {
    db::query(op, conn_json, payload_json)
}

pub fn db_vector_search(conn_json: &str, query_json: &str) -> Option<Vec<serde_json::Value>> {
    db_call(DB_OP_VECTOR_SEARCH, conn_json, query_json).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn db_fts_search(conn_json: &str, query_json: &str) -> Option<Vec<serde_json::Value>> {
    db_call(DB_OP_FTS_SEARCH, conn_json, query_json).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn db_hybrid_search(conn_json: &str, query_json: &str) -> Option<Vec<serde_json::Value>> {
    db_call(DB_OP_HYBRID_SEARCH, conn_json, query_json).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn db_insert(conn_json: &str, payload_json: &str) -> bool {
    db_call(DB_OP_INSERT, conn_json, payload_json).is_some()
}

pub fn db_upsert(conn_json: &str, payload_json: &str) -> bool {
    db_call(DB_OP_UPSERT, conn_json, payload_json).is_some()
}

pub fn db_delete(conn_json: &str, payload_json: &str) -> bool {
    db_call(DB_OP_DELETE, conn_json, payload_json).is_some()
}

pub fn db_list(conn_json: &str, payload_json: &str) -> Option<Vec<serde_json::Value>> {
    db_call(DB_OP_LIST, conn_json, payload_json).and_then(|s| serde_json::from_str(&s).ok())
}

pub fn db_count(conn_json: &str, payload_json: &str) -> Option<u64> {
    db_call(DB_OP_COUNT, conn_json, payload_json).and_then(|s| serde_json::from_str(&s).ok())
}

// ============================================================================
// HTTP
// ============================================================================

pub fn http_request(method: u8, url: &str, headers: &str, body: &[u8]) -> Option<String> {
    let body_opt = if body.is_empty() { None } else { Some(body) };
    http::request(method, url, headers, body_opt)
}

// ============================================================================
// Auth
// ============================================================================

pub fn get_oauth_token(provider: &str) -> Option<String> {
    auth::get_oauth_token(provider)
}

pub fn has_oauth_token(provider: &str) -> bool {
    auth::has_oauth_token(provider)
}

// ============================================================================
// Utilities
// ============================================================================

pub fn now() -> u64 {
    metadata::time_now()
}

pub fn random() -> f64 {
    metadata::random()
}

// ============================================================================
// WebSocket
// ============================================================================

pub fn ws_connect(url: &str, headers_json: &str) -> Option<String> {
    websocket::connect(url, headers_json)
}

pub fn ws_send(session_id: &str, message: &[u8], is_binary: bool) -> bool {
    websocket::send(session_id, message, is_binary)
}

pub fn ws_send_text(session_id: &str, text: &str) -> bool {
    websocket::send(session_id, text.as_bytes(), false)
}

pub fn ws_receive(session_id: &str, timeout_ms: u32) -> Option<String> {
    websocket::receive(session_id, timeout_ms)
}

pub fn ws_close(session_id: &str) -> bool {
    websocket::close(session_id)
}

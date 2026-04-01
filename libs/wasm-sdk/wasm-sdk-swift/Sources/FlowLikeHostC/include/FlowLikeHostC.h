#pragma once
#include <stdint.h>

// ============================================================================
// Host Imports — flowlike_log
// ============================================================================

__attribute__((import_module("flowlike_log"), import_name("trace")))
extern void flowlike_log_trace(uint32_t ptr, uint32_t len);

__attribute__((import_module("flowlike_log"), import_name("debug")))
extern void flowlike_log_debug(uint32_t ptr, uint32_t len);

__attribute__((import_module("flowlike_log"), import_name("info")))
extern void flowlike_log_info(uint32_t ptr, uint32_t len);

__attribute__((import_module("flowlike_log"), import_name("warn")))
extern void flowlike_log_warn(uint32_t ptr, uint32_t len);

__attribute__((import_module("flowlike_log"), import_name("error")))
extern void flowlike_log_error(uint32_t ptr, uint32_t len);

__attribute__((import_module("flowlike_log"), import_name("log_json")))
extern void flowlike_log_json(int32_t level, uint32_t msg_ptr, uint32_t msg_len, uint32_t data_ptr, uint32_t data_len);

// ============================================================================
// Host Imports — flowlike_pins
// ============================================================================

__attribute__((import_module("flowlike_pins"), import_name("get_input")))
extern int64_t flowlike_pins_get_input(uint32_t name_ptr, uint32_t name_len);

__attribute__((import_module("flowlike_pins"), import_name("set_output")))
extern void flowlike_pins_set_output(uint32_t name_ptr, uint32_t name_len, uint32_t val_ptr, uint32_t val_len);

__attribute__((import_module("flowlike_pins"), import_name("activate_exec")))
extern void flowlike_pins_activate_exec(uint32_t name_ptr, uint32_t name_len);

// ============================================================================
// Host Imports — flowlike_vars
// ============================================================================

__attribute__((import_module("flowlike_vars"), import_name("get")))
extern int64_t flowlike_vars_get(uint32_t name_ptr, uint32_t name_len);

__attribute__((import_module("flowlike_vars"), import_name("set")))
extern void flowlike_vars_set(uint32_t name_ptr, uint32_t name_len, uint32_t val_ptr, uint32_t val_len);

__attribute__((import_module("flowlike_vars"), import_name("delete")))
extern void flowlike_vars_delete(uint32_t name_ptr, uint32_t name_len);

__attribute__((import_module("flowlike_vars"), import_name("has")))
extern int32_t flowlike_vars_has(uint32_t name_ptr, uint32_t name_len);

// ============================================================================
// Host Imports — flowlike_cache
// ============================================================================

__attribute__((import_module("flowlike_cache"), import_name("get")))
extern int64_t flowlike_cache_get(uint32_t key_ptr, uint32_t key_len);

__attribute__((import_module("flowlike_cache"), import_name("set")))
extern void flowlike_cache_set(uint32_t key_ptr, uint32_t key_len, uint32_t val_ptr, uint32_t val_len);

__attribute__((import_module("flowlike_cache"), import_name("delete")))
extern void flowlike_cache_delete(uint32_t key_ptr, uint32_t key_len);

__attribute__((import_module("flowlike_cache"), import_name("has")))
extern int32_t flowlike_cache_has(uint32_t key_ptr, uint32_t key_len);

// ============================================================================
// Host Imports — flowlike_meta
// ============================================================================

__attribute__((import_module("flowlike_meta"), import_name("get_node_id")))
extern int64_t flowlike_meta_get_node_id(void);

__attribute__((import_module("flowlike_meta"), import_name("get_run_id")))
extern int64_t flowlike_meta_get_run_id(void);

__attribute__((import_module("flowlike_meta"), import_name("get_app_id")))
extern int64_t flowlike_meta_get_app_id(void);

__attribute__((import_module("flowlike_meta"), import_name("get_board_id")))
extern int64_t flowlike_meta_get_board_id(void);

__attribute__((import_module("flowlike_meta"), import_name("get_user_id")))
extern int64_t flowlike_meta_get_user_id(void);

__attribute__((import_module("flowlike_meta"), import_name("is_streaming")))
extern int32_t flowlike_meta_is_streaming(void);

__attribute__((import_module("flowlike_meta"), import_name("get_log_level")))
extern int32_t flowlike_meta_get_log_level(void);

__attribute__((import_module("flowlike_meta"), import_name("time_now")))
extern int64_t flowlike_meta_time_now(void);

__attribute__((import_module("flowlike_meta"), import_name("random")))
extern int64_t flowlike_meta_random(void);

// ============================================================================
// Host Imports — flowlike_storage
// ============================================================================

__attribute__((import_module("flowlike_storage"), import_name("read_request")))
extern int64_t flowlike_storage_read_request(uint32_t path_ptr, uint32_t path_len);

__attribute__((import_module("flowlike_storage"), import_name("write_request")))
extern int32_t flowlike_storage_write_request(uint32_t path_ptr, uint32_t path_len, uint32_t data_ptr, uint32_t data_len);

__attribute__((import_module("flowlike_storage"), import_name("storage_dir")))
extern int64_t flowlike_storage_storage_dir(int32_t node_scoped);

__attribute__((import_module("flowlike_storage"), import_name("upload_dir")))
extern int64_t flowlike_storage_upload_dir(void);

__attribute__((import_module("flowlike_storage"), import_name("cache_dir")))
extern int64_t flowlike_storage_cache_dir(int32_t node_scoped, int32_t user_scoped);

__attribute__((import_module("flowlike_storage"), import_name("user_dir")))
extern int64_t flowlike_storage_user_dir(int32_t node_scoped);

__attribute__((import_module("flowlike_storage"), import_name("list_request")))
extern int64_t flowlike_storage_list_request(uint32_t path_ptr, uint32_t path_len);

// ============================================================================
// Host Imports — flowlike_models
// ============================================================================

__attribute__((import_module("flowlike_models"), import_name("embed_text")))
extern int64_t flowlike_models_embed_text(uint32_t bit_ptr, uint32_t bit_len, uint32_t texts_ptr, uint32_t texts_len);

// ============================================================================
// Host Imports — flowlike_http
// ============================================================================

__attribute__((import_module("flowlike_http"), import_name("request")))
extern int32_t flowlike_http_request(int32_t method, uint32_t url_ptr, uint32_t url_len, uint32_t headers_ptr, uint32_t headers_len, uint32_t body_ptr, uint32_t body_len);

// ============================================================================
// Host Imports — flowlike_stream
// ============================================================================

__attribute__((import_module("flowlike_stream"), import_name("emit")))
extern void flowlike_stream_emit(uint32_t event_ptr, uint32_t event_len, uint32_t data_ptr, uint32_t data_len);

__attribute__((import_module("flowlike_stream"), import_name("text")))
extern void flowlike_stream_text(uint32_t text_ptr, uint32_t text_len);

// ============================================================================
// Host Imports — flowlike_auth
// ============================================================================

__attribute__((import_module("flowlike_auth"), import_name("get_oauth_token")))
extern int64_t flowlike_auth_get_oauth_token(uint32_t provider_ptr, uint32_t provider_len);

__attribute__((import_module("flowlike_auth"), import_name("has_oauth_token")))
extern int32_t flowlike_auth_has_oauth_token(uint32_t provider_ptr, uint32_t provider_len);

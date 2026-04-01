/*
 * Flow-Like WASM SDK — C Glue for Lua
 *
 * This file embeds a Lua interpreter, declares all WASM host imports, exposes
 * them to Lua via a "flowlike_host" global table, and implements the WASM
 * exports (get_node, get_nodes, run, alloc, dealloc, get_abi_version).
 *
 * Compilation pipeline: glue.c + Lua sources → Emscripten → WASM
 */

#include <lua.h>
#include <lauxlib.h>
#include <lualib.h>

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* ============================================================================
 * Stubs for Lua standard libraries that use OS syscalls unavailable in WASM.
 * liolib.c and loslib.c are excluded from the build; these stubs satisfy
 * linit.c's calls to luaopen_io / luaopen_os without pulling in syscalls.
 * Lua scripts should use the flowlike_storage / flowlike_http host APIs instead.
 * ========================================================================= */
LUAMOD_API int luaopen_io(lua_State *L) {
    lua_pushnil(L);
    return 1;
}

LUAMOD_API int luaopen_os(lua_State *L) {
    lua_pushnil(L);
    return 1;
}


/* =========================================================================
 * ABI
 * ========================================================================= */

#define ABI_VERSION 1

/* =========================================================================
 * Host imports — flowlike_log
 * ========================================================================= */

__attribute__((import_module("flowlike_log"), import_name("trace")))
extern void _fl_log_trace(const char *ptr, uint32_t len);

__attribute__((import_module("flowlike_log"), import_name("debug")))
extern void _fl_log_debug(const char *ptr, uint32_t len);

__attribute__((import_module("flowlike_log"), import_name("info")))
extern void _fl_log_info(const char *ptr, uint32_t len);

__attribute__((import_module("flowlike_log"), import_name("warn")))
extern void _fl_log_warn(const char *ptr, uint32_t len);

__attribute__((import_module("flowlike_log"), import_name("error")))
extern void _fl_log_error(const char *ptr, uint32_t len);

__attribute__((import_module("flowlike_log"), import_name("log_json")))
extern void _fl_log_json(int32_t level, const char *msg_ptr, uint32_t msg_len,
                          const char *data_ptr, uint32_t data_len);

/* =========================================================================
 * Host imports — flowlike_pins
 * ========================================================================= */

__attribute__((import_module("flowlike_pins"), import_name("get_input")))
extern int64_t _fl_get_input(const char *name_ptr, uint32_t name_len);

__attribute__((import_module("flowlike_pins"), import_name("set_output")))
extern void _fl_set_output(const char *name_ptr, uint32_t name_len,
                            const char *val_ptr, uint32_t val_len);

__attribute__((import_module("flowlike_pins"), import_name("activate_exec")))
extern void _fl_activate_exec(const char *name_ptr, uint32_t name_len);

/* =========================================================================
 * Host imports — flowlike_vars
 * ========================================================================= */

__attribute__((import_module("flowlike_vars"), import_name("get")))
extern int64_t _fl_var_get(const char *name_ptr, uint32_t name_len);

__attribute__((import_module("flowlike_vars"), import_name("set")))
extern void _fl_var_set(const char *name_ptr, uint32_t name_len,
                         const char *val_ptr, uint32_t val_len);

__attribute__((import_module("flowlike_vars"), import_name("delete")))
extern void _fl_var_delete(const char *name_ptr, uint32_t name_len);

__attribute__((import_module("flowlike_vars"), import_name("has")))
extern int32_t _fl_var_has(const char *name_ptr, uint32_t name_len);

/* =========================================================================
 * Host imports — flowlike_cache
 * ========================================================================= */

__attribute__((import_module("flowlike_cache"), import_name("get")))
extern int64_t _fl_cache_get(const char *key_ptr, uint32_t key_len);

__attribute__((import_module("flowlike_cache"), import_name("set")))
extern void _fl_cache_set(const char *key_ptr, uint32_t key_len,
                            const char *val_ptr, uint32_t val_len);

__attribute__((import_module("flowlike_cache"), import_name("delete")))
extern void _fl_cache_delete(const char *key_ptr, uint32_t key_len);

__attribute__((import_module("flowlike_cache"), import_name("has")))
extern int32_t _fl_cache_has(const char *key_ptr, uint32_t key_len);

/* =========================================================================
 * Host imports — flowlike_meta
 * ========================================================================= */

__attribute__((import_module("flowlike_meta"), import_name("get_node_id")))
extern int64_t _fl_get_node_id(void);

__attribute__((import_module("flowlike_meta"), import_name("get_run_id")))
extern int64_t _fl_get_run_id(void);

__attribute__((import_module("flowlike_meta"), import_name("get_app_id")))
extern int64_t _fl_get_app_id(void);

__attribute__((import_module("flowlike_meta"), import_name("get_board_id")))
extern int64_t _fl_get_board_id(void);

__attribute__((import_module("flowlike_meta"), import_name("get_user_id")))
extern int64_t _fl_get_user_id(void);

__attribute__((import_module("flowlike_meta"), import_name("is_streaming")))
extern int32_t _fl_is_streaming(void);

__attribute__((import_module("flowlike_meta"), import_name("get_log_level")))
extern int32_t _fl_get_log_level(void);

__attribute__((import_module("flowlike_meta"), import_name("time_now")))
extern int64_t _fl_time_now(void);

__attribute__((import_module("flowlike_meta"), import_name("random")))
extern int64_t _fl_random(void);

/* =========================================================================
 * Host imports — flowlike_storage
 * ========================================================================= */

__attribute__((import_module("flowlike_storage"), import_name("read_request")))
extern int64_t _fl_storage_read(const char *path_ptr, uint32_t path_len);

__attribute__((import_module("flowlike_storage"), import_name("write_request")))
extern int32_t _fl_storage_write(const char *path_ptr, uint32_t path_len,
                                  const char *data_ptr, uint32_t data_len);

__attribute__((import_module("flowlike_storage"), import_name("storage_dir")))
extern int64_t _fl_storage_dir(int32_t node_scoped);

__attribute__((import_module("flowlike_storage"), import_name("upload_dir")))
extern int64_t _fl_upload_dir(void);

__attribute__((import_module("flowlike_storage"), import_name("cache_dir")))
extern int64_t _fl_cache_dir(int32_t node_scoped, int32_t user_scoped);

__attribute__((import_module("flowlike_storage"), import_name("user_dir")))
extern int64_t _fl_user_dir(int32_t node_scoped);

__attribute__((import_module("flowlike_storage"), import_name("list_request")))
extern int64_t _fl_storage_list(const char *path_ptr, uint32_t path_len);

/* =========================================================================
 * Host imports — flowlike_models
 * ========================================================================= */

__attribute__((import_module("flowlike_models"), import_name("embed_text")))
extern int64_t _fl_embed_text(const char *bit_ptr, uint32_t bit_len,
                               const char *texts_ptr, uint32_t texts_len);

/* =========================================================================
 * Host imports — flowlike_http
 * ========================================================================= */

__attribute__((import_module("flowlike_http"), import_name("request")))
extern int32_t _fl_http_request(int32_t method,
                                 const char *url_ptr, uint32_t url_len,
                                 const char *hdr_ptr, uint32_t hdr_len,
                                 const char *body_ptr, uint32_t body_len);

/* =========================================================================
 * Host imports — flowlike_stream
 * ========================================================================= */

__attribute__((import_module("flowlike_stream"), import_name("emit")))
extern void _fl_stream_emit(const char *evt_ptr, uint32_t evt_len,
                              const char *data_ptr, uint32_t data_len);

__attribute__((import_module("flowlike_stream"), import_name("text")))
extern void _fl_stream_text(const char *text_ptr, uint32_t text_len);

/* =========================================================================
 * Host imports — flowlike_auth
 * ========================================================================= */

__attribute__((import_module("flowlike_auth"), import_name("get_oauth_token")))
extern int64_t _fl_get_oauth_token(const char *prov_ptr, uint32_t prov_len);

__attribute__((import_module("flowlike_auth"), import_name("has_oauth_token")))
extern int32_t _fl_has_oauth_token(const char *prov_ptr, uint32_t prov_len);

/* =========================================================================
 * Packed i64 helpers (ptr << 32 | len)
 * ========================================================================= */

static int64_t pack_i64(uint32_t ptr, uint32_t len) {
    return ((int64_t)ptr << 32) | (int64_t)len;
}

static void unpack_i64(int64_t packed, uint32_t *out_ptr, uint32_t *out_len) {
    *out_ptr = (uint32_t)(packed >> 32);
    *out_len = (uint32_t)(packed & 0xFFFFFFFF);
}

static const char *unpack_string(int64_t packed, uint32_t *out_len) {
    uint32_t ptr;
    unpack_i64(packed, &ptr, out_len);
    if (ptr == 0 || *out_len == 0) { *out_len = 0; return ""; }
    return (const char *)(uintptr_t)ptr;
}

/* =========================================================================
 * Global result buffer (keeps serialized data alive for host to read)
 * ========================================================================= */

static char  *g_result_buf  = NULL;
static size_t g_result_cap  = 0;
static size_t g_result_len  = 0;

static int64_t pack_result(const char *json, size_t len) {
    if (len + 1 > g_result_cap) {
        g_result_cap = len + 256;
        g_result_buf = (char *)realloc(g_result_buf, g_result_cap);
    }
    memcpy(g_result_buf, json, len);
    g_result_buf[len] = '\0';
    g_result_len = len;
    return pack_i64((uint32_t)(uintptr_t)g_result_buf, (uint32_t)len);
}

/* =========================================================================
 * Lua state
 * ========================================================================= */

static lua_State *L = NULL;

/* =========================================================================
 * Lua C functions — Logging
 * ========================================================================= */

static int l_log_trace(lua_State *L) {
    size_t len; const char *msg = luaL_checklstring(L, 1, &len);
    _fl_log_trace(msg, (uint32_t)len);
    return 0;
}

static int l_log_debug(lua_State *L) {
    size_t len; const char *msg = luaL_checklstring(L, 1, &len);
    _fl_log_debug(msg, (uint32_t)len);
    return 0;
}

static int l_log_info(lua_State *L) {
    size_t len; const char *msg = luaL_checklstring(L, 1, &len);
    _fl_log_info(msg, (uint32_t)len);
    return 0;
}

static int l_log_warn(lua_State *L) {
    size_t len; const char *msg = luaL_checklstring(L, 1, &len);
    _fl_log_warn(msg, (uint32_t)len);
    return 0;
}

static int l_log_error(lua_State *L) {
    size_t len; const char *msg = luaL_checklstring(L, 1, &len);
    _fl_log_error(msg, (uint32_t)len);
    return 0;
}

static int l_log_json(lua_State *L) {
    int32_t level = (int32_t)luaL_checkinteger(L, 1);
    size_t msg_len, data_len;
    const char *msg  = luaL_checklstring(L, 2, &msg_len);
    const char *data = luaL_checklstring(L, 3, &data_len);
    _fl_log_json(level, msg, (uint32_t)msg_len, data, (uint32_t)data_len);
    return 0;
}

/* =========================================================================
 * Lua C functions — Pins
 * ========================================================================= */

static int l_get_input(lua_State *L) {
    size_t len; const char *name = luaL_checklstring(L, 1, &len);
    int64_t packed = _fl_get_input(name, (uint32_t)len);
    uint32_t slen;
    const char *s = unpack_string(packed, &slen);
    lua_pushlstring(L, s, slen);
    return 1;
}

static int l_set_output(lua_State *L) {
    size_t name_len, val_len;
    const char *name = luaL_checklstring(L, 1, &name_len);
    const char *val  = luaL_checklstring(L, 2, &val_len);
    _fl_set_output(name, (uint32_t)name_len, val, (uint32_t)val_len);
    return 0;
}

static int l_activate_exec(lua_State *L) {
    size_t len; const char *name = luaL_checklstring(L, 1, &len);
    _fl_activate_exec(name, (uint32_t)len);
    return 0;
}

/* =========================================================================
 * Lua C functions — Variables
 * ========================================================================= */

static int l_var_get(lua_State *L) {
    size_t len; const char *name = luaL_checklstring(L, 1, &len);
    int64_t packed = _fl_var_get(name, (uint32_t)len);
    uint32_t slen;
    const char *s = unpack_string(packed, &slen);
    lua_pushlstring(L, s, slen);
    return 1;
}

static int l_var_set(lua_State *L) {
    size_t name_len, val_len;
    const char *name = luaL_checklstring(L, 1, &name_len);
    const char *val  = luaL_checklstring(L, 2, &val_len);
    _fl_var_set(name, (uint32_t)name_len, val, (uint32_t)val_len);
    return 0;
}

static int l_var_delete(lua_State *L) {
    size_t len; const char *name = luaL_checklstring(L, 1, &len);
    _fl_var_delete(name, (uint32_t)len);
    return 0;
}

static int l_var_has(lua_State *L) {
    size_t len; const char *name = luaL_checklstring(L, 1, &len);
    int32_t has = _fl_var_has(name, (uint32_t)len);
    lua_pushboolean(L, has != 0);
    return 1;
}

/* =========================================================================
 * Lua C functions — Cache
 * ========================================================================= */

static int l_cache_get(lua_State *L) {
    size_t len; const char *key = luaL_checklstring(L, 1, &len);
    int64_t packed = _fl_cache_get(key, (uint32_t)len);
    uint32_t slen;
    const char *s = unpack_string(packed, &slen);
    lua_pushlstring(L, s, slen);
    return 1;
}

static int l_cache_set(lua_State *L) {
    size_t key_len, val_len;
    const char *key = luaL_checklstring(L, 1, &key_len);
    const char *val = luaL_checklstring(L, 2, &val_len);
    _fl_cache_set(key, (uint32_t)key_len, val, (uint32_t)val_len);
    return 0;
}

static int l_cache_delete(lua_State *L) {
    size_t len; const char *key = luaL_checklstring(L, 1, &len);
    _fl_cache_delete(key, (uint32_t)len);
    return 0;
}

static int l_cache_has(lua_State *L) {
    size_t len; const char *key = luaL_checklstring(L, 1, &len);
    int32_t has = _fl_cache_has(key, (uint32_t)len);
    lua_pushboolean(L, has != 0);
    return 1;
}

/* =========================================================================
 * Lua C functions — Meta
 * ========================================================================= */

static int l_get_node_id(lua_State *L) {
    uint32_t slen; const char *s = unpack_string(_fl_get_node_id(), &slen);
    lua_pushlstring(L, s, slen); return 1;
}

static int l_get_run_id(lua_State *L) {
    uint32_t slen; const char *s = unpack_string(_fl_get_run_id(), &slen);
    lua_pushlstring(L, s, slen); return 1;
}

static int l_get_app_id(lua_State *L) {
    uint32_t slen; const char *s = unpack_string(_fl_get_app_id(), &slen);
    lua_pushlstring(L, s, slen); return 1;
}

static int l_get_board_id(lua_State *L) {
    uint32_t slen; const char *s = unpack_string(_fl_get_board_id(), &slen);
    lua_pushlstring(L, s, slen); return 1;
}

static int l_get_user_id(lua_State *L) {
    uint32_t slen; const char *s = unpack_string(_fl_get_user_id(), &slen);
    lua_pushlstring(L, s, slen); return 1;
}

static int l_is_streaming(lua_State *L) {
    lua_pushboolean(L, _fl_is_streaming() != 0);
    return 1;
}

static int l_get_log_level(lua_State *L) {
    lua_pushinteger(L, _fl_get_log_level());
    return 1;
}

static int l_time_now(lua_State *L) {
    lua_pushinteger(L, (lua_Integer)_fl_time_now());
    return 1;
}

static int l_random(lua_State *L) {
    lua_pushinteger(L, (lua_Integer)_fl_random());
    return 1;
}

/* =========================================================================
 * Lua C functions — Storage
 * ========================================================================= */

static int l_storage_read(lua_State *L) {
    size_t len; const char *path = luaL_checklstring(L, 1, &len);
    int64_t packed = _fl_storage_read(path, (uint32_t)len);
    uint32_t slen;
    const char *s = unpack_string(packed, &slen);
    lua_pushlstring(L, s, slen);
    return 1;
}

static int l_storage_write(lua_State *L) {
    size_t path_len, data_len;
    const char *path = luaL_checklstring(L, 1, &path_len);
    const char *data = luaL_checklstring(L, 2, &data_len);
    int32_t ret = _fl_storage_write(path, (uint32_t)path_len, data, (uint32_t)data_len);
    lua_pushinteger(L, ret);
    return 1;
}

static int l_storage_dir(lua_State *L) {
    int32_t node_scoped = (int32_t)luaL_checkinteger(L, 1);
    uint32_t slen;
    const char *s = unpack_string(_fl_storage_dir(node_scoped), &slen);
    lua_pushlstring(L, s, slen);
    return 1;
}

static int l_upload_dir(lua_State *L) {
    uint32_t slen;
    const char *s = unpack_string(_fl_upload_dir(), &slen);
    lua_pushlstring(L, s, slen);
    return 1;
}

static int l_cache_dir(lua_State *L) {
    int32_t ns = (int32_t)luaL_checkinteger(L, 1);
    int32_t us = (int32_t)luaL_checkinteger(L, 2);
    uint32_t slen;
    const char *s = unpack_string(_fl_cache_dir(ns, us), &slen);
    lua_pushlstring(L, s, slen);
    return 1;
}

static int l_user_dir(lua_State *L) {
    int32_t node_scoped = (int32_t)luaL_checkinteger(L, 1);
    uint32_t slen;
    const char *s = unpack_string(_fl_user_dir(node_scoped), &slen);
    lua_pushlstring(L, s, slen);
    return 1;
}

static int l_storage_list(lua_State *L) {
    size_t len; const char *path = luaL_checklstring(L, 1, &len);
    int64_t packed = _fl_storage_list(path, (uint32_t)len);
    uint32_t slen;
    const char *s = unpack_string(packed, &slen);
    lua_pushlstring(L, s, slen);
    return 1;
}

/* =========================================================================
 * Lua C functions — Models
 * ========================================================================= */

static int l_embed_text(lua_State *L) {
    size_t bit_len, texts_len;
    const char *bit   = luaL_checklstring(L, 1, &bit_len);
    const char *texts = luaL_checklstring(L, 2, &texts_len);
    int64_t packed = _fl_embed_text(bit, (uint32_t)bit_len, texts, (uint32_t)texts_len);
    uint32_t slen;
    const char *s = unpack_string(packed, &slen);
    lua_pushlstring(L, s, slen);
    return 1;
}

/* =========================================================================
 * Lua C functions — HTTP
 * ========================================================================= */

static int l_http_request(lua_State *L) {
    int32_t method = (int32_t)luaL_checkinteger(L, 1);
    size_t url_len, hdr_len, body_len;
    const char *url  = luaL_checklstring(L, 2, &url_len);
    const char *hdr  = luaL_checklstring(L, 3, &hdr_len);
    const char *body = luaL_checklstring(L, 4, &body_len);
    int32_t ret = _fl_http_request(method,
        url, (uint32_t)url_len, hdr, (uint32_t)hdr_len, body, (uint32_t)body_len);
    lua_pushinteger(L, ret);
    return 1;
}

/* =========================================================================
 * Lua C functions — Stream
 * ========================================================================= */

static int l_stream_emit(lua_State *L) {
    size_t evt_len, data_len;
    const char *evt  = luaL_checklstring(L, 1, &evt_len);
    const char *data = luaL_checklstring(L, 2, &data_len);
    _fl_stream_emit(evt, (uint32_t)evt_len, data, (uint32_t)data_len);
    return 0;
}

static int l_stream_text(lua_State *L) {
    size_t len; const char *text = luaL_checklstring(L, 1, &len);
    _fl_stream_text(text, (uint32_t)len);
    return 0;
}

/* =========================================================================
 * Lua C functions — Auth
 * ========================================================================= */

static int l_oauth_get_token(lua_State *L) {
    size_t len; const char *prov = luaL_checklstring(L, 1, &len);
    int64_t packed = _fl_get_oauth_token(prov, (uint32_t)len);
    uint32_t slen;
    const char *s = unpack_string(packed, &slen);
    lua_pushlstring(L, s, slen);
    return 1;
}

static int l_oauth_has_token(lua_State *L) {
    size_t len; const char *prov = luaL_checklstring(L, 1, &len);
    int32_t has = _fl_has_oauth_token(prov, (uint32_t)len);
    lua_pushboolean(L, has != 0);
    return 1;
}

/* =========================================================================
 * Register all host functions into "flowlike_host" global table
 * ========================================================================= */

static const luaL_Reg host_funcs[] = {
    /* logging */
    { "log_trace",      l_log_trace },
    { "log_debug",      l_log_debug },
    { "log_info",       l_log_info },
    { "log_warn",       l_log_warn },
    { "log_error",      l_log_error },
    { "log_json",       l_log_json },
    /* pins */
    { "get_input",      l_get_input },
    { "set_output",     l_set_output },
    { "activate_exec",  l_activate_exec },
    /* vars */
    { "var_get",        l_var_get },
    { "var_set",        l_var_set },
    { "var_delete",     l_var_delete },
    { "var_has",        l_var_has },
    /* cache */
    { "cache_get",      l_cache_get },
    { "cache_set",      l_cache_set },
    { "cache_delete",   l_cache_delete },
    { "cache_has",      l_cache_has },
    /* meta */
    { "get_node_id",    l_get_node_id },
    { "get_run_id",     l_get_run_id },
    { "get_app_id",     l_get_app_id },
    { "get_board_id",   l_get_board_id },
    { "get_user_id",    l_get_user_id },
    { "is_streaming",   l_is_streaming },
    { "get_log_level",  l_get_log_level },
    { "time_now",       l_time_now },
    { "random",         l_random },
    /* storage */
    { "storage_read",   l_storage_read },
    { "storage_write",  l_storage_write },
    { "storage_dir",    l_storage_dir },
    { "upload_dir",     l_upload_dir },
    { "cache_dir",      l_cache_dir },
    { "user_dir",       l_user_dir },
    { "storage_list",   l_storage_list },
    /* models */
    { "embed_text",     l_embed_text },
    /* http */
    { "http_request",   l_http_request },
    /* stream */
    { "stream_emit",    l_stream_emit },
    { "stream_text",    l_stream_text },
    /* auth */
    { "oauth_get_token", l_oauth_get_token },
    { "oauth_has_token", l_oauth_has_token },
    { NULL, NULL }
};

static void register_host_functions(lua_State *L) {
    lua_newtable(L);
    const luaL_Reg *reg = host_funcs;
    while (reg->name != NULL) {
        lua_pushcfunction(L, reg->func);
        lua_setfield(L, -2, reg->name);
        reg++;
    }
    lua_setglobal(L, "flowlike_host");
}

/* =========================================================================
 * Embedded Lua sources
 *
 * The SDK and node Lua files are embedded as string literals compiled into
 * the WASM binary. This avoids filesystem dependency at runtime.
 *
 * If you prefer loading from the Emscripten virtual filesystem instead,
 * replace the dostring calls with dofile.
 * ========================================================================= */

/* Forward declarations for embedded sources (provided at link time) */
extern const char _lua_sdk_source[];
extern const char _lua_node_source[];

/* Default source fallbacks (overridden when linking with actual sources) */
__attribute__((weak))
const char _lua_sdk_source[] = "";

__attribute__((weak))
const char _lua_node_source[] = "";

/* =========================================================================
 * Lua state initialization
 * ========================================================================= */

static void ensure_init(void) {
    if (L) return;

    L = luaL_newstate();
    luaL_openlibs(L);

    register_host_functions(L);

    /* Load SDK: execute the sdk source chunk, capture its return value (the
     * module table), then register it as both package.loaded["sdk"] and the
     * global "sdk" so node.lua can use  require("sdk")  or  local sdk = require("sdk"). */
    const char *sdk_src = (_lua_sdk_source[0] != '\0') ? _lua_sdk_source : NULL;
    if (sdk_src) {
        if (luaL_loadstring(L, sdk_src) == 0) {
            /* pcall the chunk; it should return the module table. */
            if (lua_pcall(L, 0, 1, 0) == 0) {
                /* Stack: [module_or_nil] */
                if (!lua_isnil(L, -1)) {
                    /* Register in package.loaded["sdk"] */
                    lua_getglobal(L, "package");
                    lua_getfield(L, -1, "loaded");
                    lua_pushvalue(L, -3);          /* dup module */
                    lua_setfield(L, -2, "sdk");    /* package.loaded["sdk"] = module */
                    lua_pop(L, 2);                 /* pop loaded, package */

                    /* Register as global "sdk" */
                    lua_setglobal(L, "sdk");
                } else {
                    lua_pop(L, 1);
                }
            } else {
                const char *err = lua_tostring(L, -1);
                if (err) _fl_log_error(err, (uint32_t)strlen(err));
                lua_pop(L, 1);
            }
        } else {
            const char *err = lua_tostring(L, -1);
            if (err) _fl_log_error(err, (uint32_t)strlen(err));
            lua_pop(L, 1);
        }
    }

    /* Load user node */
    if (_lua_node_source[0] != '\0') {
        if (luaL_dostring(L, _lua_node_source) != 0) {
            const char *err = lua_tostring(L, -1);
            if (err) _fl_log_error(err, (uint32_t)strlen(err));
            lua_pop(L, 1);
        }
    } else {
        luaL_dofile(L, "node.lua");
    }
}

/* =========================================================================
 * Helper: call a Lua global function that returns a string
 * ========================================================================= */

static int64_t call_lua_string_func(const char *func_name) {
    ensure_init();
    lua_getglobal(L, func_name);
    if (!lua_isfunction(L, -1)) {
        lua_pop(L, 1);
        return 0;
    }
    if (lua_pcall(L, 0, 1, 0) != 0) {
        const char *err = lua_tostring(L, -1);
        _fl_log_error(err, (uint32_t)strlen(err));
        lua_pop(L, 1);
        return 0;
    }
    size_t len;
    const char *str = lua_tolstring(L, -1, &len);
    int64_t result = pack_result(str, len);
    lua_pop(L, 1);
    return result;
}

/* =========================================================================
 * WASM exports
 * ========================================================================= */

__attribute__((export_name("get_node")))
int64_t get_node(void) {
    return call_lua_string_func("get_node");
}

__attribute__((export_name("get_nodes")))
int64_t get_nodes(void) {
    return call_lua_string_func("get_nodes");
}

__attribute__((export_name("run")))
int64_t run(uint32_t ptr, uint32_t len) {
    ensure_init();

    lua_getglobal(L, "run_node");
    if (!lua_isfunction(L, -1)) {
        lua_pop(L, 1);
        const char *err = "{\"outputs\":{},\"activate_exec\":[],\"pending\":false,\"error\":\"run_node function not defined\"}";
        return pack_result(err, strlen(err));
    }

    /* Pass the raw JSON string to Lua */
    const char *input_str = (const char *)(uintptr_t)ptr;
    lua_pushlstring(L, input_str, len);

    if (lua_pcall(L, 1, 1, 0) != 0) {
        const char *lua_err = lua_tostring(L, -1);
        _fl_log_error(lua_err, (uint32_t)strlen(lua_err));

        /* Build error JSON */
        const char *prefix = "{\"outputs\":{},\"activate_exec\":[],\"pending\":false,\"error\":\"";
        const char *suffix = "\"}";
        size_t total = strlen(prefix) + strlen(lua_err) + strlen(suffix);
        char *buf = (char *)malloc(total + 1);
        snprintf(buf, total + 1, "%s%s%s", prefix, lua_err, suffix);
        int64_t result = pack_result(buf, total);
        free(buf);
        lua_pop(L, 1);
        return result;
    }

    size_t result_len;
    const char *result_str = lua_tolstring(L, -1, &result_len);
    int64_t result = pack_result(result_str, result_len);
    lua_pop(L, 1);
    return result;
}

__attribute__((export_name("get_abi_version")))
uint32_t get_abi_version(void) {
    return ABI_VERSION;
}

__attribute__((export_name("alloc")))
uint32_t alloc(uint32_t size) {
    return (uint32_t)(uintptr_t)malloc(size);
}

__attribute__((export_name("dealloc")))
void dealloc(uint32_t ptr, uint32_t size) {
    (void)size;
    free((void *)(uintptr_t)ptr);
}

// Host function imports for the Flow-Like WASM runtime.
//
// Each struct groups imports by their WASM module name to avoid Zig-level
// name collisions (e.g. flowlike_vars.get vs flowlike_cache.get).
//
// Public wrapper functions below accept Zig slices and handle pointer
// conversion so callers never deal with raw u32 addresses.

const std = @import("std");
const mem = @import("memory.zig");

// ---------------------------------------------------------------------------
// Raw host imports — flowlike_log
// ---------------------------------------------------------------------------

const log_fns = struct {
    extern "flowlike_log" fn trace(ptr: u32, len: u32) void;
    extern "flowlike_log" fn debug(ptr: u32, len: u32) void;
    extern "flowlike_log" fn info(ptr: u32, len: u32) void;
    extern "flowlike_log" fn warn(ptr: u32, len: u32) void;
    extern "flowlike_log" fn @"error"(ptr: u32, len: u32) void;
    extern "flowlike_log" fn log_json(level: i32, msg_ptr: u32, msg_len: u32, data_ptr: u32, data_len: u32) void;
};

// ---------------------------------------------------------------------------
// Raw host imports — flowlike_pins
// ---------------------------------------------------------------------------

const pin_fns = struct {
    extern "flowlike_pins" fn get_input(name_ptr: u32, name_len: u32) i64;
    extern "flowlike_pins" fn set_output(name_ptr: u32, name_len: u32, val_ptr: u32, val_len: u32) void;
    extern "flowlike_pins" fn activate_exec(name_ptr: u32, name_len: u32) void;
};

// ---------------------------------------------------------------------------
// Raw host imports — flowlike_vars
// ---------------------------------------------------------------------------

const var_fns = struct {
    extern "flowlike_vars" fn get(name_ptr: u32, name_len: u32) i64;
    extern "flowlike_vars" fn set(name_ptr: u32, name_len: u32, val_ptr: u32, val_len: u32) void;
    extern "flowlike_vars" fn delete(name_ptr: u32, name_len: u32) void;
    extern "flowlike_vars" fn has(name_ptr: u32, name_len: u32) i32;
};

// ---------------------------------------------------------------------------
// Raw host imports — flowlike_cache
// ---------------------------------------------------------------------------

const cache_fns = struct {
    extern "flowlike_cache" fn get(key_ptr: u32, key_len: u32) i64;
    extern "flowlike_cache" fn set(key_ptr: u32, key_len: u32, val_ptr: u32, val_len: u32) void;
    extern "flowlike_cache" fn delete(key_ptr: u32, key_len: u32) void;
    extern "flowlike_cache" fn has(key_ptr: u32, key_len: u32) i32;
};

// ---------------------------------------------------------------------------
// Raw host imports — flowlike_meta
// ---------------------------------------------------------------------------

const meta_fns = struct {
    extern "flowlike_meta" fn get_node_id() i64;
    extern "flowlike_meta" fn get_run_id() i64;
    extern "flowlike_meta" fn get_app_id() i64;
    extern "flowlike_meta" fn get_board_id() i64;
    extern "flowlike_meta" fn get_user_id() i64;
    extern "flowlike_meta" fn is_streaming() i32;
    extern "flowlike_meta" fn get_log_level() i32;
    extern "flowlike_meta" fn time_now() i64;
    extern "flowlike_meta" fn random() i64;
};

// ---------------------------------------------------------------------------
// Raw host imports — flowlike_storage
// ---------------------------------------------------------------------------

const storage_fns = struct {
    extern "flowlike_storage" fn read_request(path_ptr: u32, path_len: u32) i64;
    extern "flowlike_storage" fn write_request(path_ptr: u32, path_len: u32, data_ptr: u32, data_len: u32) i32;
    extern "flowlike_storage" fn storage_dir(node_scoped: i32) i64;
    extern "flowlike_storage" fn upload_dir() i64;
    extern "flowlike_storage" fn cache_dir(node_scoped: i32, user_scoped: i32) i64;
    extern "flowlike_storage" fn user_dir(node_scoped: i32) i64;
    extern "flowlike_storage" fn list_request(path_ptr: u32, path_len: u32) i64;
};

// ---------------------------------------------------------------------------
// Raw host imports — flowlike_models
// ---------------------------------------------------------------------------

const model_fns = struct {
    extern "flowlike_models" fn embed_text(bit_ptr: u32, bit_len: u32, texts_ptr: u32, texts_len: u32) i64;
};

// ---------------------------------------------------------------------------
// Raw host imports — flowlike_http
// ---------------------------------------------------------------------------

const http_fns = struct {
    extern "flowlike_http" fn request(method: i32, url_ptr: u32, url_len: u32, headers_ptr: u32, headers_len: u32, body_ptr: u32, body_len: u32) i32;
};

// ---------------------------------------------------------------------------
// Raw host imports — flowlike_stream
// ---------------------------------------------------------------------------

const stream_fns = struct {
    extern "flowlike_stream" fn emit(event_ptr: u32, event_len: u32, data_ptr: u32, data_len: u32) void;
    extern "flowlike_stream" fn text(text_ptr: u32, text_len: u32) void;
};

// ---------------------------------------------------------------------------
// Raw host imports — flowlike_auth
// ---------------------------------------------------------------------------

const auth_fns = struct {
    extern "flowlike_auth" fn get_oauth_token(provider_ptr: u32, provider_len: u32) i64;
    extern "flowlike_auth" fn has_oauth_token(provider_ptr: u32, provider_len: u32) i32;
};

// ---------------------------------------------------------------------------
// Pointer helpers
// ---------------------------------------------------------------------------

inline fn toRaw(s: []const u8) struct { ptr: u32, len: u32 } {
    if (s.len == 0) return .{ .ptr = 0, .len = 0 };
    return .{ .ptr = @intFromPtr(s.ptr), .len = @intCast(s.len) };
}

inline fn unpackString(val: i64) []const u8 {
    return mem.ptrToSlice(mem.unpackHigh(val), mem.unpackLow(val));
}

// ---------------------------------------------------------------------------
// Logging wrappers
// ---------------------------------------------------------------------------

pub fn logTrace(msg: []const u8) void {
    const r = toRaw(msg);
    log_fns.trace(r.ptr, r.len);
}

pub fn logDebug(msg: []const u8) void {
    const r = toRaw(msg);
    log_fns.debug(r.ptr, r.len);
}

pub fn logInfo(msg: []const u8) void {
    const r = toRaw(msg);
    log_fns.info(r.ptr, r.len);
}

pub fn logWarn(msg: []const u8) void {
    const r = toRaw(msg);
    log_fns.warn(r.ptr, r.len);
}

pub fn logError(msg: []const u8) void {
    const r = toRaw(msg);
    log_fns.@"error"(r.ptr, r.len);
}

pub fn logJson(level: i32, msg: []const u8, data: []const u8) void {
    const m = toRaw(msg);
    const d = toRaw(data);
    log_fns.log_json(level, m.ptr, m.len, d.ptr, d.len);
}

// ---------------------------------------------------------------------------
// Pin wrappers
// ---------------------------------------------------------------------------

pub fn getInput(name: []const u8) []const u8 {
    const r = toRaw(name);
    return unpackString(pin_fns.get_input(r.ptr, r.len));
}

pub fn setOutput(name: []const u8, value: []const u8) void {
    const n = toRaw(name);
    const v = toRaw(value);
    pin_fns.set_output(n.ptr, n.len, v.ptr, v.len);
}

pub fn activateExec(name: []const u8) void {
    const r = toRaw(name);
    pin_fns.activate_exec(r.ptr, r.len);
}

// ---------------------------------------------------------------------------
// Variable wrappers
// ---------------------------------------------------------------------------

pub fn getVariable(name: []const u8) []const u8 {
    const r = toRaw(name);
    return unpackString(var_fns.get(r.ptr, r.len));
}

pub fn setVariable(name: []const u8, value: []const u8) void {
    const n = toRaw(name);
    const v = toRaw(value);
    var_fns.set(n.ptr, n.len, v.ptr, v.len);
}

pub fn deleteVariable(name: []const u8) void {
    const r = toRaw(name);
    var_fns.delete(r.ptr, r.len);
}

pub fn hasVariable(name: []const u8) bool {
    const r = toRaw(name);
    return var_fns.has(r.ptr, r.len) != 0;
}

// ---------------------------------------------------------------------------
// Cache wrappers
// ---------------------------------------------------------------------------

pub fn cacheGet(key: []const u8) []const u8 {
    const r = toRaw(key);
    return unpackString(cache_fns.get(r.ptr, r.len));
}

pub fn cacheSet(key: []const u8, value: []const u8) void {
    const k = toRaw(key);
    const v = toRaw(value);
    cache_fns.set(k.ptr, k.len, v.ptr, v.len);
}

pub fn cacheDelete(key: []const u8) void {
    const r = toRaw(key);
    cache_fns.delete(r.ptr, r.len);
}

pub fn cacheHas(key: []const u8) bool {
    const r = toRaw(key);
    return cache_fns.has(r.ptr, r.len) != 0;
}

// ---------------------------------------------------------------------------
// Metadata wrappers
// ---------------------------------------------------------------------------

pub fn getNodeId() []const u8 {
    return unpackString(meta_fns.get_node_id());
}
pub fn getRunId() []const u8 {
    return unpackString(meta_fns.get_run_id());
}
pub fn getAppId() []const u8 {
    return unpackString(meta_fns.get_app_id());
}
pub fn getBoardId() []const u8 {
    return unpackString(meta_fns.get_board_id());
}
pub fn getUserId() []const u8 {
    return unpackString(meta_fns.get_user_id());
}
pub fn isStreaming() bool {
    return meta_fns.is_streaming() != 0;
}
pub fn getLogLevel() i32 {
    return meta_fns.get_log_level();
}
pub fn timeNow() i64 {
    return meta_fns.time_now();
}
pub fn random() i64 {
    return meta_fns.random();
}

// ---------------------------------------------------------------------------
// Storage wrappers
// ---------------------------------------------------------------------------

pub fn storageRead(path: []const u8) []const u8 {
    const r = toRaw(path);
    return unpackString(storage_fns.read_request(r.ptr, r.len));
}

pub fn storageWrite(path: []const u8, data: []const u8) bool {
    const p = toRaw(path);
    const d = toRaw(data);
    return storage_fns.write_request(p.ptr, p.len, d.ptr, d.len) != 0;
}

pub fn storageDir(node_scoped: bool) []const u8 {
    return unpackString(storage_fns.storage_dir(if (node_scoped) @as(i32, 1) else @as(i32, 0)));
}

pub fn uploadDir() []const u8 {
    return unpackString(storage_fns.upload_dir());
}

pub fn cacheDirPath(node_scoped: bool, user_scoped: bool) []const u8 {
    return unpackString(storage_fns.cache_dir(
        if (node_scoped) @as(i32, 1) else @as(i32, 0),
        if (user_scoped) @as(i32, 1) else @as(i32, 0),
    ));
}

pub fn userDir(node_scoped: bool) []const u8 {
    return unpackString(storage_fns.user_dir(if (node_scoped) @as(i32, 1) else @as(i32, 0)));
}

pub fn storageList(flow_path_json: []const u8) []const u8 {
    const r = toRaw(flow_path_json);
    return unpackString(storage_fns.list_request(r.ptr, r.len));
}

// ---------------------------------------------------------------------------
// Model wrappers
// ---------------------------------------------------------------------------

pub fn embedText(bit_json: []const u8, texts_json: []const u8) []const u8 {
    const b = toRaw(bit_json);
    const t = toRaw(texts_json);
    return unpackString(model_fns.embed_text(b.ptr, b.len, t.ptr, t.len));
}

// ---------------------------------------------------------------------------
// HTTP wrappers
// ---------------------------------------------------------------------------

pub fn httpRequest(method: i32, url: []const u8, headers: []const u8, body: []const u8) bool {
    const u = toRaw(url);
    const h = toRaw(headers);
    const b = toRaw(body);
    return http_fns.request(method, u.ptr, u.len, h.ptr, h.len, b.ptr, b.len) != 0;
}

// ---------------------------------------------------------------------------
// Stream wrappers
// ---------------------------------------------------------------------------

pub fn streamEmit(event_type: []const u8, data: []const u8) void {
    const e = toRaw(event_type);
    const d = toRaw(data);
    stream_fns.emit(e.ptr, e.len, d.ptr, d.len);
}

pub fn streamText(txt: []const u8) void {
    const r = toRaw(txt);
    stream_fns.text(r.ptr, r.len);
}

// ---------------------------------------------------------------------------
// Auth wrappers
// ---------------------------------------------------------------------------

pub fn getOAuthToken(provider: []const u8) []const u8 {
    const r = toRaw(provider);
    return unpackString(auth_fns.get_oauth_token(r.ptr, r.len));
}

pub fn hasOAuthToken(provider: []const u8) bool {
    const r = toRaw(provider);
    return auth_fns.has_oauth_token(r.ptr, r.len) != 0;
}

// Host.swift — WASM host import bindings for all Flow-Like modules.
//
// All host functions are declared in the FlowLikeHostC C target using
// __attribute__((import_module, import_name)) so that they emit clean
// C-ABI WASM imports (no hidden Swift context/error parameters).

import FlowLikeHostC

// MARK: - Logging

public func logTrace(_ msg: String) {
    let (p, l) = stringToPtr(msg)
    flowlike_log_trace(p, l)
}

public func logDebug(_ msg: String) {
    let (p, l) = stringToPtr(msg)
    flowlike_log_debug(p, l)
}

public func logInfo(_ msg: String) {
    let (p, l) = stringToPtr(msg)
    flowlike_log_info(p, l)
}

public func logWarn(_ msg: String) {
    let (p, l) = stringToPtr(msg)
    flowlike_log_warn(p, l)
}

public func logError(_ msg: String) {
    let (p, l) = stringToPtr(msg)
    flowlike_log_error(p, l)
}

public func logJSON(level: Int, msg: String, data: String) {
    let (mp, ml) = stringToPtr(msg)
    let (dp, dl) = stringToPtr(data)
    flowlike_log_json(Int32(level), mp, ml, dp, dl)
}

// MARK: - Pins

public func getInput(_ name: String) -> String {
    let (p, l) = stringToPtr(name)
    return unpackString(flowlike_pins_get_input(p, l))
}

public func setOutput(_ name: String, _ value: String) {
    let (np, nl) = stringToPtr(name)
    let (vp, vl) = stringToPtr(value)
    flowlike_pins_set_output(np, nl, vp, vl)
}

public func activateExec(_ name: String) {
    let (p, l) = stringToPtr(name)
    flowlike_pins_activate_exec(p, l)
}

// MARK: - Variables

public func getVariable(_ name: String) -> String {
    let (p, l) = stringToPtr(name)
    return unpackString(flowlike_vars_get(p, l))
}

public func setVariable(_ name: String, _ value: String) {
    let (np, nl) = stringToPtr(name)
    let (vp, vl) = stringToPtr(value)
    flowlike_vars_set(np, nl, vp, vl)
}

public func deleteVariable(_ name: String) {
    let (p, l) = stringToPtr(name)
    flowlike_vars_delete(p, l)
}

public func hasVariable(_ name: String) -> Bool {
    let (p, l) = stringToPtr(name)
    return flowlike_vars_has(p, l) != 0
}

// MARK: - Cache

public func cacheGet(_ key: String) -> String {
    let (p, l) = stringToPtr(key)
    return unpackString(flowlike_cache_get(p, l))
}

public func cacheSet(_ key: String, _ value: String) {
    let (kp, kl) = stringToPtr(key)
    let (vp, vl) = stringToPtr(value)
    flowlike_cache_set(kp, kl, vp, vl)
}

public func cacheDelete(_ key: String) {
    let (p, l) = stringToPtr(key)
    flowlike_cache_delete(p, l)
}

public func cacheHas(_ key: String) -> Bool {
    let (p, l) = stringToPtr(key)
    return flowlike_cache_has(p, l) != 0
}

// MARK: - Metadata

public func getNodeID() -> String { unpackString(flowlike_meta_get_node_id()) }
public func getRunID() -> String { unpackString(flowlike_meta_get_run_id()) }
public func getAppID() -> String { unpackString(flowlike_meta_get_app_id()) }
public func getBoardID() -> String { unpackString(flowlike_meta_get_board_id()) }
public func getUserID() -> String { unpackString(flowlike_meta_get_user_id()) }
public func isStreaming() -> Bool { flowlike_meta_is_streaming() != 0 }
public func getLogLevelValue() -> Int { Int(flowlike_meta_get_log_level()) }
public func timeNow() -> Int64 { flowlike_meta_time_now() }
public func random() -> Int64 { flowlike_meta_random() }

// MARK: - Storage

public func storageRead(_ path: String) -> String {
    let (p, l) = stringToPtr(path)
    return unpackString(flowlike_storage_read_request(p, l))
}

public func storageWrite(_ path: String, _ data: String) -> Bool {
    let (pp, pl) = stringToPtr(path)
    let (dp, dl) = stringToPtr(data)
    return flowlike_storage_write_request(pp, pl, dp, dl) != 0
}

public func storageDir(nodeScoped: Bool) -> String {
    unpackString(flowlike_storage_storage_dir(nodeScoped ? 1 : 0))
}

public func uploadDir() -> String {
    unpackString(flowlike_storage_upload_dir())
}

public func cacheDirPath(nodeScoped: Bool, userScoped: Bool) -> String {
    unpackString(flowlike_storage_cache_dir(nodeScoped ? 1 : 0, userScoped ? 1 : 0))
}

public func userDir(nodeScoped: Bool) -> String {
    unpackString(flowlike_storage_user_dir(nodeScoped ? 1 : 0))
}

public func storageList(_ flowPathJSON: String) -> String {
    let (p, l) = stringToPtr(flowPathJSON)
    return unpackString(flowlike_storage_list_request(p, l))
}

// MARK: - Models

public func embedText(bitJSON: String, textsJSON: String) -> String {
    let (bp, bl) = stringToPtr(bitJSON)
    let (tp, tl) = stringToPtr(textsJSON)
    return unpackString(flowlike_models_embed_text(bp, bl, tp, tl))
}

// MARK: - HTTP

public func httpRequest(method: Int, url: String, headers: String, body: String) -> Bool {
    let (up, ul) = stringToPtr(url)
    let (hp, hl) = stringToPtr(headers)
    let (bp, bl) = stringToPtr(body)
    return flowlike_http_request(Int32(method), up, ul, hp, hl, bp, bl) != 0
}

// MARK: - Streaming

public func streamEmit(eventType: String, data: String) {
    let (ep, el) = stringToPtr(eventType)
    let (dp, dl) = stringToPtr(data)
    flowlike_stream_emit(ep, el, dp, dl)
}

public func streamText(_ text: String) {
    let (p, l) = stringToPtr(text)
    flowlike_stream_text(p, l)
}

// MARK: - Auth

public func getOAuthToken(_ provider: String) -> String {
    let (p, l) = stringToPtr(provider)
    return unpackString(flowlike_auth_get_oauth_token(p, l))
}

public func hasOAuthToken(_ provider: String) -> Bool {
    let (p, l) = stringToPtr(provider)
    return flowlike_auth_has_oauth_token(p, l) != 0
}


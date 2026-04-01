import memory

const hostHeader = "flow_like_host.h"

proc fl_log_trace(p: cstring; l: uint32) {.importc, nodecl, header: hostHeader.}
proc fl_log_debug(p: cstring; l: uint32) {.importc, nodecl, header: hostHeader.}
proc fl_log_info(p: cstring; l: uint32) {.importc, nodecl, header: hostHeader.}
proc fl_log_warn(p: cstring; l: uint32) {.importc, nodecl, header: hostHeader.}
proc fl_log_error(p: cstring; l: uint32) {.importc, nodecl, header: hostHeader.}
proc fl_log_json(level: int32; msgP: cstring; msgL: uint32; dataP: cstring; dataL: uint32) {.importc, nodecl, header: hostHeader.}

proc fl_get_input(nameP: cstring; nameL: uint32): int64 {.importc, nodecl, header: hostHeader.}
proc fl_set_output(nameP: cstring; nameL: uint32; valP: cstring; valL: uint32) {.importc, nodecl, header: hostHeader.}
proc fl_activate_exec(nameP: cstring; nameL: uint32) {.importc, nodecl, header: hostHeader.}

proc fl_var_get(nameP: cstring; nameL: uint32): int64 {.importc, nodecl, header: hostHeader.}
proc fl_var_set(nameP: cstring; nameL: uint32; valP: cstring; valL: uint32) {.importc, nodecl, header: hostHeader.}
proc fl_var_delete(nameP: cstring; nameL: uint32) {.importc, nodecl, header: hostHeader.}
proc fl_var_has(nameP: cstring; nameL: uint32): int32 {.importc, nodecl, header: hostHeader.}

proc fl_cache_get(keyP: cstring; keyL: uint32): int64 {.importc, nodecl, header: hostHeader.}
proc fl_cache_set(keyP: cstring; keyL: uint32; valP: cstring; valL: uint32) {.importc, nodecl, header: hostHeader.}
proc fl_cache_delete(keyP: cstring; keyL: uint32) {.importc, nodecl, header: hostHeader.}
proc fl_cache_has(keyP: cstring; keyL: uint32): int32 {.importc, nodecl, header: hostHeader.}

proc fl_get_node_id(): int64 {.importc, nodecl, header: hostHeader.}
proc fl_get_run_id(): int64 {.importc, nodecl, header: hostHeader.}
proc fl_get_app_id(): int64 {.importc, nodecl, header: hostHeader.}
proc fl_get_board_id(): int64 {.importc, nodecl, header: hostHeader.}
proc fl_get_user_id(): int64 {.importc, nodecl, header: hostHeader.}
proc fl_is_streaming(): int32 {.importc, nodecl, header: hostHeader.}
proc fl_get_log_level(): int32 {.importc, nodecl, header: hostHeader.}
proc fl_time_now(): int64 {.importc, nodecl, header: hostHeader.}
proc fl_random(): int64 {.importc, nodecl, header: hostHeader.}

proc fl_storage_read(pathP: cstring; pathL: uint32): int64 {.importc, nodecl, header: hostHeader.}
proc fl_storage_write(pathP: cstring; pathL: uint32; dataP: cstring; dataL: uint32): int32 {.importc, nodecl, header: hostHeader.}
proc fl_storage_dir(nodeScoped: int32): int64 {.importc, nodecl, header: hostHeader.}
proc fl_upload_dir(): int64 {.importc, nodecl, header: hostHeader.}
proc fl_cache_dir(nodeScoped: int32; userScoped: int32): int64 {.importc, nodecl, header: hostHeader.}
proc fl_user_dir(nodeScoped: int32): int64 {.importc, nodecl, header: hostHeader.}
proc fl_storage_list(pathP: cstring; pathL: uint32): int64 {.importc, nodecl, header: hostHeader.}

proc fl_embed_text(bitP: cstring; bitL: uint32; textsP: cstring; textsL: uint32): int64 {.importc, nodecl, header: hostHeader.}

proc fl_http_request(meth: int32; urlP: cstring; urlL: uint32; hdrP: cstring; hdrL: uint32; bodyP: cstring; bodyL: uint32): int32 {.importc, nodecl, header: hostHeader.}

proc fl_stream_emit(evtP: cstring; evtL: uint32; dataP: cstring; dataL: uint32) {.importc, nodecl, header: hostHeader.}
proc fl_stream_text(txtP: cstring; txtL: uint32) {.importc, nodecl, header: hostHeader.}

proc fl_get_oauth_token(provP: cstring; provL: uint32): int64 {.importc, nodecl, header: hostHeader.}
proc fl_has_oauth_token(provP: cstring; provL: uint32): int32 {.importc, nodecl, header: hostHeader.}

# High-level Nim wrappers

proc logTrace*(msg: string) = fl_log_trace(cstring(msg), uint32(msg.len))
proc logDebug*(msg: string) = fl_log_debug(cstring(msg), uint32(msg.len))
proc logInfo*(msg: string) = fl_log_info(cstring(msg), uint32(msg.len))
proc logWarn*(msg: string) = fl_log_warn(cstring(msg), uint32(msg.len))
proc logError*(msg: string) = fl_log_error(cstring(msg), uint32(msg.len))
proc logJson*(level: int32; msg, data: string) =
  fl_log_json(level, cstring(msg), uint32(msg.len), cstring(data), uint32(data.len))

proc getInput*(name: string): string =
  unpackString(fl_get_input(cstring(name), uint32(name.len)))
proc setOutput*(name, jsonValue: string) =
  fl_set_output(cstring(name), uint32(name.len), cstring(jsonValue), uint32(jsonValue.len))
proc activateExec*(name: string) =
  fl_activate_exec(cstring(name), uint32(name.len))

proc varGet*(name: string): string =
  unpackString(fl_var_get(cstring(name), uint32(name.len)))
proc varSet*(name, value: string) =
  fl_var_set(cstring(name), uint32(name.len), cstring(value), uint32(value.len))
proc varDelete*(name: string) =
  fl_var_delete(cstring(name), uint32(name.len))
proc varHas*(name: string): bool =
  fl_var_has(cstring(name), uint32(name.len)) != 0

proc cacheGet*(key: string): string =
  unpackString(fl_cache_get(cstring(key), uint32(key.len)))
proc cacheSet*(key, value: string) =
  fl_cache_set(cstring(key), uint32(key.len), cstring(value), uint32(value.len))
proc cacheDelete*(key: string) =
  fl_cache_delete(cstring(key), uint32(key.len))
proc cacheHas*(key: string): bool =
  fl_cache_has(cstring(key), uint32(key.len)) != 0

proc metaNodeId*(): string = unpackString(fl_get_node_id())
proc metaRunId*(): string = unpackString(fl_get_run_id())
proc metaAppId*(): string = unpackString(fl_get_app_id())
proc metaBoardId*(): string = unpackString(fl_get_board_id())
proc metaUserId*(): string = unpackString(fl_get_user_id())
proc metaIsStreaming*(): bool = fl_is_streaming() != 0
proc metaLogLevel*(): int32 = fl_get_log_level()
proc metaTimeNow*(): int64 = fl_time_now()
proc metaRandom*(): int64 = fl_random()

proc storageRead*(path: string): string =
  unpackString(fl_storage_read(cstring(path), uint32(path.len)))
proc storageWrite*(path, data: string): int32 =
  fl_storage_write(cstring(path), uint32(path.len), cstring(data), uint32(data.len))
proc storageDir*(nodeScoped: bool): string =
  unpackString(fl_storage_dir(int32(ord(nodeScoped))))
proc uploadDir*(): string = unpackString(fl_upload_dir())
proc cacheDir*(nodeScoped, userScoped: bool): string =
  unpackString(fl_cache_dir(int32(ord(nodeScoped)), int32(ord(userScoped))))
proc userDir*(nodeScoped: bool): string =
  unpackString(fl_user_dir(int32(ord(nodeScoped))))
proc storageList*(path: string): string =
  unpackString(fl_storage_list(cstring(path), uint32(path.len)))

proc embedText*(bit, texts: string): string =
  unpackString(fl_embed_text(cstring(bit), uint32(bit.len), cstring(texts), uint32(texts.len)))

proc httpRequest*(meth: int32; url, headers, body: string): int32 =
  fl_http_request(meth, cstring(url), uint32(url.len), cstring(headers), uint32(headers.len), cstring(body), uint32(body.len))

proc streamEmit*(eventType, data: string) =
  fl_stream_emit(cstring(eventType), uint32(eventType.len), cstring(data), uint32(data.len))
proc streamText*(text: string) =
  fl_stream_text(cstring(text), uint32(text.len))

proc oauthGetToken*(provider: string): string =
  unpackString(fl_get_oauth_token(cstring(provider), uint32(provider.len)))
proc oauthHasToken*(provider: string): bool =
  fl_has_oauth_token(cstring(provider), uint32(provider.len)) != 0

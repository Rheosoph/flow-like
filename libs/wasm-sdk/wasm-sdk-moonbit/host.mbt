///| Raw host function imports (ptr/len ABI).
///  These are provided by the Flow-Like runtime.

// -- Logging --
fn host_log_trace(msg_ptr : Int, msg_len : Int) = "flowlike_log" "trace"
fn host_log_debug(msg_ptr : Int, msg_len : Int) = "flowlike_log" "debug"
fn host_log_info(msg_ptr : Int, msg_len : Int) = "flowlike_log" "info"
fn host_log_warn(msg_ptr : Int, msg_len : Int) = "flowlike_log" "warn"
fn host_log_error(msg_ptr : Int, msg_len : Int) = "flowlike_log" "error"
fn host_log_json(level : Int, msg_ptr : Int, msg_len : Int, data_ptr : Int, data_len : Int) = "flowlike_log" "log_json"

// -- Pin values --
fn host_get_input(name_ptr : Int, name_len : Int) -> Int64 = "flowlike_pins" "get_input"
fn host_set_output(name_ptr : Int, name_len : Int, value_ptr : Int, value_len : Int) = "flowlike_pins" "set_output"
fn host_activate_exec(name_ptr : Int, name_len : Int) = "flowlike_pins" "activate_exec"

// -- Variables --
fn host_get_var(name_ptr : Int, name_len : Int) -> Int64 = "flowlike_vars" "get"
fn host_set_var(name_ptr : Int, name_len : Int, val_ptr : Int, val_len : Int) = "flowlike_vars" "set"
fn host_delete_var(name_ptr : Int, name_len : Int) = "flowlike_vars" "delete"
fn host_has_var(name_ptr : Int, name_len : Int) -> Int = "flowlike_vars" "has"

// -- Streaming --
fn host_stream_emit(type_ptr : Int, type_len : Int, data_ptr : Int, data_len : Int) = "flowlike_stream" "emit"
fn host_stream_text(text_ptr : Int, text_len : Int) = "flowlike_stream" "text"

// -- Metadata --
fn host_time_now() -> Int64 = "flowlike_meta" "time_now"
fn host_random() -> Int64 = "flowlike_meta" "random"
fn host_get_node_id() -> Int64 = "flowlike_meta" "get_node_id"
fn host_get_run_id() -> Int64 = "flowlike_meta" "get_run_id"
fn host_get_app_id() -> Int64 = "flowlike_meta" "get_app_id"
fn host_get_board_id() -> Int64 = "flowlike_meta" "get_board_id"
fn host_get_user_id() -> Int64 = "flowlike_meta" "get_user_id"
fn host_is_streaming() -> Int = "flowlike_meta" "is_streaming"
fn host_get_log_level() -> Int = "flowlike_meta" "get_log_level"

// -- Cache --
fn host_cache_get(key_ptr : Int, key_len : Int) -> Int64 = "flowlike_cache" "get"
fn host_cache_set(key_ptr : Int, key_len : Int, val_ptr : Int, val_len : Int) = "flowlike_cache" "set"
fn host_cache_delete(key_ptr : Int, key_len : Int) = "flowlike_cache" "delete"
fn host_cache_has(key_ptr : Int, key_len : Int) -> Int = "flowlike_cache" "has"

// -- Storage --
fn host_storage_dir(node_scoped : Int) -> Int64 = "flowlike_storage" "storage_dir"
fn host_upload_dir() -> Int64 = "flowlike_storage" "upload_dir"
fn host_cache_dir(node_scoped : Int, user_scoped : Int) -> Int64 = "flowlike_storage" "cache_dir"
fn host_user_dir(node_scoped : Int) -> Int64 = "flowlike_storage" "user_dir"
fn host_storage_read(path_ptr : Int, path_len : Int) -> Int64 = "flowlike_storage" "read_request"
fn host_storage_write(path_ptr : Int, path_len : Int, data_ptr : Int, data_len : Int) -> Int = "flowlike_storage" "write_request"
fn host_storage_list(path_ptr : Int, path_len : Int) -> Int64 = "flowlike_storage" "list_request"

// -- Models --
fn host_embed_text(bit_ptr : Int, bit_len : Int, texts_ptr : Int, texts_len : Int) -> Int64 = "flowlike_models" "embed_text"

// -- HTTP --
fn host_http_request(http_method : Int, url_ptr : Int, url_len : Int, headers_ptr : Int, headers_len : Int, body_ptr : Int, body_len : Int) -> Int = "flowlike_http" "request"

// -- Auth --
fn host_get_oauth_token(prov_ptr : Int, prov_len : Int) -> Int64 = "flowlike_auth" "get_oauth_token"
fn host_has_oauth_token(prov_ptr : Int, prov_len : Int) -> Int = "flowlike_auth" "has_oauth_token"

fn unpack_optional_string(packed : Int64) -> String? {
  let ptr = unpack_ptr(packed)
  let len = unpack_len(packed)
  if ptr == 0 || len == 0 {
    None
  } else {
    Some(read_string_from_mem(ptr, len))
  }
}

// ── Logging ─────────────────────────────────────────────────────────────────

///| Log a message at the given level.
pub fn log(level : LogLevel, message : String) -> Unit {
  let (ptr, len) = write_string_to_mem(message)
  match level {
    LogLevel::Trace => host_log_trace(ptr, len)
    LogLevel::Debug => host_log_debug(ptr, len)
    LogLevel::Info => host_log_info(ptr, len)
    LogLevel::Warn => host_log_warn(ptr, len)
    LogLevel::Error => host_log_error(ptr, len)
    LogLevel::Fatal => host_log_error(ptr, len)
  }
}

///| Convenience: log at debug level.
pub fn debug(message : String) -> Unit { log(LogLevel::Debug, message) }

///| Convenience: log at info level.
pub fn info(message : String) -> Unit { log(LogLevel::Info, message) }

///| Convenience: log at warn level.
pub fn warn(message : String) -> Unit { log(LogLevel::Warn, message) }

///| Convenience: log at error level.
pub fn error(message : String) -> Unit { log(LogLevel::Error, message) }

///| Convenience: log at trace level.
pub fn trace(message : String) -> Unit { log(LogLevel::Trace, message) }

///| Structured JSON logging.
pub fn log_json(level : LogLevel, message : String, data_json : String) -> Unit {
  let (mp, ml) = write_string_to_mem(message)
  let (dp, dl) = write_string_to_mem(data_json)
  host_log_json(level.to_int(), mp, ml, dp, dl)
}

// ── Streaming ───────────────────────────────────────────────────────────────

///| Send a stream event.
pub fn stream(event_type : String, data : String) -> Unit {
  let (tp, tl) = write_string_to_mem(event_type)
  let (dp, dl) = write_string_to_mem(data)
  host_stream_emit(tp, tl, dp, dl)
}

///| Stream text via the emit pathway (event_type="text").
pub fn stream_text(content : String) -> Unit {
  stream("text", content)
}

///| Stream text directly via the raw text host call.
pub fn stream_text_raw(content : String) -> Unit {
  let (ptr, len) = write_string_to_mem(content)
  host_stream_text(ptr, len)
}

///| Stream a pre-serialized JSON string.
pub fn stream_json(data_json : String) -> Unit {
  stream("json", data_json)
}

///| Stream a progress event.
pub fn stream_progress(progress : Double, message : String) -> Unit {
  let payload = "{\"progress\":" + progress.to_string() + ",\"message\":" + json_string(message) + "}"
  stream("progress", payload)
}

///| Get an input pin by name.
pub fn get_input(name : String) -> String? {
  let (np, nl) = write_string_to_mem(name)
  unpack_optional_string(host_get_input(np, nl))
}

///| Set an output pin value.
pub fn set_output(name : String, value : String) -> Unit {
  let (np, nl) = write_string_to_mem(name)
  let (vp, vl) = write_string_to_mem(value)
  host_set_output(np, nl, vp, vl)
}

///| Activate an exec output pin.
pub fn activate_exec(name : String) -> Unit {
  let (np, nl) = write_string_to_mem(name)
  host_activate_exec(np, nl)
}

///| Get a variable by name. Returns None when absent.
pub fn get_variable(name : String) -> String? {
  let (np, nl) = write_string_to_mem(name)
  unpack_optional_string(host_get_var(np, nl))
}

///| Set a variable.
pub fn set_variable(name : String, value : String) -> Bool {
  let (np, nl) = write_string_to_mem(name)
  let (vp, vl) = write_string_to_mem(value)
  host_set_var(np, nl, vp, vl)
  true
}

///| Delete a variable.
pub fn delete_variable(name : String) -> Unit {
  let (np, nl) = write_string_to_mem(name)
  host_delete_var(np, nl)
}

///| Check whether a variable exists.
pub fn has_variable(name : String) -> Bool {
  let (np, nl) = write_string_to_mem(name)
  host_has_var(np, nl) != 0
}

///| Get a value from the cache via key.
pub fn cache_get(key : String) -> String? {
  let (kp, kl) = write_string_to_mem(key)
  unpack_optional_string(host_cache_get(kp, kl))
}

///| Store a value in the cache.
pub fn cache_set(key : String, value : String) -> Unit {
  let (kp, kl) = write_string_to_mem(key)
  let (vp, vl) = write_string_to_mem(value)
  host_cache_set(kp, kl, vp, vl)
}

///| Delete a cache entry.
pub fn cache_delete(key : String) -> Unit {
  let (kp, kl) = write_string_to_mem(key)
  host_cache_delete(kp, kl)
}

///| Check whether a cache key exists.
pub fn cache_has(key : String) -> Bool {
  let (kp, kl) = write_string_to_mem(key)
  host_cache_has(kp, kl) != 0
}

///| Current time from the host.
pub fn time_now() -> Int64 {
  host_time_now()
}

///| Random value from the host runtime.
pub fn random_bits() -> Int64 {
  host_random()
}

///| Get the node ID from the host runtime.
pub fn get_node_id_from_host() -> String? {
  unpack_optional_string(host_get_node_id())
}

///| Get the run ID from the host runtime.
pub fn get_run_id_from_host() -> String? {
  unpack_optional_string(host_get_run_id())
}

///| Get the app ID from the host runtime.
pub fn get_app_id_from_host() -> String? {
  unpack_optional_string(host_get_app_id())
}

///| Get the board ID from the host runtime.
pub fn get_board_id_from_host() -> String? {
  unpack_optional_string(host_get_board_id())
}

///| Get the user ID from the host runtime.
pub fn get_user_id_from_host() -> String? {
  unpack_optional_string(host_get_user_id())
}

///| Check whether streaming is enabled in the host.
pub fn is_streaming_from_host() -> Bool {
  host_is_streaming() != 0
}

///| Get the current log level from the host.
pub fn get_log_level_from_host() -> LogLevel {
  LogLevel::from_int(host_get_log_level())
}

///| Get the node-scoped or shared storage directory.
pub fn storage_dir(node_scoped : Bool) -> String? {
  unpack_optional_string(host_storage_dir(if node_scoped { 1 } else { 0 }))
}

///| Get the upload directory.
pub fn upload_dir() -> String? {
  unpack_optional_string(host_upload_dir())
}

///| Get a cache directory.
pub fn cache_dir(node_scoped : Bool, user_scoped : Bool) -> String? {
  unpack_optional_string(host_cache_dir(
    if node_scoped { 1 } else { 0 },
    if user_scoped { 1 } else { 0 },
  ))
}

///| Get the user directory.
pub fn user_dir(node_scoped : Bool) -> String? {
  unpack_optional_string(host_user_dir(if node_scoped { 1 } else { 0 }))
}

///| Read bytes from a storage path (returned as JSON string of the FlowPath).
pub fn storage_read(flow_path_json : String) -> String? {
  let (pp, pl) = write_string_to_mem(flow_path_json)
  unpack_optional_string(host_storage_read(pp, pl))
}

///| Write bytes to a storage path.
pub fn storage_write(flow_path_json : String, data : String) -> Bool {
  let (pp, pl) = write_string_to_mem(flow_path_json)
  let (dp, dl) = write_string_to_mem(data)
  host_storage_write(pp, pl, dp, dl) != 0
}

///| List entries at a storage path.
pub fn storage_list(flow_path_json : String) -> String? {
  let (pp, pl) = write_string_to_mem(flow_path_json)
  unpack_optional_string(host_storage_list(pp, pl))
}

///| Embed text using a model.
pub fn embed_text(bit_json : String, texts_json : String) -> String? {
  let (bp, bl) = write_string_to_mem(bit_json)
  let (tp, tl) = write_string_to_mem(texts_json)
  unpack_optional_string(host_embed_text(bp, bl, tp, tl))
}

///| Make an HTTP request.
pub fn http_request(http_method : Int, url : String, headers : String, body : String) -> Bool {
  let (up, ul) = write_string_to_mem(url)
  let (hp, hl) = write_string_to_mem(headers)
  let (bp, bl) = write_string_to_mem(body)
  host_http_request(http_method, up, ul, hp, hl, bp, bl) != 0
}

///| Get an OAuth token for the given provider.
pub fn get_oauth_token(provider : String) -> String? {
  let (pp, pl) = write_string_to_mem(provider)
  unpack_optional_string(host_get_oauth_token(pp, pl))
}

///| Check whether an OAuth token exists for the provider.
pub fn has_oauth_token(provider : String) -> Bool {
  let (pp, pl) = write_string_to_mem(provider)
  host_has_oauth_token(pp, pl) != 0
}

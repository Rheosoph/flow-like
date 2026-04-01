///| Execution context — high-level façade over the host bridge.

pub struct Context {
  pub input : ExecutionInput
  result : ExecutionResult
}

pub fn Context::new(input : ExecutionInput) -> Context {
  { input, result: ExecutionResult::ok() }
}

// ── Accessor methods ────────────────────────────────────────────────────────

///| Get the node ID.
pub fn Context::node_id(self : Context) -> String { self.input.node_id }

///| Get the node name (for multi-node packages).
pub fn Context::node_name(self : Context) -> String { self.input.node_name }

///| Get the run ID.
pub fn Context::run_id(self : Context) -> String { self.input.run_id }

///| Get the app ID.
pub fn Context::app_id(self : Context) -> String { self.input.app_id }

///| Get the board ID.
pub fn Context::board_id(self : Context) -> String { self.input.board_id }

///| Get the user ID.
pub fn Context::user_id(self : Context) -> String { self.input.user_id }

///| Check whether streaming is enabled.
pub fn Context::stream_enabled(self : Context) -> Bool { self.input.stream_state }

///| Get the current log level.
pub fn Context::log_level(self : Context) -> LogLevel { self.input.log_level }

// ── Input getters ───────────────────────────────────────────────────────────

///| Get the raw JSON value for an input pin.
pub fn Context::get_input(self : Context, name : String) -> JsonValue? {
  self.input.inputs.get(name)
}

///| Get a required input, returning an error string if missing.
pub fn Context::require_input(self : Context, name : String) -> Result[JsonValue, String] {
  match self.get_input(name) {
    Some(v) => Ok(v)
    None => Err("Missing required input: " + name)
  }
}

///| Get a string input with a default value.
pub fn Context::get_string(self : Context, name : String, default~ : String = "") -> String {
  match self.get_input(name) {
    Some(Str(s)) => s
    Some(v) => v.to_raw_json()
    None => default
  }
}

///| Get an integer input with a default value.
pub fn Context::get_i64(self : Context, name : String, default~ : Int = 0) -> Int {
  match self.get_input(name) {
    Some(Number(n)) => n.to_int()
    _ => default
  }
}

///| Get a float input with a default value.
pub fn Context::get_f64(self : Context, name : String, default~ : Double = 0.0) -> Double {
  match self.get_input(name) {
    Some(Number(n)) => n
    _ => default
  }
}

///| Get a boolean input with a default value.
pub fn Context::get_bool(self : Context, name : String, default~ : Bool = false) -> Bool {
  match self.get_input(name) {
    Some(Bool(b)) => b
    _ => default
  }
}

// ── Output setters ──────────────────────────────────────────────────────────

///| Set an output pin value (raw JSON string).
pub fn Context::set_output(self : Context, name : String, value : String) -> Unit {
  self.result.set_output(name, value)
}

///| Activate an exec output pin.
pub fn Context::activate_exec(self : Context, pin_name : String) -> Unit {
  self.result.exec(pin_name)
}

///| Set the pending state.
pub fn Context::set_pending(self : Context, pending : Bool) -> Unit {
  self.result.set_pending(pending)
}

///| Set an error message without finalizing the result.
pub fn Context::set_error(self : Context, err : String) -> Unit {
  self.result.error = Some(err)
}

// ── Level-gated logging ─────────────────────────────────────────────────────

///| Check whether a given log level should be emitted.
pub fn Context::should_log(self : Context, level : LogLevel) -> Bool {
  self.input.log_level <= level
}

pub fn Context::trace(self : Context, msg : String) -> Unit {
  if self.should_log(LogLevel::Trace) { log(LogLevel::Trace, msg) }
}

pub fn Context::debug(self : Context, msg : String) -> Unit {
  if self.should_log(LogLevel::Debug) { log(LogLevel::Debug, msg) }
}

pub fn Context::info(self : Context, msg : String) -> Unit {
  if self.should_log(LogLevel::Info) { log(LogLevel::Info, msg) }
}

pub fn Context::warn(self : Context, msg : String) -> Unit {
  if self.should_log(LogLevel::Warn) { log(LogLevel::Warn, msg) }
}

pub fn Context::error(self : Context, msg : String) -> Unit {
  if self.should_log(LogLevel::Error) { log(LogLevel::Error, msg) }
}

// ── Streaming (only when enabled) ───────────────────────────────────────────

pub fn Context::stream_text(self : Context, text : String) -> Unit {
  if self.input.stream_state { stream("text", text) }
}

pub fn Context::stream_json(self : Context, data : String) -> Unit {
  if self.input.stream_state { stream("json", data) }
}

pub fn Context::stream_progress(self : Context, progress : Double, message : String) -> Unit {
  if self.input.stream_state {
    stream_progress(progress, message)
  }
}

// ── Variables ───────────────────────────────────────────────────────────────

pub fn Context::get_variable(self : Context, name : String) -> String? {
  ignore(self)
  get_variable(name)
}

pub fn Context::set_variable(self : Context, name : String, value : String) -> Bool {
  ignore(self)
  set_variable(name, value)
}

pub fn Context::delete_variable(self : Context, name : String) -> Unit {
  ignore(self)
  delete_variable(name)
}

pub fn Context::has_variable(self : Context, name : String) -> Bool {
  ignore(self)
  has_variable(name)
}

// ── Cache ───────────────────────────────────────────────────────────────────

pub fn Context::cache_get(self : Context, key : String) -> String? {
  ignore(self)
  cache_get(key)
}

pub fn Context::cache_set(self : Context, key : String, value : String) -> Unit {
  ignore(self)
  cache_set(key, value)
}

pub fn Context::cache_delete(self : Context, key : String) -> Unit {
  ignore(self)
  cache_delete(key)
}

pub fn Context::cache_has(self : Context, key : String) -> Bool {
  ignore(self)
  cache_has(key)
}

// ── Storage ─────────────────────────────────────────────────────────────────

pub fn Context::storage_dir(self : Context, node_scoped~ : Bool = false) -> String? {
  ignore(self)
  storage_dir(node_scoped)
}

pub fn Context::upload_dir(self : Context) -> String? {
  ignore(self)
  upload_dir()
}

pub fn Context::cache_dir(self : Context, node_scoped~ : Bool = false, user_scoped~ : Bool = false) -> String? {
  ignore(self)
  cache_dir(node_scoped, user_scoped)
}

pub fn Context::user_dir(self : Context, node_scoped~ : Bool = false) -> String? {
  ignore(self)
  user_dir(node_scoped)
}

pub fn Context::storage_read(self : Context, flow_path_json : String) -> String? {
  ignore(self)
  storage_read(flow_path_json)
}

pub fn Context::storage_write(self : Context, flow_path_json : String, data : String) -> Bool {
  ignore(self)
  storage_write(flow_path_json, data)
}

pub fn Context::storage_list(self : Context, flow_path_json : String) -> String? {
  ignore(self)
  storage_list(flow_path_json)
}

// ── Models ──────────────────────────────────────────────────────────────────

pub fn Context::embed_text(self : Context, bit_json : String, texts_json : String) -> String? {
  ignore(self)
  embed_text(bit_json, texts_json)
}

// ── HTTP ────────────────────────────────────────────────────────────────────

pub fn Context::http_request(self : Context, http_method : Int, url : String, headers : String, body : String) -> Bool {
  ignore(self)
  http_request(http_method, url, headers, body)
}

pub fn Context::http_get(self : Context, url : String, headers~ : String = "{}") -> Bool {
  ignore(self)
  http_request(0, url, headers, "")
}

pub fn Context::http_post(self : Context, url : String, body~ : String = "", headers~ : String = "{}") -> Bool {
  ignore(self)
  http_request(1, url, headers, body)
}

// ── Auth ────────────────────────────────────────────────────────────────────

pub fn Context::get_oauth_token(self : Context, provider : String) -> String? {
  ignore(self)
  get_oauth_token(provider)
}

pub fn Context::has_oauth_token(self : Context, provider : String) -> Bool {
  ignore(self)
  has_oauth_token(provider)
}

// ── Finalization ────────────────────────────────────────────────────────────

///| Mark execution as successful, activate "exec_out", and return the result.
pub fn Context::success(self : Context) -> ExecutionResult {
  self.result.exec("exec_out")
  self.result
}

///| Mark execution as failed and return the result.
pub fn Context::fail(self : Context, err : String) -> ExecutionResult {
  self.result.error = Some(err)
  self.result
}

///| Return the result as-is (no automatic exec activation).
pub fn Context::finish(self : Context) -> ExecutionResult {
  self.result
}

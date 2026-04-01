///| ABI version — must match the host runtime
pub let abi_version : Int = 1

///| Log severity levels
pub enum LogLevel {
  Trace = 0
  Debug = 1
  Info = 2
  Warn = 3
  Error = 4
  Fatal = 5
} derive(Eq, Compare)

pub fn LogLevel::from_int(n : Int) -> LogLevel {
  match n {
    0 => Trace
    1 => Debug
    2 => Info
    3 => Warn
    4 => Error
    _ => Fatal
  }
}

pub fn LogLevel::to_int(self : LogLevel) -> Int {
  match self {
    Trace => 0
    Debug => 1
    Info => 2
    Warn => 3
    Error => 4
    Fatal => 5
  }
}

///| Pin data types
pub enum DataType {
  Exec
  StringType
  I64Type
  F64Type
  BoolType
  Generic
  ByteType
  DateTime
  PathBuf
  StructType
}

pub fn DataType::json_name(self : DataType) -> String {
  match self {
    Exec => "Exec"
    StringType => "String"
    I64Type => "I64"
    F64Type => "F64"
    BoolType => "Bool"
    Generic => "Generic"
    ByteType => "Bytes"
    DateTime => "Date"
    PathBuf => "PathBuf"
    StructType => "Struct"
  }
}

pub fn data_type_exec() -> DataType { Exec }

pub fn data_type_string() -> DataType { StringType }

pub fn data_type_i64() -> DataType { I64Type }

pub fn data_type_f64() -> DataType { F64Type }

pub fn data_type_bool() -> DataType { BoolType }

pub fn data_type_generic() -> DataType { Generic }

pub fn data_type_bytes() -> DataType { ByteType }

pub fn data_type_datetime() -> DataType { DateTime }

pub fn data_type_pathbuf() -> DataType { PathBuf }

pub fn data_type_struct() -> DataType { StructType }

///| Value container types
pub enum ValueType {
  Normal
  ArrayType
  HashMap
  HashSet
}

pub fn ValueType::json_name(self : ValueType) -> String {
  match self {
    Normal => "Normal"
    ArrayType => "Array"
    HashMap => "HashMap"
    HashSet => "HashSet"
  }
}

///| Convert a snake_case name to Friendly Name
pub fn humanize(name : String) -> String {
  let mut result = ""
  let mut capitalize_next = true
  let mut first = true
  for c in name {
    if c == '_' {
      result = result + " "
      capitalize_next = true
    } else if capitalize_next {
      if not(first) || true {
        let code = c.to_int()
        if code >= 97 && code <= 122 {
          result = result + Int::unsafe_to_char(code - 32).to_string()
        } else {
          result = result + c.to_string()
        }
      }
      capitalize_next = false
    } else {
      result = result + c.to_string()
    }
    first = false
  }
  result
}

///| Escape a string for JSON output
pub fn json_escape_string(s : String) -> String {
  let mut result = ""
  for c in s {
    match c {
      '"' => result = result + "\\\""
      '\\' => result = result + "\\\\"
      '\n' => result = result + "\\n"
      '\r' => result = result + "\\r"
      '\t' => result = result + "\\t"
      _ => {
        let code = c.to_int()
        if code < 0x20 {
          let hex = "0123456789abcdef"
          let d3 = hex[(code >> 12) & 0xF].to_string()
          let d2 = hex[(code >> 8) & 0xF].to_string()
          let d1 = hex[(code >> 4) & 0xF].to_string()
          let d0 = hex[code & 0xF].to_string()
          result = result + "\\u" + d3 + d2 + d1 + d0
        } else {
          result = result + c.to_string()
        }
      }
    }
  }
  result
}

///| Wrap a value as a JSON string literal
pub fn json_string(s : String) -> String {
  "\"" + json_escape_string(s) + "\""
}

///| Node quality scores (0–10 scale, higher is better)
pub struct NodeScores {
  privacy : Int
  security : Int
  performance : Int
  governance : Int
  reliability : Int
  cost : Int
}

pub fn NodeScores::new() -> NodeScores {
  { privacy: 0, security: 0, performance: 0, governance: 0, reliability: 0, cost: 0 }
}

pub fn NodeScores::to_json(self : NodeScores) -> String {
  let mut s = "{\"privacy\":" + self.privacy.to_string()
  s = s + ",\"security\":" + self.security.to_string()
  s = s + ",\"performance\":" + self.performance.to_string()
  s = s + ",\"governance\":" + self.governance.to_string()
  s = s + ",\"reliability\":" + self.reliability.to_string()
  s = s + ",\"cost\":" + self.cost.to_string()
  s + "}"
}

///| Pin definition (builder pattern via struct update)
pub struct PinDefinition {
  pub name : String
  pub friendly_name : String
  pub description : String
  pub pin_type : String
  pub data_type : DataType
  pub default_value : String?
  pub value_type : ValueType?
  pub schema : String?
  pub options : Array[String]?
  pub range : (Double, Double)?
  pub step : Double?
  pub sensitive : Bool?
  pub enforce_schema : Bool?
  pub enforce_generic_value_type : Bool?
}

pub fn input_pin(
  name : String,
  friendly_name : String,
  description : String,
  data_type : DataType
) -> PinDefinition {
  {
    name,
    friendly_name,
    description,
    pin_type: "Input",
    data_type,
    default_value: None,
    value_type: None,
    schema: None,
    options: None,
    range: None,
    step: None,
    sensitive: None,
    enforce_schema: None,
    enforce_generic_value_type: None,
  }
}

pub fn output_pin(
  name : String,
  friendly_name : String,
  description : String,
  data_type : DataType
) -> PinDefinition {
  {
    name,
    friendly_name,
    description,
    pin_type: "Output",
    data_type,
    default_value: None,
    value_type: None,
    schema: None,
    options: None,
    range: None,
    step: None,
    sensitive: None,
    enforce_schema: None,
    enforce_generic_value_type: None,
  }
}

pub fn PinDefinition::with_default(self : PinDefinition, value : String) -> PinDefinition {
  { ..self, default_value: Some(value) }
}

pub fn PinDefinition::with_value_type(self : PinDefinition, vt : ValueType) -> PinDefinition {
  { ..self, value_type: Some(vt) }
}

pub fn PinDefinition::with_schema(self : PinDefinition, schema : String) -> PinDefinition {
  { ..self, schema: Some(schema) }
}

pub fn PinDefinition::with_options(self : PinDefinition, opts : Array[String]) -> PinDefinition {
  { ..self, options: Some(opts) }
}

pub fn PinDefinition::with_range(self : PinDefinition, min : Double, max : Double) -> PinDefinition {
  { ..self, range: Some((min, max)) }
}

pub fn PinDefinition::with_step(self : PinDefinition, step : Double) -> PinDefinition {
  { ..self, step: Some(step) }
}

pub fn PinDefinition::with_sensitive(self : PinDefinition, sensitive : Bool) -> PinDefinition {
  { ..self, sensitive: Some(sensitive) }
}

pub fn PinDefinition::with_enforce_schema(self : PinDefinition, enforce : Bool) -> PinDefinition {
  { ..self, enforce_schema: Some(enforce) }
}

pub fn PinDefinition::with_enforce_generic_value_type(self : PinDefinition, enforce : Bool) -> PinDefinition {
  { ..self, enforce_generic_value_type: Some(enforce) }
}

pub fn PinDefinition::to_json(self : PinDefinition) -> String {
  let mut s = "{\"name\":" + json_string(self.name)
  s = s + ",\"friendly_name\":" + json_string(self.friendly_name)
  s = s + ",\"description\":" + json_string(self.description)
  s = s + ",\"pin_type\":" + json_string(self.pin_type)
  s = s + ",\"data_type\":" + json_string(self.data_type.json_name())
  match self.default_value {
    Some(v) => s = s + ",\"default_value\":" + v
    None => ()
  }
  match self.value_type {
    Some(vt) => s = s + ",\"value_type\":" + json_string(vt.json_name())
    None => ()
  }
  match self.schema {
    Some(sc) => s = s + ",\"schema\":" + json_string(sc)
    None => ()
  }
  match self.options {
    Some(opts) => {
      let mut arr = "["
      for i = 0; i < opts.length(); i = i + 1 {
        if i > 0 { arr = arr + "," }
        arr = arr + json_string(opts[i])
      }
      arr = arr + "]"
      s = s + ",\"valid_values\":" + arr
    }
    None => ()
  }
  match self.range {
    Some((lo, hi)) => s = s + ",\"range\":[" + lo.to_string() + "," + hi.to_string() + "]"
    None => ()
  }
  match self.step {
    Some(st) => s = s + ",\"step\":" + st.to_string()
    None => ()
  }
  match self.sensitive {
    Some(sn) => s = s + ",\"sensitive\":" + sn.to_string()
    None => ()
  }
  match self.enforce_schema {
    Some(es) => s = s + ",\"enforce_schema\":" + es.to_string()
    None => ()
  }
  match self.enforce_generic_value_type {
    Some(eg) => s = s + ",\"enforce_generic_value_type\":" + eg.to_string()
    None => ()
  }
  s + "}"
}

///| Node definition (mutable for incremental building)
pub struct NodeDefinition {
  pub name : String
  pub friendly_name : String
  pub description : String
  pub category : String
  pub mut icon : String?
  pub pins : Array[PinDefinition]
  pub mut scores : NodeScores?
  pub mut long_running : Bool?
  pub mut docs : String?
  pub permissions : Array[String]
}

pub fn NodeDefinition::new(
  name : String,
  friendly_name : String,
  description : String,
  category : String
) -> NodeDefinition {
  {
    name,
    friendly_name,
    description,
    category,
    icon: None,
    pins: [],
    scores: None,
    long_running: None,
    docs: None,
    permissions: [],
  }
}

pub fn NodeDefinition::add_pin(self : NodeDefinition, pin : PinDefinition) -> Unit {
  self.pins.push(pin)
}

pub fn NodeDefinition::add_permission(self : NodeDefinition, perm : String) -> Unit {
  self.permissions.push(perm)
}

pub fn NodeDefinition::set_scores(self : NodeDefinition, scores : NodeScores) -> Unit {
  self.scores = Some(scores)
}

pub fn NodeDefinition::set_long_running(self : NodeDefinition, lr : Bool) -> Unit {
  self.long_running = Some(lr)
}

pub fn NodeDefinition::set_icon(self : NodeDefinition, icon : String) -> Unit {
  self.icon = Some(icon)
}

pub fn NodeDefinition::set_docs(self : NodeDefinition, docs : String) -> Unit {
  self.docs = Some(docs)
}

pub fn NodeDefinition::to_json(self : NodeDefinition) -> String {
  let mut s = "{\"name\":" + json_string(self.name)
  s = s + ",\"friendly_name\":" + json_string(self.friendly_name)
  s = s + ",\"description\":" + json_string(self.description)
  s = s + ",\"category\":" + json_string(self.category)
  s = s + ",\"abi_version\":" + abi_version.to_string()
  match self.icon {
    Some(ic) => s = s + ",\"icon\":" + json_string(ic)
    None => ()
  }
  // Pins
  let mut pins_json = "["
  for i = 0; i < self.pins.length(); i = i + 1 {
    if i > 0 { pins_json = pins_json + "," }
    pins_json = pins_json + self.pins[i].to_json()
  }
  pins_json = pins_json + "]"
  s = s + ",\"pins\":" + pins_json
  match self.scores {
    Some(sc) => s = s + ",\"scores\":" + sc.to_json()
    None => ()
  }
  match self.long_running {
    Some(lr) => s = s + ",\"long_running\":" + lr.to_string()
    None => ()
  }
  match self.docs {
    Some(d) => s = s + ",\"docs\":" + json_string(d)
    None => ()
  }
  if self.permissions.length() > 0 {
    let mut perms = "["
    for i = 0; i < self.permissions.length(); i = i + 1 {
      if i > 0 { perms = perms + "," }
      perms = perms + json_string(self.permissions[i])
    }
    perms = perms + "]"
    s = s + ",\"permissions\":" + perms
  }
  s + "}"
}

///| Execution input (parsed from host JSON)
pub struct ExecutionInput {
  pub inputs : JsonValue
  pub node_id : String
  pub run_id : String
  pub app_id : String
  pub board_id : String
  pub user_id : String
  pub stream_state : Bool
  pub log_level : LogLevel
  pub node_name : String
}

pub fn ExecutionInput::from_json(val : JsonValue) -> ExecutionInput {
  {
    inputs: val.get("inputs").unwrap_or(JsonValue::Obj([])),
    node_id: val.get("node_id").bind(JsonValue::as_string).unwrap_or(""),
    run_id: val.get("run_id").bind(JsonValue::as_string).unwrap_or(""),
    app_id: val.get("app_id").bind(JsonValue::as_string).unwrap_or(""),
    board_id: val.get("board_id").bind(JsonValue::as_string).unwrap_or(""),
    user_id: val.get("user_id").bind(JsonValue::as_string).unwrap_or(""),
    stream_state: val.get("stream_state").bind(JsonValue::as_bool).unwrap_or(false),
    log_level: LogLevel::from_int(val.get("log_level").bind(JsonValue::as_int).unwrap_or(1)),
    node_name: val.get("node_name").bind(JsonValue::as_string).unwrap_or(""),
  }
}

///| Execution result (built incrementally, serialized to JSON)
pub struct ExecutionResult {
  pub outputs : Array[(String, String)]
  pub mut error : String?
  pub activate_exec : Array[String]
  pub mut pending : Bool?
}

pub fn ExecutionResult::ok() -> ExecutionResult {
  { outputs: [], error: None, activate_exec: [], pending: None }
}

pub fn ExecutionResult::fail(message : String) -> ExecutionResult {
  { outputs: [], error: Some(message), activate_exec: [], pending: None }
}

pub fn ExecutionResult::set_output(self : ExecutionResult, name : String, value : String) -> Unit {
  self.outputs.push((name, value))
}

pub fn ExecutionResult::exec(self : ExecutionResult, pin_name : String) -> Unit {
  self.activate_exec.push(pin_name)
}

pub fn ExecutionResult::set_pending(self : ExecutionResult, pending : Bool) -> Unit {
  self.pending = Some(pending)
}

pub fn ExecutionResult::to_json(self : ExecutionResult) -> String {
  let mut s = "{\"outputs\":{"
  for i = 0; i < self.outputs.length(); i = i + 1 {
    if i > 0 { s = s + "," }
    let (k, v) = self.outputs[i]
    s = s + json_string(k) + ":" + v
  }
  s = s + "},\"activate_exec\":["
  for i = 0; i < self.activate_exec.length(); i = i + 1 {
    if i > 0 { s = s + "," }
    s = s + json_string(self.activate_exec[i])
  }
  s = s + "]"
  match self.error {
    Some(e) => s = s + ",\"error\":" + json_string(e)
    None => ()
  }
  match self.pending {
    Some(p) => s = s + ",\"pending\":" + p.to_string()
    None => ()
  }
  s + "}"
}

// ── Multi-node package container ────────────────────────────────────────────

///| Container for multiple node definitions (multi-node packages).
pub struct PackageNodes {
  pub nodes : Array[NodeDefinition]
}

pub fn PackageNodes::new() -> PackageNodes {
  { nodes: [] }
}

pub fn PackageNodes::add_node(self : PackageNodes, def : NodeDefinition) -> Unit {
  self.nodes.push(def)
}

pub fn PackageNodes::to_json(self : PackageNodes) -> String {
  let mut s = "["
  for i = 0; i < self.nodes.length(); i = i + 1 {
    if i > 0 { s = s + "," }
    s = s + self.nodes[i].to_json()
  }
  s + "]"
}

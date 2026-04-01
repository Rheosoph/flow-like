import std/[tables, strutils, strformat]

const ABI_VERSION* = 1'u32

const
  LogLevelTrace* = 0'u8
  LogLevelDebug* = 1'u8
  LogLevelInfo* = 2'u8
  LogLevelWarn* = 3'u8
  LogLevelError* = 4'u8
  LogLevelFatal* = 5'u8

type
  PinType* = enum
    Input = "Input"
    Output = "Output"

  DataType* = enum
    Exec = "Exec"
    String = "String"
    I64 = "I64"
    F64 = "F64"
    Bool = "Bool"
    Generic = "Generic"
    Bytes = "Bytes"
    Date = "Date"
    PathBuf = "PathBuf"
    Struct = "Struct"

proc dataTypeStr*(dt: DataType): string =
  case dt
  of Exec: "Exec"
  of String: "String"
  of I64: "I64"
  of F64: "F64"
  of Bool: "Bool"
  of Generic: "Generic"
  of Bytes: "Bytes"
  of Date: "Date"
  of PathBuf: "PathBuf"
  of Struct: "Struct"

type
  ValueType* = enum
    Normal = "Normal"
    Array = "Array"
    HashMap = "HashMap"
    HashSet = "HashSet"

proc valueTypeStr*(vt: ValueType): string =
  case vt
  of Normal: "Normal"
  of Array: "Array"
  of HashMap: "HashMap"
  of HashSet: "HashSet"

proc jsonQuote*(s: string): string =
  result = newStringOfCap(s.len + 2)
  result.add '"'
  for c in s:
    case c
    of '"': result.add "\\\""
    of '\\': result.add "\\\\"
    of '\n': result.add "\\n"
    of '\r': result.add "\\r"
    of '\t': result.add "\\t"
    else:
      if ord(c) < 0x20:
        result.add fmt"\u{ord(c):04x}"
      else:
        result.add c
  result.add '"'

type
  NodeScores* = object
    privacy*: uint8
    security*: uint8
    performance*: uint8
    governance*: uint8
    reliability*: uint8
    cost*: uint8

proc toJson*(s: NodeScores): string =
  "{\"privacy\":" & $s.privacy &
  ",\"security\":" & $s.security &
  ",\"performance\":" & $s.performance &
  ",\"governance\":" & $s.governance &
  ",\"reliability\":" & $s.reliability &
  ",\"cost\":" & $s.cost & "}"

type
  PinDefinition* = object
    name*: string
    friendlyName*: string
    description*: string
    pinType*: PinType
    dataType*: DataType
    defaultValue*: string
    valueType*: string
    schema*: string
    validValues*: seq[string]
    range*: tuple[min: float64, max: float64]
    hasRange*: bool
    step*: float64
    hasStep*: bool
    sensitive*: bool
    hasSensitive*: bool
    enforceSchema*: bool
    hasEnforceSchema*: bool
    enforceGenericValueType*: bool
    hasEnforceGenericValueType*: bool

proc inputPin*(name, friendlyName, description: string; dataType: DataType): PinDefinition =
  PinDefinition(
    name: name, friendlyName: friendlyName, description: description,
    pinType: Input, dataType: dataType)

proc outputPin*(name, friendlyName, description: string; dataType: DataType): PinDefinition =
  PinDefinition(
    name: name, friendlyName: friendlyName, description: description,
    pinType: Output, dataType: dataType)

proc withDefault*(pin: PinDefinition; v: string): PinDefinition =
  result = pin
  result.defaultValue = v

proc withValueType*(pin: PinDefinition; v: string): PinDefinition =
  result = pin
  result.valueType = v

proc withSchema*(pin: PinDefinition; v: string): PinDefinition =
  result = pin
  result.schema = v

proc withValidValues*(pin: PinDefinition; values: seq[string]): PinDefinition =
  result = pin
  result.validValues = values

proc withRange*(pin: PinDefinition; min, max: float64): PinDefinition =
  result = pin
  result.range = (min: min, max: max)
  result.hasRange = true

proc withStep*(pin: PinDefinition; step: float64): PinDefinition =
  result = pin
  result.step = step
  result.hasStep = true

proc withSensitive*(pin: PinDefinition; sensitive: bool = true): PinDefinition =
  result = pin
  result.sensitive = sensitive
  result.hasSensitive = true

proc withEnforceSchema*(pin: PinDefinition; enforce: bool = true): PinDefinition =
  result = pin
  result.enforceSchema = enforce
  result.hasEnforceSchema = true

proc withEnforceGenericValueType*(pin: PinDefinition; enforce: bool = true): PinDefinition =
  result = pin
  result.enforceGenericValueType = enforce
  result.hasEnforceGenericValueType = true

proc toJson*(p: PinDefinition): string =
  result = "{\"name\":" & jsonQuote(p.name) &
    ",\"friendly_name\":" & jsonQuote(p.friendlyName) &
    ",\"description\":" & jsonQuote(p.description) &
    ",\"pin_type\":\"" & $p.pinType & "\"" &
    ",\"data_type\":\"" & dataTypeStr(p.dataType) & "\""
  if p.defaultValue.len > 0:
    result.add ",\"default_value\":" & p.defaultValue
  if p.valueType.len > 0:
    result.add ",\"value_type\":" & jsonQuote(p.valueType)
  if p.schema.len > 0:
    result.add ",\"schema\":" & jsonQuote(p.schema)
  if p.validValues.len > 0:
    var vvJson = "["
    for i, v in p.validValues:
      if i > 0: vvJson.add ","
      vvJson.add jsonQuote(v)
    vvJson.add "]"
    result.add ",\"valid_values\":" & vvJson
  if p.hasRange:
    result.add ",\"range\":[" & $p.range.min & "," & $p.range.max & "]"
  if p.hasStep:
    result.add ",\"step\":" & $p.step
  if p.hasSensitive:
    result.add ",\"sensitive\":" & (if p.sensitive: "true" else: "false")
  if p.hasEnforceSchema:
    result.add ",\"enforce_schema\":" & (if p.enforceSchema: "true" else: "false")
  if p.hasEnforceGenericValueType:
    result.add ",\"enforce_generic_value_type\":" & (if p.enforceGenericValueType: "true" else: "false")
  result.add "}"

type
  NodeDefinition* = object
    name*: string
    friendlyName*: string
    description*: string
    category*: string
    icon*: string
    docs*: string
    longRunning*: bool
    abiVersion*: uint32
    pins*: seq[PinDefinition]
    scores*: NodeScores
    hasScores*: bool
    permissions*: seq[string]

proc initNodeDefinition*(): NodeDefinition =
  result.abiVersion = ABI_VERSION

proc addPin*(def: var NodeDefinition; pin: PinDefinition) =
  def.pins.add pin

proc setScores*(def: var NodeDefinition; s: NodeScores) =
  def.scores = s
  def.hasScores = true

proc addPermission*(def: var NodeDefinition; perm: string) =
  def.permissions.add perm

proc toJson*(def: NodeDefinition): string =
  var pinsJson = "["
  for i, pin in def.pins:
    if i > 0: pinsJson.add ","
    pinsJson.add pin.toJson()
  pinsJson.add "]"

  var permsJson = "["
  for i, perm in def.permissions:
    if i > 0: permsJson.add ","
    permsJson.add jsonQuote(perm)
  permsJson.add "]"

  result = "{\"name\":" & jsonQuote(def.name) &
    ",\"friendly_name\":" & jsonQuote(def.friendlyName) &
    ",\"description\":" & jsonQuote(def.description) &
    ",\"category\":" & jsonQuote(def.category) &
    ",\"pins\":" & pinsJson &
    ",\"long_running\":" & (if def.longRunning: "true" else: "false") &
    ",\"abi_version\":" & $def.abiVersion
  if def.icon.len > 0:
    result.add ",\"icon\":" & jsonQuote(def.icon)
  if def.hasScores:
    result.add ",\"scores\":" & def.scores.toJson()
  if def.docs.len > 0:
    result.add ",\"docs\":" & jsonQuote(def.docs)
  if def.permissions.len > 0:
    result.add ",\"permissions\":" & permsJson
  result.add "}"

type
  ExecutionInput* = object
    inputs*: Table[string, string]
    nodeId*: string
    nodeName*: string
    runId*: string
    appId*: string
    boardId*: string
    userId*: string
    streamState*: bool
    logLevel*: uint8

  ExecutionResult* = object
    outputs*: Table[string, string]
    error*: string
    activateExec*: seq[string]
    pending*: bool

proc ok*(): ExecutionResult =
  ExecutionResult()

proc fail*(msg: string): ExecutionResult =
  ExecutionResult(error: msg)

proc setOutput*(r: var ExecutionResult; name, jsonValue: string) =
  r.outputs[name] = jsonValue

proc exec*(r: var ExecutionResult; pin: string) =
  r.activateExec.add pin

proc setPending*(r: var ExecutionResult; p: bool) =
  r.pending = p

proc toJson*(r: ExecutionResult): string =
  var outJson = "{"
  var first = true
  for k, v in r.outputs:
    if not first: outJson.add ","
    outJson.add jsonQuote(k) & ":" & v
    first = false
  outJson.add "}"

  var execJson = "["
  for i, e in r.activateExec:
    if i > 0: execJson.add ","
    execJson.add jsonQuote(e)
  execJson.add "]"

  result = "{\"outputs\":" & outJson &
    ",\"activate_exec\":" & execJson &
    ",\"pending\":" & (if r.pending: "true" else: "false")
  if r.error.len > 0:
    result.add ",\"error\":" & jsonQuote(r.error)
  result.add "}"

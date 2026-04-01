## Local SDK re-export — imports from the adjacent wasm-sdk-nim directory.
## The nim.cfg adds ../wasm-sdk-nim/src to the path so these imports work.

import types, host, context, memory
export types, host, context, memory

# Re-export the higher-level SDK helpers from the SDK's sdk module.
# We can't `import sdk` here because Nim resolves to this file (same name).
# Instead, pull in the parsing/serialization procs directly.
import std/[tables, strutils]

proc isWs(c: char): bool =
  c == ' ' or c == '\t' or c == '\n' or c == '\r'

proc extractJsonString*(json, key: string): string =
  let needle = "\"" & key & "\""
  let pos = json.find(needle)
  if pos < 0: return ""
  var i = pos + needle.len
  while i < json.len and (isWs(json[i]) or json[i] == ':'): inc i
  if i >= json.len or json[i] != '"': return ""
  inc i
  result = ""
  while i < json.len and json[i] != '"':
    if json[i] == '\\' and i + 1 < json.len:
      inc i
      case json[i]
      of '"': result.add '"'
      of '\\': result.add '\\'
      of 'n': result.add '\n'
      of 'r': result.add '\r'
      of 't': result.add '\t'
      else: result.add json[i]
    else:
      result.add json[i]
    inc i

proc extractJsonBool*(json, key: string): bool =
  let needle = "\"" & key & "\""
  let pos = json.find(needle)
  if pos < 0: return false
  var i = pos + needle.len
  while i < json.len and (isWs(json[i]) or json[i] == ':'): inc i
  result = i + 3 < json.len and json[i .. i + 3] == "true"

proc extractJsonInt*(json, key: string): int64 =
  let needle = "\"" & key & "\""
  let pos = json.find(needle)
  if pos < 0: return 0
  var i = pos + needle.len
  while i < json.len and (isWs(json[i]) or json[i] == ':'): inc i
  var neg = false
  if i < json.len and json[i] == '-':
    neg = true
    inc i
  var num: int64 = 0
  while i < json.len and json[i] >= '0' and json[i] <= '9':
    num = num * 10 + int64(ord(json[i]) - ord('0'))
    inc i
  if neg: -num else: num

proc parseInputsObject*(json: string): Table[string, string] =
  result = initTable[string, string]()
  let inputsPos = json.find("\"inputs\"")
  if inputsPos < 0: return

  var objStart = json.find('{', inputsPos + 8)
  if objStart < 0: return

  var depth = 1
  var objEnd = objStart + 1
  while depth > 0 and objEnd < json.len:
    if json[objEnd] == '{': inc depth
    elif json[objEnd] == '}': dec depth
    inc objEnd

  let sub = json[objStart ..< objEnd]
  var i = 1

  while i < sub.len - 1:
    while i < sub.len and isWs(sub[i]): inc i
    if i >= sub.len - 1 or sub[i] == '}': break
    if sub[i] != '"':
      inc i
      continue

    inc i
    let ks = i
    while i < sub.len and sub[i] != '"': inc i
    let k = sub[ks ..< i]
    inc i

    while i < sub.len and (isWs(sub[i]) or sub[i] == ':'): inc i

    let vs = i
    if i < sub.len and sub[i] == '"':
      inc i
      while i < sub.len:
        if sub[i] == '"' and sub[i - 1] != '\\': break
        inc i
      inc i
    elif i < sub.len and sub[i] == '{':
      var d = 1
      inc i
      while d > 0 and i < sub.len:
        if sub[i] == '{': inc d
        elif sub[i] == '}': dec d
        inc i
    elif i < sub.len and sub[i] == '[':
      var d = 1
      inc i
      while d > 0 and i < sub.len:
        if sub[i] == '[': inc d
        elif sub[i] == ']': dec d
        inc i
    else:
      while i < sub.len and not isWs(sub[i]) and sub[i] != ',' and sub[i] != '}': inc i

    result[k] = sub[vs ..< i]
    while i < sub.len and (isWs(sub[i]) or sub[i] == ','): inc i

# =============================================================================
# Public API
# =============================================================================

proc jsonString*(s: string): string =
  jsonQuote(s)

proc parseInput*(raw: string): ExecutionInput =
  var inp = ExecutionInput()
  inp.nodeId = extractJsonString(raw, "node_id")
  inp.nodeName = extractJsonString(raw, "node_name")
  inp.runId = extractJsonString(raw, "run_id")
  inp.appId = extractJsonString(raw, "app_id")
  inp.boardId = extractJsonString(raw, "board_id")
  inp.userId = extractJsonString(raw, "user_id")
  inp.streamState = extractJsonBool(raw, "stream_state")
  inp.logLevel = uint8(extractJsonInt(raw, "log_level"))
  inp.inputs = parseInputsObject(raw)
  inp

proc serializeDefinition*(def: NodeDefinition): int64 =
  packResult(def.toJson())

proc serializeResult*(res: ExecutionResult): int64 =
  packResult(res.toJson())

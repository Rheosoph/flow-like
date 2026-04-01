import std/[strutils, tables]
import types, host

type
  Context* = object
    input*: ExecutionInput
    result*: ExecutionResult

proc newContext*(input: ExecutionInput): Context =
  Context(input: input, result: ok())

# -- Input getters --

proc getRaw*(ctx: Context; name: string): string =
  if ctx.input.inputs.hasKey(name):
    ctx.input.inputs[name]
  else:
    ""

proc getString*(ctx: Context; name: string; default: string = ""): string =
  if not ctx.input.inputs.hasKey(name): return default
  let v = ctx.input.inputs[name]
  if v.len >= 2 and v[0] == '"' and v[^1] == '"':
    v[1 ..< v.len - 1]
  else:
    v

proc getI64*(ctx: Context; name: string; default: int64 = 0): int64 =
  if not ctx.input.inputs.hasKey(name): return default
  try:
    parseBiggestInt(ctx.input.inputs[name])
  except ValueError:
    default

proc getF64*(ctx: Context; name: string; default: float64 = 0.0): float64 =
  if not ctx.input.inputs.hasKey(name): return default
  try:
    parseFloat(ctx.input.inputs[name])
  except ValueError:
    default

proc getBool*(ctx: Context; name: string; default: bool = false): bool =
  if not ctx.input.inputs.hasKey(name): return default
  ctx.input.inputs[name] == "true"

# -- Metadata shortcuts --

proc nodeId*(ctx: Context): string = ctx.input.nodeId
proc nodeName*(ctx: Context): string = ctx.input.nodeName
proc runId*(ctx: Context): string = ctx.input.runId
proc appId*(ctx: Context): string = ctx.input.appId
proc boardId*(ctx: Context): string = ctx.input.boardId
proc userId*(ctx: Context): string = ctx.input.userId
proc streamEnabled*(ctx: Context): bool = ctx.input.streamState
proc getLogLevel*(ctx: Context): uint8 = ctx.input.logLevel

# -- Output setters --

proc setOutput*(ctx: var Context; name, jsonValue: string) =
  ctx.result.outputs[name] = jsonValue

proc activateExec*(ctx: var Context; pin: string) =
  ctx.result.activateExec.add pin

proc setPending*(ctx: var Context; p: bool) =
  ctx.result.pending = p

proc setError*(ctx: var Context; msg: string) =
  ctx.result.error = msg

# -- Level-gated logging --

proc trace*(ctx: Context; msg: string) =
  if ctx.input.logLevel <= LogLevelTrace: logTrace(msg)

proc debug*(ctx: Context; msg: string) =
  if ctx.input.logLevel <= LogLevelDebug: logDebug(msg)

proc info*(ctx: Context; msg: string) =
  if ctx.input.logLevel <= LogLevelInfo: logInfo(msg)

proc warn*(ctx: Context; msg: string) =
  if ctx.input.logLevel <= LogLevelWarn: logWarn(msg)

proc error*(ctx: Context; msg: string) =
  if ctx.input.logLevel <= LogLevelError: logError(msg)

# -- Conditional streaming --

proc streamText*(ctx: Context; text: string) =
  if ctx.input.streamState: host.streamText(text)

proc streamJson*(ctx: Context; json: string) =
  if ctx.input.streamState: streamEmit("json", json)

proc streamProgress*(ctx: Context; pct: float; message: string) =
  if ctx.input.streamState:
    let data = "{\"progress\":" & $pct & ",\"message\":" & jsonQuote(message) & "}"
    streamEmit("progress", data)

# -- Variables --

proc getVariable*(ctx: Context; name: string): string =
  varGet(name)

proc setVariable*(ctx: Context; name, value: string) =
  varSet(name, value)

proc deleteVariable*(ctx: Context; name: string) =
  varDelete(name)

proc hasVariable*(ctx: Context; name: string): bool =
  varHas(name)

# -- Storage --

proc readStorage*(ctx: Context; path: string): string =
  storageRead(path)

proc writeStorage*(ctx: Context; path, data: string): int32 =
  storageWrite(path, data)

proc getStorageDir*(ctx: Context; nodeScoped: bool = false): string =
  storageDir(nodeScoped)

proc getUploadDir*(ctx: Context): string =
  uploadDir()

proc getCacheDir*(ctx: Context; nodeScoped: bool = false; userScoped: bool = false): string =
  cacheDir(nodeScoped, userScoped)

proc getUserDir*(ctx: Context; nodeScoped: bool = false): string =
  userDir(nodeScoped)

proc listStorage*(ctx: Context; path: string): string =
  storageList(path)

# -- Models --

proc embedText*(ctx: Context; bit, texts: string): string =
  host.embedText(bit, texts)

# -- HTTP --

proc httpRequest*(ctx: Context; meth: int32; url, headers, body: string): int32 =
  host.httpRequest(meth, url, headers, body)

# -- Finalization --

proc finish*(ctx: var Context): ExecutionResult =
  result = ctx.result

proc success*(ctx: var Context): ExecutionResult =
  ctx.activateExec("exec_out")
  ctx.finish()

proc fail*(ctx: var Context; msg: string): ExecutionResult =
  ctx.setError(msg)
  ctx.finish()

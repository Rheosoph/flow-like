## HTTP Request Node — Demonstrates declaring HTTP permissions (Nim)
##
## This example shows how to declare the "http" permission and use the
## host import to make outbound HTTP requests from a Nim WASM node.
## Copy this pattern into your node.nim when you need network access.

import sdk

# ============================================================================
# Node definition — note `addPermission("http")`
# ============================================================================

proc buildHttpGetDefinition(): NodeDefinition =
  var def = initNodeDefinition()
  def.name = "http_get_request_nim"
  def.friendlyName = "HTTP GET Request (Nim)"
  def.description = "Sends a GET request to a URL and reports the result"
  def.category = "Network/HTTP"
  def.addPermission("http")

  def.addPin inputPin("exec", "Execute", "Trigger execution", Exec)
  def.addPin inputPin("url", "URL", "Target URL", String).withDefault("\"https://httpbin.org/get\"")
  def.addPin inputPin("headers_json", "Headers (JSON)", "Request headers as JSON", String).withDefault("\"{}\"")

  def.addPin outputPin("exec_out", "Done", "Fires after the request", Exec)
  def.addPin outputPin("success", "Success", "Whether the HTTP call was accepted", Bool)

  def

# ============================================================================
# Run handler — uses httpRequest host import directly
# ============================================================================

proc handleHttpGet(ctx: var Context): ExecutionResult =
  let url = ctx.getString("url", "https://httpbin.org/get")
  let headers = ctx.getString("headers_json", "{}")

  logInfo("Sending GET request to " & url)

  # Method 0 = GET. The host checks the "http" capability.
  let resultCode = httpRequest(0, url, headers, "")
  let ok = resultCode != -1

  if ok:
    logInfo("HTTP capability granted — request dispatched")
  else:
    logError("HTTP capability denied — is the 'http' permission declared?")

  ctx.setOutput("success", if ok: "true" else: "false")
  ctx.success()

# ============================================================================
# WASM Exports
# ============================================================================

proc get_node(): int64 {.exportc.} =
  serializeDefinition(buildHttpGetDefinition())

proc get_nodes(): int64 {.exportc.} =
  let def = buildHttpGetDefinition()
  packResult("[" & def.toJson() & "]")

proc run(p: uint32; l: uint32): int64 {.exportc.} =
  var raw = newString(l)
  if l > 0:
    copyMem(addr raw[0], cast[pointer](p), l)
  let input = parseInput(raw)
  var ctx = newContext(input)
  let res = handleHttpGet(ctx)
  serializeResult(res)

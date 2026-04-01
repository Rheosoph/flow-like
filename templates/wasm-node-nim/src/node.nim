## Flow-Like WASM Node Template (Nim)
##
## A template for creating custom nodes in Nim that compile to WebAssembly
## via Nim → C → Emscripten.
##
## Building:
##   nimble build
##
## The compiled .wasm file will be at: build/node.wasm

import sdk

# ============================================================================
# Node Definition
# ============================================================================

proc buildDefinition(): NodeDefinition =
  var def = initNodeDefinition()
  def.name = "my_custom_node_nim"
  def.friendlyName = "My Custom Node (Nim)"
  def.description = "A template WASM node built with Nim"
  def.category = "Custom/WASM"
  def.addPermission("streaming")

  # Input pins
  def.addPin inputPin("exec", "Execute", "Trigger execution", Exec)
  def.addPin inputPin("input_text", "Input Text", "Text to process", String).withDefault("\"\"")
  def.addPin inputPin("multiplier", "Multiplier", "Number of times to repeat", I64).withDefault("1")

  # Output pins
  def.addPin outputPin("exec_out", "Done", "Execution complete", Exec)
  def.addPin outputPin("output_text", "Output Text", "Processed text", String)
  def.addPin outputPin("char_count", "Character Count", "Number of characters in output", I64)

  def

# ============================================================================
# Node Execution
# ============================================================================

proc handleRun(ctx: var Context): ExecutionResult =
  let inputText = ctx.getString("input_text")
  let multiplier = ctx.getI64("multiplier", 1)

  ctx.debug("Processing: '" & inputText & "' x " & $multiplier)

  var output = ""
  for i in 0 ..< multiplier:
    output.add inputText

  let charCount = output.len

  ctx.streamText("Generated " & $charCount & " characters")

  ctx.setOutput("output_text", jsonString(output))
  ctx.setOutput("char_count", $charCount)

  ctx.success()

# ============================================================================
# WASM Exports
# ============================================================================

proc get_node(): int64 {.exportc.} =
  serializeDefinition(buildDefinition())

proc get_nodes(): int64 {.exportc.} =
  let def = buildDefinition()
  packResult("[" & def.toJson() & "]")

proc run(p: uint32; l: uint32): int64 {.exportc.} =
  var raw = newString(l)
  if l > 0:
    copyMem(addr raw[0], cast[pointer](p), l)
  let input = parseInput(raw)
  var ctx = newContext(input)
  let res = handleRun(ctx)
  serializeResult(res)

package node

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import sdk.*

// ============================================================================
// Node Definition
// ============================================================================

@WasmExport
fun get_node(): Long {
    val def = NodeDefinition(
        name = "my_custom_node_kt",
        friendlyName = "My Custom Node (Kotlin)",
        description = "A template WASM node built with Kotlin",
        category = "Custom/WASM",
    )
    def.addPermission("streaming")

    // Input pins
    def.addPin(PinDefinition.input("exec", "Execute", "Trigger execution", DataType.EXEC))
    def.addPin(PinDefinition.input("input_text", "Input Text", "Text to process", DataType.STRING).withDefault(JsonPrimitive("")))
    def.addPin(PinDefinition.input("multiplier", "Multiplier", "Number of times to repeat", DataType.I64).withDefault(JsonPrimitive(1)))

    // Output pins
    def.addPin(PinDefinition.output("exec_out", "Done", "Execution complete", DataType.EXEC))
    def.addPin(PinDefinition.output("output_text", "Output Text", "Processed text", DataType.STRING))
    def.addPin(PinDefinition.output("char_count", "Character Count", "Number of characters in output", DataType.I64))

    val json = Json.encodeToString(NodeDefinition.serializer(), def)
    return packResult(json)
}

@WasmExport
fun get_nodes(): Long {
    val def = NodeDefinition(
        name = "my_custom_node_kt",
        friendlyName = "My Custom Node (Kotlin)",
        description = "A template WASM node built with Kotlin",
        category = "Custom/WASM",
    )
    def.addPermission("streaming")

    def.addPin(PinDefinition.input("exec", "Execute", "Trigger execution", DataType.EXEC))
    def.addPin(PinDefinition.input("input_text", "Input Text", "Text to process", DataType.STRING).withDefault(JsonPrimitive("")))
    def.addPin(PinDefinition.input("multiplier", "Multiplier", "Number of times to repeat", DataType.I64).withDefault(JsonPrimitive(1)))

    def.addPin(PinDefinition.output("exec_out", "Done", "Execution complete", DataType.EXEC))
    def.addPin(PinDefinition.output("output_text", "Output Text", "Processed text", DataType.STRING))
    def.addPin(PinDefinition.output("char_count", "Character Count", "Number of characters in output", DataType.I64))

    val json = Json.encodeToString(NodeDefinition.serializer(), def)
    return packResult("[$json]")
}

// ============================================================================
// Node Execution
// ============================================================================

@WasmExport
fun run(ptr: Int, len: Int): Long {
    val inputJson = ptrToString(ptr, len)
    val input = Json.decodeFromString(ExecutionInput.serializer(), inputJson)
    val ctx = Context(input)

    val inputText = ctx.getString("input_text")
    val multiplier = ctx.getI64("multiplier", 1L)

    ctx.debug("Processing: '$inputText' x $multiplier")

    val outputText = inputText.repeat(maxOf(multiplier.toInt(), 0))
    val charCount = outputText.length.toLong()

    ctx.streamText("Generated $charCount characters")

    ctx.setOutput("output_text", outputText)
    ctx.setOutput("char_count", charCount)

    val result = ctx.success()
    val resultJson = Json.encodeToString(ExecutionResult.serializer(), result)
    return packResult(resultJson)
}

// ============================================================================
// ABI Version
// ============================================================================

@WasmExport
fun get_abi_version(): Int = ABI_VERSION

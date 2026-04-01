package sdk

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

const val ABI_VERSION: Int = 1

object PinType {
    const val INPUT = "Input"
    const val OUTPUT = "Output"
}

object DataType {
    const val EXEC = "Exec"
    const val STRING = "String"
    const val I64 = "I64"
    const val F64 = "F64"
    const val BOOL = "Bool"
    const val GENERIC = "Generic"
    const val BYTES = "Bytes"
    const val DATE = "Date"
    const val PATH_BUF = "PathBuf"
    const val STRUCT = "Struct"
}

object ValueType {
    const val NORMAL = "Normal"
    const val ARRAY = "Array"
    const val HASH_MAP = "HashMap"
    const val HASH_SET = "HashSet"
}

object LogLevel {
    const val DEBUG: Int = 0
    const val INFO: Int = 1
    const val WARN: Int = 2
    const val ERROR: Int = 3
    const val FATAL: Int = 4
}

@Serializable
data class NodeScores(
    val privacy: Int = 0,
    val security: Int = 0,
    val performance: Int = 0,
    val governance: Int = 0,
    val reliability: Int = 0,
    val cost: Int = 0,
)

@Serializable
data class PinDefinition(
    val name: String,
    @SerialName("friendly_name") val friendlyName: String,
    val description: String,
    @SerialName("pin_type") val pinType: String,
    @SerialName("data_type") val dataType: String,
    @SerialName("default_value") val defaultValue: JsonElement? = null,
    @SerialName("value_type") val valueType: String? = null,
    val schema: String? = null,
    val step: Double? = null,
    val sensitive: Boolean? = null,
    @SerialName("enforce_schema") val enforceSchema: Boolean? = null,
    @SerialName("enforce_generic_value_type") val enforceGenericValueType: Boolean? = null,
) {
    companion object {
        fun input(name: String, friendlyName: String, description: String, dataType: String): PinDefinition =
            PinDefinition(name, friendlyName, description, PinType.INPUT, dataType)

        fun output(name: String, friendlyName: String, description: String, dataType: String): PinDefinition =
            PinDefinition(name, friendlyName, description, PinType.OUTPUT, dataType)
    }

    fun withDefault(value: JsonElement): PinDefinition = copy(defaultValue = value)

    /** Set the value type (e.g. "Array", "HashMap", "HashSet"). */
    fun withValueType(valueType: String): PinDefinition = copy(valueType = valueType)

    /** Attach a raw JSON Schema string to this pin. */
    fun withSchema(schema: String): PinDefinition = copy(schema = schema)

    fun withStep(step: Double): PinDefinition = copy(step = step)

    fun withSensitive(sensitive: Boolean = true): PinDefinition = copy(sensitive = sensitive)

    fun withEnforceSchema(enforce: Boolean = true): PinDefinition = copy(enforceSchema = enforce)

    fun withEnforceGenericValueType(enforce: Boolean = true): PinDefinition = copy(enforceGenericValueType = enforce)
}

@Serializable
data class NodeDefinition(
    val name: String,
    @SerialName("friendly_name") val friendlyName: String,
    val description: String,
    val category: String,
    val icon: String? = null,
    val pins: MutableList<PinDefinition> = mutableListOf(),
    val scores: NodeScores? = null,
    @SerialName("long_running") val longRunning: Boolean = false,
    val docs: String? = null,
    val permissions: MutableList<String> = mutableListOf(),
    @SerialName("abi_version") val abiVersion: Int = ABI_VERSION,
) {
    fun addPin(pin: PinDefinition): NodeDefinition {
        pins.add(pin)
        return this
    }

    fun addPermission(permission: String): NodeDefinition {
        permissions.add(permission)
        return this
    }
}

@Serializable
data class ExecutionInput(
    val inputs: Map<String, JsonElement> = emptyMap(),
    @SerialName("node_id") val nodeId: String = "",
    @SerialName("node_name") val nodeName: String = "",
    @SerialName("run_id") val runId: String = "",
    @SerialName("app_id") val appId: String = "",
    @SerialName("board_id") val boardId: String = "",
    @SerialName("user_id") val userId: String = "",
    @SerialName("stream_state") val streamState: Boolean = false,
    @SerialName("log_level") val logLevel: Int = LogLevel.INFO,
)

@Serializable
data class ExecutionResult(
    val outputs: MutableMap<String, JsonElement> = mutableMapOf(),
    var error: String? = null,
    @SerialName("activate_exec") val activateExec: MutableList<String> = mutableListOf(),
    var pending: Boolean? = null,
) {
    companion object {
        fun success(): ExecutionResult = ExecutionResult()

        fun fail(message: String): ExecutionResult = ExecutionResult(error = message)
    }

    fun setOutput(name: String, value: JsonElement): ExecutionResult {
        outputs[name] = value
        return this
    }

    fun activateExec(pinName: String): ExecutionResult {
        activateExec.add(pinName)
        return this
    }

    fun setPending(value: Boolean): ExecutionResult {
        pending = value
        return this
    }
}

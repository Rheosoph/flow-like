package sdk

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.double
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.long
import kotlinx.serialization.json.longOrNull

class Context(private val input: ExecutionInput) {
    private val result = ExecutionResult.success()
    private val outputs = mutableMapOf<String, String>()

    // -- Metadata --

    val nodeId: String get() = input.nodeId
    val nodeName: String get() = input.nodeName
    val runId: String get() = input.runId
    val appId: String get() = input.appId
    val boardId: String get() = input.boardId
    val userId: String get() = input.userId
    val streamEnabled: Boolean get() = input.streamState
    val logLevelValue: Int get() = input.logLevel

    // -- Input getters --

    fun getString(name: String, default: String = ""): String {
        val element = input.inputs[name] ?: return default
        if (element is JsonPrimitive) {
            return element.contentOrNull ?: default
        }
        return default
    }

    fun getI64(name: String, default: Long = 0L): Long {
        val element = input.inputs[name] ?: return default
        if (element is JsonPrimitive) {
            return element.longOrNull ?: default
        }
        return default
    }

    fun getF64(name: String, default: Double = 0.0): Double {
        val element = input.inputs[name] ?: return default
        if (element is JsonPrimitive) {
            return element.doubleOrNull ?: default
        }
        return default
    }

    fun getBool(name: String, default: Boolean = false): Boolean {
        val element = input.inputs[name] ?: return default
        if (element is JsonPrimitive) {
            return element.booleanOrNull ?: default
        }
        return default
    }

    fun getRawInput(name: String): String? {
        val element = input.inputs[name] ?: return null
        return Json.encodeToString(kotlinx.serialization.json.JsonElement.serializer(), element)
    }

    // -- Output setters --

    fun setOutput(name: String, value: String) {
        outputs[name] = Json.encodeToString(kotlinx.serialization.json.JsonElement.serializer(), JsonPrimitive(value))
    }

    fun setOutput(name: String, value: Long) {
        outputs[name] = value.toString()
    }

    fun setOutput(name: String, value: Double) {
        outputs[name] = value.toString()
    }

    fun setOutput(name: String, value: Boolean) {
        outputs[name] = value.toString()
    }

    fun setOutputJson(name: String, json: String) {
        outputs[name] = json
    }

    fun activateExec(pinName: String) {
        result.activateExec.add(pinName)
    }

    fun setPending(pending: Boolean) {
        result.pending = pending
    }

    fun setError(error: String) {
        result.error = error
    }

    // -- Level-gated logging --

    private fun shouldLog(level: Int): Boolean = level >= input.logLevel

    fun debug(message: String) {
        if (shouldLog(LogLevel.DEBUG)) logDebug(message)
    }

    fun info(message: String) {
        if (shouldLog(LogLevel.INFO)) logInfo(message)
    }

    fun warn(message: String) {
        if (shouldLog(LogLevel.WARN)) logWarn(message)
    }

    fun error(message: String) {
        if (shouldLog(LogLevel.ERROR)) logError(message)
    }

    // -- Conditional streaming --

    fun streamText(text: String) {
        if (streamEnabled) sdk.streamText(text)
    }

    fun streamJson(data: String) {
        if (streamEnabled) stream("json", data)
    }

    fun streamProgress(progress: Float, message: String) {
        if (streamEnabled) stream("progress", """{"progress":$progress,"message":"$message"}""")
    }

    // -- Variables --

    fun getVariable(name: String): String? = sdk.getVariable(name)

    fun setVariable(name: String, value: String) = sdk.setVariable(name, value)

    // -- Finalize --

    fun finish(): ExecutionResult {
        for ((name, value) in outputs) {
            val element = Json.decodeFromString(kotlinx.serialization.json.JsonElement.serializer(), value)
            result.outputs[name] = element
            hostSetOutput(name, value)
        }
        return result
    }

    fun success(): ExecutionResult {
        activateExec("exec_out")
        return finish()
    }

    fun fail(error: String): ExecutionResult {
        setError(error)
        return finish()
    }

    private fun hostSetOutput(name: String, value: String) {
        sdk.setOutput(name, value)
    }
}

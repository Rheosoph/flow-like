package com.flow_like.app

import org.json.JSONArray
import org.json.JSONObject

internal object BotAutomationSignals {
    @Volatile var cancelled: ((String, String) -> Unit)? = null
}

/** Native selection limits references; the authenticated app rechecks access before execution. */
internal object BotAutomationPolicy {
    private val identifier = Regex("[A-Za-z0-9_-]{1,128}")
    private fun entries(snapshot: JSONObject, key: String): List<JSONObject> {
        val values = snapshot.optJSONArray(key) ?: JSONArray()
        return (0 until values.length()).map { values.getJSONObject(it) }
    }
    fun validate(snapshot: JSONObject, action: JSONObject, responseMode: String) {
        require(responseMode in listOf("text", "result"))
        require(!action.optBoolean("voice") && (action.optJSONArray("files")?.length() ?: 0) == 0)
        val text = action.optString("prompt").ifBlank { action.optString("text") }
        require(text.toByteArray(Charsets.UTF_8).size <= 60000)
        when (action.optString("kind")) {
            "ask_bot", "run_bot_task" -> {
                require(responseMode == "text") { "Bot answers return text" }
                val id = action.optString("botId")
                require(identifier.matches(id) && entries(snapshot,"bots").any { it.optString("id") == id && it.optBoolean("enabled",true) }) { "The bot is unavailable" }
                require(!action.has("conversationId")) { "Select the bot's own conversation" }
                if (action.optString("kind") == "run_bot_task") {
                    val task = action.optString("taskId")
                    require(identifier.matches(task) && entries(snapshot,"botTasks").any { it.optString("id") == task && it.optString("botId") == id && it.optBoolean("enabled") }) { "The task is unavailable or paused" }
                } else require(text.isNotBlank()) { "Enter a question" }
            }
            "flowpilot" -> { require(responseMode == "text" && text.isNotBlank()) { "Enter a question" } }
            "run_event", "open_event" -> {
                val app = action.optString("appId")
                val event = action.optString("eventId")
                require(identifier.matches(app) && identifier.matches(event))
                val exposed = entries(snapshot,"events").firstOrNull { it.optString("appId") == app && it.optString("eventId") == event }
                    ?: error("The Event is unavailable")
                val surfaces = exposed.optJSONArray("surfaces") ?: JSONArray()
                require((0 until surfaces.length()).any { surfaces.optString(it) in listOf("siri","shortcuts") }) { "The Event is not exposed to automation" }
                val expected = exposed.optJSONObject("action")?.optString("operation").orEmpty()
                require(action.optString("operation") == expected) { "The Event operation changed" }
                if (exposed.optString("eventType") == "simple_chat") require(text.isNotBlank() && responseMode == "text") { "Enter a chat question" }
            }
            else -> error("This action cannot return an automation result")
        }
    }
}

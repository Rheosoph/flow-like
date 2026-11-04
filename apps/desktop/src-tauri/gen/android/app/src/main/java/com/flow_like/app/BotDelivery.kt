package com.flow_like.app

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject
import java.net.URI
import java.util.UUID
import javax.net.ssl.HttpsURLConnection

/** Only explicit plain-chat notification actions can be sent without the foreground runtime. */
internal object BotDeliveryPolicy {
    data class Request(val url: String, val body: JSONObject)
    private val identifier = Regex("[A-Za-z0-9_-]{1,128}")

    fun apiBase(scopeValue: String): String? {
        val scope = JSONArray(scopeValue)
        if (scope.length() != 3 || scope.optString(1).isBlank() || scope.optString(2) in listOf("", "local")) return null
        val origin = URI(scope.getString(0))
        if (origin.scheme != "https" || origin.host.isNullOrBlank() || origin.rawUserInfo != null ||
            origin.rawQuery != null || origin.rawFragment != null || origin.rawPath.contains('%') || origin.rawPath.contains('\\')) return null
        return origin.toASCIIString().trimEnd('/').removeSuffix("/api/v1") + "/api/v1"
    }

    fun request(item: JSONObject, snapshot: JSONObject): Request? {
        if (!item.optBoolean("backgroundDelivery") || item.optString("scope") != snapshot.optString("scope") ||
            item.has("responseMode") && !item.isNull("responseMode")) return null
        val base = apiBase(item.getString("scope")) ?: return null
        val action = item.getJSONObject("action")
        val botId = action.optString("botId")
        val bots = snapshot.optJSONArray("bots") ?: return null
        val bot = (0 until bots.length()).map { bots.getJSONObject(it) }.firstOrNull { it.optString("id") == botId } ?: return null
        if (bot.optBoolean("local") || bot.optBoolean("encrypted") || !identifier.matches(botId) || action.optBoolean("voice") ||
            (action.optJSONArray("files")?.length() ?: 0) != 0) return null
        UUID.fromString(item.getString("id"))
        return when (action.optString("kind")) {
            "ask_bot" -> {
                val conversation = action.optString("conversationId")
                val text = action.optString("prompt").ifBlank { action.optString("text") }
                if (!identifier.matches(conversation) || text.isBlank() || text.toByteArray().size > 60000 ||
                    (action.optJSONArray("files")?.length() ?: 0) != 0) return null
                Request("$base/inbox/conversations/$conversation/entries", JSONObject()
                    .put("message_id", item.getString("id")).put("kind", "message")
                    .put("envelope", JSONObject().put("v", 1).put("enc", "none")
                        .put("body", JSONObject().put("text", text).put("mentions", JSONArray().put(botId)))))
            }
            "respond_bot_approval" -> {
                val turn = action.optString("turnId")
                val approval = action.optString("approvalId")
                val decision = action.optString("decision")
                if (!identifier.matches(turn) || !identifier.matches(approval) || decision !in listOf("once", "deny")) return null
                Request("$base/bots/$botId/turns/$turn/approval", JSONObject().put("approval_id", approval).put("decision", decision))
            }
            else -> null
        }
    }

    fun freshBot(value: JSONObject, botId: String, subject: String): Boolean =
        value.optString("id") == botId && value.optString("owner_id") == subject &&
            value.optString("location") in listOf("server", "desktop", "device") &&
            value.optJSONArray("capabilities")?.let { caps -> (0 until caps.length()).none { caps.optString(it) == "encryption" } } == true

    fun plainConversation(value: JSONObject, conversationId: String, botId: String, subject: String): Boolean {
        val members = value.optJSONArray("members") ?: return false
        return value.optString("id") == conversationId && value.optString("encryption") == "plain" &&
            (0 until members.length()).any { members.getJSONObject(it).let { m -> m.optString("kind") == "bot" && m.optString("principal_id") == botId } } &&
            (0 until members.length()).any { members.getJSONObject(it).let { m -> m.optString("kind") == "user" && m.optString("principal_id") == subject } }
    }

    fun accessToken(scope: String, records: BotRecordStore, nowSeconds: Long = System.currentTimeMillis() / 1000): String? {
        val parts = JSONArray(scope)
        if (parts.length() != 3) return null
        val binding = JSONArray().put(parts.getString(1)).put(parts.getString(0).trimEnd('/')).toString()
        val key = records.read("credential:native-auth-binding:$binding")?.toString(Charsets.UTF_8) ?: return null
        if (!key.matches(Regex("native-auth:v1:[a-f0-9]{64}"))) return null
        val session = records.read("credential:$key")?.let { JSONObject(it.toString(Charsets.UTF_8)) } ?: return null
        val user = session.optJSONObject("user") ?: return null
        if (user.optJSONObject("profile")?.optString("sub") != parts.getString(2) || user.optLong("expires_at") <= nowSeconds + 30) return null
        if (!user.optString("token_type", "Bearer").equals("Bearer", ignoreCase = true)) return null
        return user.optString("access_token").takeIf { it.isNotBlank() && it.length <= 65536 && it.all { char -> char.code in 33..126 } }
    }
}

internal class BotDelivery(private val context: Context) {
    enum class Outcome { COMPLETE, RETRY, FOREGROUND }

    fun deliver(item: JSONObject): Outcome {
        val store = BotNativeStore(context)
        val scope = item.getString("scope")
        val id = item.getString("id")
        val snapshot = store.snapshot() ?: return Outcome.FOREGROUND
        if (snapshot.optString("scope") != scope) return Outcome.COMPLETE
        if (item.has("responseDeadline") && BotNativeStore.expired(item.getString("responseDeadline"))) {
            store.deliveryFailed(id, scope, "expired")
            return Outcome.COMPLETE
        }
        val request = BotDeliveryPolicy.request(item, snapshot) ?: return Outcome.FOREGROUND
        if (store.snapshot()?.optString("scope") != scope) return Outcome.COMPLETE
        val http = BotHttp(context, scope)
        val action = item.getJSONObject("action")
        val botId = action.getString("botId")
        val bot = http.request("GET", "bots/$botId")
        if (bot.status != 200) return failure(store, id, scope, bot.status)
        if (bot.body == null || !BotDeliveryPolicy.freshBot(bot.body, botId, BotNativeStore.subject(scope) ?: ""))
            return failure(store, id, scope, 409)
        if (action.optString("kind") == "ask_bot") {
            val conversationId = action.getString("conversationId")
            val conversation = http.request("GET", "inbox/conversations/$conversationId")
            if (conversation.status != 200) return failure(store, id, scope, conversation.status)
            if (conversation.body == null || !BotDeliveryPolicy.plainConversation(conversation.body, conversationId, botId, BotNativeStore.subject(scope) ?: ""))
                return failure(store, id, scope, 409)
        }
        val base = BotDeliveryPolicy.apiBase(scope) ?: return Outcome.FOREGROUND
        val response = http.request("POST", request.url.removePrefix("$base/"), request.body)
        return when (response.status) {
                in 200..299 -> { store.acknowledge(id, scope); Outcome.COMPLETE }
                else -> failure(store, id, scope, response.status)
        }
    }

    private fun failure(store: BotNativeStore, id: String, scope: String, status: Int): Outcome = when (status) {
                401 -> Outcome.RETRY
                408, 429, in 500..599 -> Outcome.RETRY
                else -> {
                    store.deliveryFailed(id, scope, "http_$status")
                    BotSurfaces.deliveryFailure(context)
                    Outcome.COMPLETE
                }
    }
}

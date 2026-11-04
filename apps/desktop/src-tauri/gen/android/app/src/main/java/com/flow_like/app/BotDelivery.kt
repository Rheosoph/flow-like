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
        return origin.toASCIIString().trimEnd('/') + "/api/v1"
    }

    fun request(item: JSONObject, snapshot: JSONObject): Request? {
        if (!item.optBoolean("backgroundDelivery") || item.optString("scope") != snapshot.optString("scope")) return null
        val base = apiBase(item.getString("scope")) ?: return null
        val action = item.getJSONObject("action")
        val botId = action.optString("botId")
        val bots = snapshot.optJSONArray("bots") ?: return null
        val bot = (0 until bots.length()).map { bots.getJSONObject(it) }.firstOrNull { it.optString("id") == botId } ?: return null
        if (bot.optBoolean("local") || bot.optBoolean("encrypted") || !identifier.matches(botId)) return null
        UUID.fromString(item.getString("id"))
        return when (action.optString("kind")) {
            "ask_bot" -> {
                val conversation = action.optString("conversationId")
                val text = action.optString("text")
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
        val token = BotDeliveryPolicy.accessToken(scope, BotSecureStore(context)) ?: return Outcome.FOREGROUND
        if (store.snapshot()?.optString("scope") != scope) return Outcome.COMPLETE
        val connection = URI(request.url).toURL().openConnection() as HttpsURLConnection
        try {
            connection.instanceFollowRedirects = false
            connection.requestMethod = "POST"
            connection.connectTimeout = 15000
            connection.readTimeout = 15000
            connection.setRequestProperty("Authorization", "Bearer $token")
            connection.setRequestProperty("Content-Type", "application/json")
            connection.setRequestProperty("Accept", "application/json")
            val bytes = request.body.toString().toByteArray(Charsets.UTF_8)
            connection.doOutput = true
            connection.setFixedLengthStreamingMode(bytes.size)
            connection.outputStream.use { it.write(bytes) }
            return when (val status = connection.responseCode) {
                in 200..299 -> { store.acknowledge(id, scope); Outcome.COMPLETE }
                401 -> Outcome.FOREGROUND
                408, 429, in 500..599 -> Outcome.RETRY
                else -> {
                    store.deliveryFailed(id, scope, "http_$status")
                    BotSurfaces.deliveryFailure(context)
                    Outcome.COMPLETE
                }
            }
        } finally { connection.disconnect() }
    }
}

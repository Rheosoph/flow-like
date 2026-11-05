package com.flow_like.app

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.time.Instant
import java.util.UUID

/** Actions remain scoped to their account until the authenticated runtime acknowledges them. */
internal class BotNativeStore(private val directory: File, private val secure: BotRecordStore) {
    constructor(context: Context) : this(sharedDirectory(context), BotSecureStore(context))

    companion object {
        private val lock = Any()
        private const val MAX_ACTIONS = 128
        private const val MAX_ACTION_AGE_MS = 24 * 60 * 60 * 1000L
        fun sharedDirectory(context: Context): File = File(context.noBackupFilesDir, "bot-shares").apply { mkdirs() }
        fun expired(value: String): Boolean = try { Instant.parse(value).toEpochMilli() <= System.currentTimeMillis() } catch (_: Exception) { true }
        fun subject(scope: String): String? = try {
            val parts = JSONArray(scope)
            if (parts.length() == 3) parts.optString(2).takeIf { it.isNotBlank() } else null
        } catch (_: Exception) { null }
    }

    private fun json(key: String): JSONObject? = secure.read(key)?.let { JSONObject(String(it, Charsets.UTF_8)) }
    private fun save(key: String, value: JSONObject) = secure.write(key, value.toString().toByteArray())

    fun snapshot(): JSONObject? = synchronized(lock) {
        json("native-snapshot")?.takeUnless { expired(it.optString("expiresAt")) }
    }

    fun publish(snapshot: JSONObject) = synchronized(lock) {
        require(snapshot.optInt("version") == 1 && subject(snapshot.optString("scope")) != null) { "Invalid native snapshot" }
        require(!expired(snapshot.optString("expiresAt"))) { "Native snapshot expired" }
        val previous = json("native-snapshot")?.optString("scope")
        if (previous != null && previous != snapshot.getString("scope")) clear()
        save("native-snapshot", snapshot)
    }

    fun mergeStatus(scope: String, turns: JSONArray) = synchronized(lock) {
        val current = snapshot() ?: return@synchronized false
        if (current.optString("scope") != scope) return@synchronized false
        save("native-snapshot", BotStatusPolicy.merge(current, turns, Instant.now()))
        true
    }

    fun clear() = synchronized(lock) {
        secure.delete("native-snapshot")
        secure.delete("native-actions")
        secure.delete("native-notification-actions")
        secure.delete("native-results")
        secure.delete("native-automation")
        directory.listFiles().orEmpty().filter { it.isFile }.forEach { it.delete() }
    }

    private fun queue(): JSONArray = json("native-actions")?.optJSONArray("actions") ?: JSONArray()
    private fun writeQueue(queue: JSONArray) = save("native-actions", JSONObject().put("actions", queue))

    fun pending(): JSONArray = synchronized(lock) {
        val scope = snapshot()?.optString("scope") ?: return@synchronized JSONArray()
        val active = JSONArray()
        val queued = queue()
        for (index in 0 until queued.length()) {
            val item = queued.getJSONObject(index)
            val created = item.optLong("queuedAt", 0)
            if (item.optString("scope") == scope) {
                if (System.currentTimeMillis() - created < MAX_ACTION_AGE_MS &&
                    (!item.has("responseDeadline") || !expired(item.getString("responseDeadline")))) active.put(item)
                else if (item.has("responseMode")) cancelAutomation(item.getString("id"), scope, "The automation timed out.")
                else deliveryFailed(item.getString("id"), scope, "expired")
            }
        }
        writeQueue(active)
        active
    }

    fun enqueue(action: JSONObject, scope: String, id: String = UUID.randomUUID().toString(),
                backgroundDelivery: Boolean = false, deadline: String? = null): JSONObject = synchronized(lock) {
        require(snapshot()?.optString("scope") == scope) { "The active account changed" }
        UUID.fromString(id)
        val queue = pending()
        for (index in 0 until queue.length()) if (queue.getJSONObject(index).getString("id") == id) return@synchronized queue.getJSONObject(index)
        require(queue.length() < MAX_ACTIONS) { "Too many pending actions. Open Flow-Like to finish them." }
        require(action.toString().length <= 128 * 1024) { "Native action is too large" }
        val value = JSONObject().put("id", id).put("scope", scope).put("action", action)
            .put("createdAt", Instant.now().toString()).put("queuedAt", System.currentTimeMillis())
            .put("backgroundDelivery", backgroundDelivery)
        if (deadline != null) value.put("responseDeadline", deadline)
        queue.put(value)
        writeQueue(queue)
        value
    }

    fun acknowledge(id: String, scope: String) = synchronized(lock) {
        val result = JSONArray()
        val queued = queue()
        for (index in 0 until queued.length()) {
            val item = queued.getJSONObject(index)
            if (item.optString("id") != id || item.optString("scope") != scope) result.put(item)
            else item.optJSONObject("action")?.optJSONArray("files")?.let { files ->
                val root = directory.canonicalFile
                for (fileIndex in 0 until files.length()) {
                    val file = File(files.getString(fileIndex)).canonicalFile
                    if (file.parentFile == root) file.delete()
                }
            }
        }
        writeQueue(result)
        val actions = json("native-notification-actions")
        val stored = actions?.optJSONObject(id)
        if (stored?.optString("scope") == scope) {
            actions.remove(id)
            save("native-notification-actions", actions)
        }
        val results = json("native-results")
        if (results?.optJSONObject(id)?.optString("scope") == scope) {
            results.remove(id)
            save("native-results", results)
        }
    }

    fun rememberAction(action: JSONObject, scope: String, expiresAt: String? = null,
                       backgroundDelivery: Boolean = false): String = synchronized(lock) {
        require(snapshot()?.optString("scope") == scope) { "The active account changed" }
        val actions = json("native-notification-actions") ?: JSONObject()
        val retained = JSONObject()
        for (key in actions.keys()) {
            val value = actions.getJSONObject(key)
            if (value.optString("scope") == scope && !expired(value.optString("expiresAt"))) retained.put(key, value)
        }
        require(retained.length() < 256) { "Too many native actions" }
        val id = UUID.randomUUID().toString()
        retained.put(id, JSONObject().put("action", action).put("scope", scope)
            .put("backgroundDelivery", backgroundDelivery)
            .put("expiresAt", expiresAt ?: Instant.now().plusSeconds(86400).toString()))
        save("native-notification-actions", retained)
        id
    }

    fun resolveAction(id: String): JSONObject? = synchronized(lock) {
        val value = json("native-notification-actions")?.optJSONObject(id) ?: return@synchronized null
        value.takeIf { it.optString("scope") == snapshot()?.optString("scope") && !expired(it.optString("expiresAt")) }
    }

    fun complete(result: JSONObject) = synchronized(lock) {
        val id = result.getString("id")
        val scope = result.getString("scope")
        if (snapshot()?.optString("scope") != scope) return@synchronized
        val records = json("native-automation")
        val record = records?.optJSONObject(id)
        if (record != null) {
            if (record.optString("scope") != scope || record.has("result") || expired(record.optString("responseDeadline"))) return@synchronized
            require(result.optString("status") in listOf("success", "error", "interaction_required")) { "Invalid automation outcome" }
            require(result.toString().toByteArray(Charsets.UTF_8).size <= 256 * 1024) { "Automation result is too large" }
            if (result.has("json") && !result.isNull("json")) org.json.JSONTokener(result.getString("json")).nextValue()
            record.put("result", JSONObject(result.toString())).put("expiresAt", Instant.now().plusSeconds(300).toString())
            save("native-automation", records)
        }
        acknowledge(id, scope)
    }

    fun enqueueAutomation(action: JSONObject, scope: String, responseMode: String,
                          id: String = UUID.randomUUID().toString()): JSONObject = synchronized(lock) {
        require(responseMode in listOf("text", "result")) { "Invalid response format" }
        val snapshot = snapshot() ?: error("The workspace is unavailable")
        require(snapshot.optString("scope") == scope)
        BotAutomationPolicy.validate(snapshot, action, responseMode)
        val now = Instant.now()
        val records = json("native-automation") ?: JSONObject()
        val retained = JSONObject()
        for (key in records.keys()) {
            val item = records.getJSONObject(key)
            if (item.optString("scope") == scope && !expired(item.optString("expiresAt"))) retained.put(key, item)
        }
        require(retained.length() < 32) { "Too many automation requests" }
        require(!retained.has(id)) { "This automation already started" }
        val deadline = now.plusSeconds(90).toString()
        val request = enqueue(action, scope, id, deadline = deadline).put("responseMode", responseMode).put("automation", true)
        val queued = queue()
        for (index in 0 until queued.length()) if (queued.getJSONObject(index).optString("id") == id) queued.put(index, request)
        writeQueue(queued)
        retained.put(id, JSONObject().put("id", id).put("scope", scope).put("responseDeadline", deadline)
            .put("expiresAt", now.plusSeconds(390).toString()).put("request", request))
        save("native-automation", retained)
        request
    }

    fun automationCurrent(id: String, scope: String): Boolean = synchronized(lock) {
        if (snapshot()?.optString("scope") != scope) return@synchronized false
        val record = json("native-automation")?.optJSONObject(id) ?: return@synchronized false
        record.optString("scope") == scope && !record.has("result") && !expired(record.optString("responseDeadline"))
    }

    fun automationResult(id: String, scope: String, consume: Boolean = false): JSONObject? = synchronized(lock) {
        if (snapshot()?.optString("scope") != scope) return@synchronized null
        val records = json("native-automation") ?: return@synchronized null
        val record = records.optJSONObject(id) ?: return@synchronized null
        if (record.optString("scope") != scope || expired(record.optString("expiresAt"))) return@synchronized null
        val result = record.optJSONObject("result") ?: return@synchronized null
        if (consume) { records.remove(id); save("native-automation", records) }
        JSONObject(result.toString())
    }

    fun cancelAutomation(id: String, scope: String, reason: String = "The automation was cancelled.") = synchronized(lock) {
        if (snapshot()?.optString("scope") != scope) return@synchronized
        val records = json("native-automation") ?: return@synchronized
        val record = records.optJSONObject(id) ?: return@synchronized
        if (record.optString("scope") != scope || record.has("result")) return@synchronized
        record.put("result", JSONObject().put("id", id).put("scope", scope).put("status", "error").put("error", reason.take(1000)))
            .put("expiresAt", Instant.now().plusSeconds(300).toString())
        save("native-automation", records)
        acknowledge(id, scope)
        BotAutomationSignals.cancelled?.invoke(id, scope)
    }

    fun foregroundActions(): JSONArray = synchronized(lock) {
        val result = JSONArray()
        val actions = pending()
        for (index in 0 until actions.length()) {
            val value = actions.getJSONObject(index)
            if (!value.optBoolean("backgroundDelivery")) result.put(value)
        }
        val scope = snapshot()?.optString("scope")
        val failures = json("native-results") ?: JSONObject()
        for (id in failures.keys()) {
            val failure = failures.getJSONObject(id)
            if (failure.optString("scope") == scope) result.put(failure)
        }
        result
    }

    fun pauseDelivery(id: String, scope: String) = synchronized(lock) {
        val actions = queue()
        for (index in 0 until actions.length()) {
            val value = actions.getJSONObject(index)
            if (value.optString("id") == id && value.optString("scope") == scope) value.put("backgroundDelivery", false)
        }
        writeQueue(actions)
    }

    fun deliveryFailed(id: String, scope: String, code: String) = synchronized(lock) {
        val queued = queue()
        val item = (0 until queued.length()).map { queued.getJSONObject(it) }
            .firstOrNull { it.optString("id") == id && it.optString("scope") == scope } ?: return@synchronized
        acknowledge(id, scope)
        val results = json("native-results") ?: JSONObject()
        if (results.length() >= MAX_ACTIONS) results.keys().asSequence().firstOrNull()?.let { results.remove(it) }
        results.put(id, item.put("backgroundDelivery", false)
            .put("backgroundError", "Bot action failed ($code). Open the conversation to review it."))
        save("native-results", results)
    }
}

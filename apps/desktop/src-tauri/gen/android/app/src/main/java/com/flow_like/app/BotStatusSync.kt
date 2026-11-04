package com.flow_like.app

import android.content.Context
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import org.json.JSONArray
import org.json.JSONObject
import java.net.URI
import java.time.Instant
import java.util.concurrent.TimeUnit
import javax.net.ssl.HttpsURLConnection

internal object BotStatusPolicy {
    fun merge(snapshot: JSONObject, turns: JSONArray, now: Instant): JSONObject {
        require(turns.length() <= 50) { "Too many bot statuses" }
        val result = JSONObject(snapshot.toString())
        val bots = result.optJSONArray("bots") ?: JSONArray()
        val remoteBots = (0 until bots.length()).map { bots.getJSONObject(it) }
            .filter { !it.optBoolean("local") }.associateBy { it.getString("id") }
        val active = mutableListOf<JSONObject>()
        val identifier = Regex("[A-Za-z0-9_-]{1,128}")
        val owner = BotNativeStore.subject(snapshot.getString("scope"))
        for (index in 0 until turns.length()) {
            val turn = turns.getJSONObject(index)
            val bot = remoteBots[turn.optString("bot_id")] ?: continue
            if (turn.optString("status") !in listOf("queued", "running", "waiting") || turn.optString("owner_id") != owner) continue
            if (!identifier.matches(turn.optString("id")) || !identifier.matches(turn.optString("conversation_id"))) continue
            val item = JSONObject().put("id", turn.getString("id")).put("title", bot.getString("title"))
                .put("status", turn.getString("status")).put("live", true)
                .put("action", JSONObject().put("kind", "open_conversation").put("botId", bot.getString("id"))
                    .put("conversationId", turn.getString("conversation_id")).put("turnId", turn.getString("id")))
            for ((source, target) in listOf("started_at" to "startedAt", "updated_at" to "updatedAt")) {
                val time = turn.optString(source)
                if (time.isNotEmpty()) try { Instant.parse(time); item.put(target, time) } catch (_: Exception) { }
            }
            active.add(item)
        }
        val sections = result.optJSONArray("sections") ?: JSONArray()
        for (index in 0 until sections.length()) {
            val section = sections.getJSONObject(index)
            if (section.optString("kind") != "bot_tasks") continue
            val expiry = minOf(Instant.parse(snapshot.getString("expiresAt")), now.plusSeconds(3600))
            section.put("items", JSONArray(active.take(20))).put("state", "ready")
                .put("updatedAt", now.toString()).put("staleAt", minOf(expiry, now.plusSeconds(1800)).toString())
                .put("expiresAt", expiry.toString())
        }
        // Only observed remote turns become fresh. The separate botTasks catalog keeps its age.
        return result
    }
}

internal object BotStatusSync {
    private const val WORK = "bot-status-refresh"
    enum class Outcome { COMPLETE, RETRY }

    fun schedule(context: Context) {
        val snapshot = BotNativeStore(context).snapshot()
        if (snapshot == null || BotNativeStore.subject(snapshot.optString("scope")) in listOf(null, "local")) { cancel(context); return }
        val request = PeriodicWorkRequestBuilder<BotStatusWorker>(15, TimeUnit.MINUTES)
            .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()).build()
        WorkManager.getInstance(context).enqueueUniquePeriodicWork(WORK, ExistingPeriodicWorkPolicy.KEEP, request)
    }

    fun cancel(context: Context) { WorkManager.getInstance(context).cancelUniqueWork(WORK) }

    fun refresh(context: Context): Outcome {
        val store = BotNativeStore(context)
        val snapshot = store.snapshot() ?: run { cancel(context); BotSurfaces.clearProgress(context); return Outcome.COMPLETE }
        val scope = snapshot.getString("scope")
        val base = BotDeliveryPolicy.apiBase(scope) ?: run { cancel(context); return Outcome.COMPLETE }
        val token = BotDeliveryPolicy.accessToken(scope, BotSecureStore(context)) ?: run { cancel(context); BotSurfaces.clearProgress(context); return Outcome.COMPLETE }
        if (store.snapshot()?.optString("scope") != scope) return Outcome.COMPLETE
        val connection = URI("$base/bots/mobile-status").toURL().openConnection() as HttpsURLConnection
        try {
            connection.instanceFollowRedirects = false
            connection.requestMethod = "GET"
            connection.connectTimeout = 10000
            connection.readTimeout = 10000
            connection.setRequestProperty("Authorization", "Bearer $token")
            connection.setRequestProperty("Accept", "application/json")
            return when (connection.responseCode) {
                200 -> {
                    val body = connection.inputStream.use { it.readBytesBounded(512 * 1024) }
                    val turns = JSONObject(body.toString(Charsets.UTF_8)).getJSONArray("turns")
                    if (store.mergeStatus(scope, turns)) BotSurfaces.refresh(context)
                    Outcome.COMPLETE
                }
                401, 403 -> { cancel(context); BotSurfaces.clearProgress(context); Outcome.COMPLETE }
                408, 429, in 500..599 -> Outcome.RETRY
                else -> Outcome.COMPLETE
            }
        } finally { connection.disconnect() }
    }

    private fun java.io.InputStream.readBytesBounded(limit: Int): ByteArray {
        val output = java.io.ByteArrayOutputStream()
        val buffer = ByteArray(8192)
        while (true) {
            val count = read(buffer)
            if (count < 0) break
            require(output.size() + count <= limit) { "Bot status response is too large" }
            output.write(buffer, 0, count)
        }
        return output.toByteArray()
    }
}

class BotStatusWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result = try {
        if (BotStatusSync.refresh(applicationContext) == BotStatusSync.Outcome.RETRY) Result.retry() else Result.success()
    } catch (_: java.io.IOException) { Result.retry() }
      catch (_: Exception) { Result.failure() }
}

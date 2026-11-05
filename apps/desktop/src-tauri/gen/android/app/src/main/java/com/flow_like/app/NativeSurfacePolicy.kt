package com.flow_like.app

import org.json.JSONArray
import org.json.JSONObject
import java.net.URI
import java.net.URLDecoder
import java.security.MessageDigest
import java.time.Instant

internal data class NativeSurfaceChoice(val kind: String, val id: String = "", val title: String)

/** Only published entities and routes become native launchers. */
internal object NativeSurfacePolicy {
    val sections = linkedMapOf("flowpilot" to "FlowPilot", "bots" to "Bots", "bot_tasks" to "Bot tasks",
        "bot_approvals" to "Bot approvals", "inbox" to "Notifications", "attention" to "Needs attention",
        "recent_apps" to "Recent apps", "recent_runs" to "Recent runs", "usage" to "App usage",
        "workspace" to "Workspace", "event_favorites" to "Event favorites")

    fun array(value: JSONArray?): List<JSONObject> = (0 until minOf(value?.length() ?: 0, 200))
        .mapNotNull { value?.optJSONObject(it) }
    fun key(scope: String, kind: String, id: String): String = MessageDigest.getInstance("SHA-256")
        .digest("$scope\u0000$kind\u0000$id".toByteArray()).joinToString("") { "%02x".format(it) }
    fun choices(snapshot: JSONObject, search: Boolean = false): List<NativeSurfaceChoice> {
        val result = if (search) mutableListOf() else sections.map { NativeSurfaceChoice(it.key, title = it.value) }.toMutableList()
        for (bot in array(snapshot.optJSONArray("bots"))) {
            if (!search || arrayStrings(bot.optJSONArray("surfaces")).contains("spotlight"))
                result.add(NativeSurfaceChoice("bot", bot.optString("id"), bot.optString("title", "Bot")))
        }
        for (app in array(snapshot.optJSONArray("apps"))) {
            if (!search || app.optBoolean("spotlightEligible"))
                result.add(NativeSurfaceChoice("app", app.optString("id"), app.optString("title", "App")))
        }
        for (event in array(snapshot.optJSONArray("events"))) {
            if (arrayStrings(event.optJSONArray("surfaces")).contains(if (search) "spotlight" else "widget"))
                result.add(NativeSurfaceChoice("event", event.optString("id"), event.optString("title", "Event")))
        }
        if (!search) for (widget in array(snapshot.optJSONArray("customWidgets"))) {
            if (validCustom(widget, snapshot)) result.add(NativeSurfaceChoice("custom", widget.getString("id"), widget.optString("title", "App widget")))
        }
        return result.filter { it.title.isNotBlank() && (it.id.isNotBlank() || it.kind in sections) }
    }
    private fun arrayStrings(array: JSONArray?) = (0 until (array?.length() ?: 0)).map { array!!.optString(it) }
    fun section(snapshot: JSONObject, kind: String): JSONObject? = array(snapshot.optJSONArray("sections")).firstOrNull { it.optString("kind") == kind }
    fun custom(snapshot: JSONObject, id: String): JSONObject? = array(snapshot.optJSONArray("customWidgets"))
        .firstOrNull { it.optString("id") == id && validCustom(it, snapshot) }
    private fun validCustom(widget: JSONObject, snapshot: JSONObject): Boolean =
        widget.optString("kind") in listOf("chart", "page") && widget.toString().toByteArray().size <= 131072 &&
            !BotNativeStore.expired(widget.optString("expiresAt")) && array(snapshot.optJSONArray("apps")).any { it.optString("id") == widget.optString("appId") }

    fun action(snapshot: JSONObject, choice: NativeSurfaceChoice, itemId: String? = null): JSONObject? {
        val action = when (choice.kind) {
            "bot" -> array(snapshot.optJSONArray("bots")).firstOrNull { it.optString("id") == choice.id }?.let { JSONObject().put("kind", "open_bot").put("botId", choice.id) }
            "app" -> array(snapshot.optJSONArray("apps")).firstOrNull { it.optString("id") == choice.id }?.let { JSONObject().put("kind", "open_app").put("appId", choice.id) }
            "event" -> array(snapshot.optJSONArray("events")).firstOrNull { it.optString("id") == choice.id }?.optJSONObject("action")
            "custom" -> custom(snapshot, choice.id)?.optJSONObject("action")
            "flowpilot" -> JSONObject().put("kind", "flowpilot")
            "inbox" -> if (itemId == null) JSONObject().put("kind", "open_inbox") else item(snapshot, choice.kind, itemId)
            "bots" -> if (itemId == null) JSONObject().put("kind", "open_bot") else item(snapshot, choice.kind, itemId)
            else -> if (itemId == null) JSONObject().put("kind", "open_home") else item(snapshot, choice.kind, itemId)
        } ?: return null
        return action.takeIf { validAction(it, snapshot) }?.let { JSONObject(it.toString()) }
    }
    private fun item(snapshot: JSONObject, kind: String, id: String): JSONObject? {
        val section = section(snapshot, kind) ?: return null
        if (section.has("expiresAt") && BotNativeStore.expired(section.getString("expiresAt"))) return null
        return array(section.optJSONArray("items")).firstOrNull { it.optString("id") == id }?.optJSONObject("action")
    }
    fun validAction(action: JSONObject, snapshot: JSONObject): Boolean {
        val kind = action.optString("kind")
        if (kind !in setOf("open_home", "open_inbox", "flowpilot", "open_bot", "open_conversation", "open_app", "open_event", "run_event", "open_run", "run_bot_task", "capture_bot_voice", "capture_bot_document", "respond_bot_approval")) return false
        if (action.has("botId") && array(snapshot.optJSONArray("bots")).none { it.optString("id") == action.optString("botId") }) return false
        if (action.has("appId") && array(snapshot.optJSONArray("apps")).none { it.optString("id") == action.optString("appId") }) return false
        if (kind in listOf("open_event", "run_event") && array(snapshot.optJSONArray("events")).none {
            it.optString("appId") == action.optString("appId") && it.optString("eventId") == action.optString("eventId")
        }) return false
        if (action.has("path") && !validPath(action.optString("path"))) return false
        return action.toString().length <= 16384
    }
    fun validPath(path: String): Boolean = try {
        val uri = URI(path)
        path.length <= 4096 && path.startsWith('/') && !path.startsWith("//") && !path.contains('\\') &&
            uri.scheme == null && uri.rawAuthority == null && uri.rawFragment == null &&
            uri.path.split('/').none { it == ".." || it == "." } &&
            uri.path.none { it.code < 32 || it == '\\' } && path.none { it.code < 32 }
    } catch (_: Exception) { false }

    fun continuation(snapshot: JSONObject, url: String): Boolean = try {
        val target = URI(url)
        val web = URI(snapshot.getString("webOrigin"))
        url.length <= 8192 && target.scheme == "https" && web.scheme == "https" && target.host == web.host &&
            target.port == web.port && target.rawUserInfo == null && target.rawFragment == null &&
            !url.contains('\\') && (target.path == "/bots" || target.path == "/use" || target.path.startsWith("/use/")) &&
            target.path.split('/').none { it == "." || it == ".." } &&
            target.path.none { it.code < 32 || it == '\\' } && !target.rawPath.lowercase().contains("%2f")
    } catch (_: Exception) { false }
    fun continuationRequest(snapshot: JSONObject, raw: String): String? = try {
        val outer = URI(raw)
        require(raw.length <= 32768 && outer.scheme == "flow-like" && outer.host == "continue" &&
            outer.rawUserInfo == null && outer.port == -1 && outer.rawFragment == null && outer.rawPath in listOf("", "/"))
        val query = (outer.rawQuery ?: "").split('&').map { parameter ->
            val pair = parameter.split('=', limit = 2)
            URLDecoder.decode(pair[0], "UTF-8") to URLDecoder.decode(pair.getOrElse(1) { "" }, "UTF-8")
        }
        val scopes = query.filter { it.first == "scope" }.map { it.second }
        val urls = query.filter { it.first == "url" }.map { it.second }
        require(scopes.size == 1 && scopes.single() == snapshot.getString("scope") && urls.size == 1)
        urls.single().takeIf { continuation(snapshot, it) }
    } catch (_: Exception) { null }
    fun observedAge(snapshot: JSONObject, source: JSONObject = snapshot): String {
        val staleAt = source.optString("staleAt", snapshot.optString("staleAt"))
        if (!BotNativeStore.expired(staleAt)) return ""
        val timestamp = source.optString("updatedAt", snapshot.optString("generatedAt"))
        return try {
            val minutes = ((Instant.now().toEpochMilli() - Instant.parse(timestamp).toEpochMilli()) / 60000).coerceAtLeast(1)
            if (minutes < 60) "Updated ${minutes}m ago" else "Updated ${minutes / 60}h ago"
        } catch (_: Exception) { "Open Flow-Like to refresh" }
    }
}

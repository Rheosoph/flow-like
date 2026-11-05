package com.flow_like.app

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.PersistableBundle
import android.util.Base64
import androidx.core.content.FileProvider
import org.json.JSONObject
import java.io.File
import java.util.UUID

internal object NativeClipboardPolicy {
    fun validate(payload: JSONObject, now: Long) {
        for (field in listOf("appPrivate", "localOnly")) require(!payload.has(field) || payload.get(field) is Boolean) { "Invalid clipboard privacy option." }
        for (field in listOf("expiresAt", "requestDeadline")) if (payload.has(field) && !payload.isNull(field)) {
            val value = payload.get(field)
            require(value is Number && value.toDouble().isFinite() && value.toDouble() == value.toLong().toDouble() && value.toLong() > now) { "Clipboard deadline must be a future timestamp." }
        }
        require(payload.getString("format") in listOf("text", "html", "image")) { "Unsupported clipboard format." }
    }
    fun readable(value: JSONObject, scope: String?, now: Long): Boolean = !scope.isNullOrEmpty() && value.optString("scope") == scope && value.optLong("expiresAt") > now
}

internal object NativeDeviceClipboard {
    private const val KEY = "device-private-clipboard"
    private const val MAX_BYTES = 3 * 1024 * 1024
    private val remoteExclusion get() = try { ClipDescription::class.java.getMethod("setExcludedFromRemoteClipboard", Boolean::class.javaPrimitiveType) } catch (_: Exception) { null }

    fun capabilities(): JSONObject = JSONObject().put("localOnly", remoteExclusion != null).put("expiry", false)
        .put("appPrivate", true).put("appPrivateExpiry", true)

    fun write(context: Context, payload: JSONObject): JSONObject { return try {
        val now = System.currentTimeMillis()
        NativeClipboardPolicy.validate(payload, now)
        val deadline = payload.optLong("requestDeadline", Long.MAX_VALUE)
        require(deadline > now) { "The clipboard request expired." }
        val private = payload.optBoolean("appPrivate")
        val local = payload.optBoolean("localOnly")
        val expiry = if (payload.has("expiresAt") && !payload.isNull("expiresAt")) payload.getLong("expiresAt") else null
        require(expiry == null || expiry > now) { "Clipboard expiration must be in the future." }
        val format = payload.getString("format")
        require(format in listOf("text", "html", "image")) { "Unsupported clipboard format." }
        val value = JSONObject().put("format", format)
        when (format) {
            "text" -> value.put("text", payload.getString("text"))
            "html" -> value.put("html", payload.getString("html")).put("text", payload.optString("text"))
            "image" -> {
                val encoded = payload.getString("imageBase64")
                require(encoded.length <= MAX_BYTES)
                val bytes = Base64.decode(encoded, Base64.DEFAULT)
                require(bytes.size >= 8 && bytes.take(8) == listOf(137, 80, 78, 71, 13, 10, 26, 10).map { it.toByte() })
                value.put("imageBase64", encoded)
            }
        }
        require(value.toString().toByteArray().size <= MAX_BYTES) { "This clipboard item is too large." }
        if (private) {
            val scope = BotNativeStore(context).snapshot()?.optString("scope")?.takeIf { it.isNotEmpty() }
                ?: return deviceFailure("unavailable", "Open a workspace before using private copy.")
            if (payload.has("scope") && payload.optString("scope") != scope) return deviceFailure("scope_changed", "The active workspace changed before private copy.")
            val expires = expiry ?: (now + 30 * 60_000)
            value.put("scope", scope).put("expiresAt", expires).put("appPrivate", true)
            BotSecureStore(context).write(KEY, value.toString().toByteArray())
            deviceSuccess(JSONObject().put("appPrivate", true).put("localOnly", true).put("expiresAt", expires))
        } else {
            if (expiry != null) return deviceFailure("unsupported", "Android cannot guarantee system clipboard expiry. Choose private copy for an expiring item.")
            if (local && remoteExclusion == null) return deviceFailure("unsupported", "This Android version cannot exclude system clipboard sync. Choose private copy to keep content in Flow Like.")
            val clip = when (format) {
                "text" -> ClipData.newPlainText("Flow Like", value.getString("text"))
                "html" -> ClipData.newHtmlText("Flow Like", value.getString("text"), value.getString("html"))
                else -> {
                    val folder = File(context.cacheDir, "device-clipboard").apply { mkdirs() }
                    folder.listFiles().orEmpty().filter { now - it.lastModified() > 24 * 60 * 60_000 }.forEach { it.delete() }
                    val file = File(folder, "${UUID.randomUUID()}.png")
                    file.writeBytes(Base64.decode(value.getString("imageBase64"), Base64.DEFAULT))
                    ClipData.newUri(context.contentResolver, "Flow Like image", FileProvider.getUriForFile(context, context.packageName + ".fileprovider", file))
                }
            }
            clip.description.extras = PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
            if (local) remoteExclusion!!.invoke(clip.description, true)
            context.getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
            deviceSuccess(JSONObject().put("appPrivate", false).put("localOnly", local))
        }
    } catch (error: IllegalArgumentException) { deviceFailure("invalid_input", error.message ?: "Invalid clipboard content.") }
      catch (_: Exception) { deviceFailure("unavailable", "The clipboard item could not be written.") }
    }

    fun readPrivate(context: Context): JSONObject { return try {
        val store = BotSecureStore(context)
        val bytes = store.read(KEY) ?: return deviceFailure("empty", "There is no private copy to paste.")
        val value = JSONObject(bytes.toString(Charsets.UTF_8))
        if (!NativeClipboardPolicy.readable(value, BotNativeStore(context).snapshot()?.optString("scope"), System.currentTimeMillis())) {
            store.delete(KEY)
            deviceFailure("expired", "The private copy expired or belongs to another workspace.")
        } else {
            value.remove("scope")
            deviceSuccess(value)
        }
    } catch (_: Exception) { deviceFailure("unavailable", "The private copy could not be read.") }
    }

    fun clearPrivate(context: Context) { BotSecureStore(context).delete(KEY) }
}

package com.flow_like.app

import android.content.Context
import androidx.annotation.Keep
import org.json.JSONObject
import java.io.File

/** Rust owns login generations and refresh rotation, including WorkManager cold starts. */
@Keep
internal object BotCredentialBroker {
    @Volatile private var application: Context? = null
    @Volatile private var loaded = false

    @Synchronized fun init(context: Context) {
        application = context.applicationContext
        if (!loaded) { System.loadLibrary("app"); loaded = true }
    }

    fun accessToken(context: Context, scope: String, rejectedToken: String? = null): String? = try {
        init(context)
        if (currentScope() != scope || BotDeliveryPolicy.apiBase(scope) == null) null
        else nativeAccessToken(scope, rejectedToken)?.takeIf {
            currentScope() == scope && it.isNotEmpty() && it.length <= 65536 && it.all { c -> c.code in 33..126 }
        }
    } catch (_: Exception) { null } catch (_: LinkageError) { null }

    @JvmStatic private external fun nativeAccessToken(scope: String, rejectedToken: String?): String?
    @JvmStatic private external fun nativeGeofenceReady(): Boolean
    fun geofenceReady(context: Context): Boolean = try { init(context); nativeGeofenceReady() }
        catch (_: Exception) { false } catch (_: LinkageError) { false }

    @JvmStatic fun secureRecordRead(key: String): ByteArray? = BotSecureStore(requireNotNull(application)).read("credential:$key")
    @JvmStatic fun secureRecordWrite(key: String, value: ByteArray) = BotSecureStore(requireNotNull(application)).write("credential:$key", value)
    @JvmStatic fun secureRecordDelete(key: String) = BotSecureStore(requireNotNull(application)).delete("credential:$key")
    @JvmStatic fun currentScope(): String? = application?.let { BotNativeStore(it).snapshot()?.optString("scope") }
    @JvmStatic fun settingsJson(): String? {
        val root = application?.filesDir ?: return null
        val file = File(root, "flow-like/global-settings.json")
        if (!file.isFile || file.length() > 4 * 1024 * 1024) return null
        return file.readText(Charsets.UTF_8)
    }
}

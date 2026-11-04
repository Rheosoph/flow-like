package com.flow_like.app

import android.content.Context
import org.json.JSONObject
import java.io.IOException
import java.net.URI
import javax.net.ssl.HttpsURLConnection

internal class BotHttp(private val context: Context, private val scope: String) {
    data class Response(val status: Int, val body: JSONObject?)
    fun request(method: String, path: String, body: JSONObject? = null): Response {
        val base = BotDeliveryPolicy.apiBase(scope) ?: throw IOException("Invalid scoped hub")
        var token = BotCredentialBroker.accessToken(context, scope) ?: throw IOException("Native sign-in is unavailable")
        repeat(2) { attempt ->
            if (BotNativeStore(context).snapshot()?.optString("scope") != scope) throw IOException("Workspace changed")
            val connection = URI("$base/$path").toURL().openConnection() as HttpsURLConnection
            val response = try {
                connection.instanceFollowRedirects = false
                connection.requestMethod = method
                connection.connectTimeout = 8000
                connection.readTimeout = 8000
                connection.setRequestProperty("Authorization", "Bearer $token")
                connection.setRequestProperty("Accept", "application/json")
                if (body != null) {
                    val bytes = body.toString().toByteArray(Charsets.UTF_8)
                    connection.setRequestProperty("Content-Type", "application/json")
                    connection.doOutput = true
                    connection.setFixedLengthStreamingMode(bytes.size)
                    connection.outputStream.use { it.write(bytes) }
                }
                val status = connection.responseCode
                val value = if (status in 200..299) connection.inputStream.use { input ->
                    val output = java.io.ByteArrayOutputStream()
                    val buffer = ByteArray(8192)
                    while (true) {
                        val count = input.read(buffer)
                        if (count < 0) break
                        if (output.size() + count > 512 * 1024) throw IOException("Bot response is too large")
                        output.write(buffer, 0, count)
                    }
                    if (output.size() == 0) null else JSONObject(output.toString("UTF-8"))
                } else null
                Response(status, value)
            } finally { connection.disconnect() }
            if (response.status != 401 || attempt != 0) return response
            token = BotCredentialBroker.accessToken(context, scope, token) ?: return response
        }
        throw IOException("Sign-in renewal failed")
    }
}

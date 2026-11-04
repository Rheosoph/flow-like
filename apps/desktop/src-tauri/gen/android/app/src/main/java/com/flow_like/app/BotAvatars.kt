package com.flow_like.app

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.MalformedURLException
import java.net.URL

/** What a bot push shows: decisions without Android dependencies, so they stay unit-testable. */
internal object BotSenderPolicy {
    private val BOT_ID = Regex("[A-Za-z0-9_-]{1,128}")

    /** The bot names the message, so a title that only repeats its name is dropped. */
    fun messageText(title: String?, body: String?, sender: String): String =
        listOf(title?.takeIf { it.trim() != sender.trim() }, body)
            .filterNot { it.isNullOrBlank() }
            .joinToString("\n")

    /** The hub signs a public HTTPS link to the picture of the bot's current look. */
    fun avatarUrl(value: String?): URL? = try {
        value?.let(::URL)?.takeIf { it.protocol == "https" && it.userInfo == null && !it.host.isNullOrEmpty() }
    } catch (_: MalformedURLException) {
        null
    }

    fun cacheName(botId: String): String? = botId.takeIf { BOT_ID.matches(it) }?.let { "$it.png" }
}

/** The picture of each bot's look, kept so a push that can't fetch it still shows the last one. */
internal object BotAvatars {
    private const val DIRECTORY = "bot-avatars"
    private const val MAX_BYTES = 1024 * 1024
    private const val MAX_PIXELS = 256
    private const val MAX_SOURCE_PIXELS = 4096
    private const val TIMEOUT_MS = 5_000

    fun load(context: Context, botId: String, url: String?): Bitmap? =
        BotSenderPolicy.avatarUrl(url)?.let(::fetch)?.also { store(context, botId, it) } ?: cached(context, botId)

    fun cached(context: Context, botId: String): Bitmap? = try {
        file(context, botId)?.takeIf { it.isFile }?.let { decode(it.readBytes()) }
    } catch (_: Exception) {
        null
    }

    fun clear(context: Context) {
        File(context.cacheDir, DIRECTORY).deleteRecursively()
    }

    private fun file(context: Context, botId: String): File? =
        BotSenderPolicy.cacheName(botId)?.let { File(File(context.cacheDir, DIRECTORY), it) }

    private fun fetch(url: URL): Bitmap? = try {
        val connection = url.openConnection() as HttpURLConnection
        connection.connectTimeout = TIMEOUT_MS
        connection.readTimeout = TIMEOUT_MS
        connection.instanceFollowRedirects = false
        try {
            if (connection.responseCode != HttpURLConnection.HTTP_OK || connection.contentLengthLong > MAX_BYTES) null
            else connection.inputStream.use(::bounded)?.let(::decode)
        } finally {
            connection.disconnect()
        }
    } catch (_: Exception) {
        null
    }

    private fun bounded(input: InputStream): ByteArray? {
        val output = ByteArrayOutputStream()
        val buffer = ByteArray(16 * 1024)
        while (true) {
            val read = input.read(buffer)
            if (read < 0) return output.toByteArray()
            if (output.size() + read > MAX_BYTES) return null
            output.write(buffer, 0, read)
        }
    }

    private fun decode(bytes: ByteArray): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        val (width, height) = bounds.outWidth to bounds.outHeight
        if (width <= 0 || height <= 0 || width > MAX_SOURCE_PIXELS || height > MAX_SOURCE_PIXELS) return null
        var sample = 1
        while (width / (sample * 2) >= MAX_PIXELS && height / (sample * 2) >= MAX_PIXELS) sample *= 2
        val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
            ?: return null
        val scale = MAX_PIXELS.toFloat() / maxOf(bitmap.width, bitmap.height)
        return if (scale >= 1f) bitmap
        else Bitmap.createScaledBitmap(bitmap, (bitmap.width * scale).toInt().coerceAtLeast(1), (bitmap.height * scale).toInt().coerceAtLeast(1), true)
    }

    private fun store(context: Context, botId: String, bitmap: Bitmap) {
        val target = file(context, botId) ?: return
        try {
            target.parentFile?.mkdirs()
            val staged = File(target.parentFile, target.name + ".tmp")
            staged.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
            if (!staged.renameTo(target)) staged.delete()
        } catch (_: Exception) { }
    }
}

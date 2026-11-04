package com.flow_like.app

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import android.graphics.pdf.PdfRenderer
import android.media.ExifInterface
import android.os.Looper
import android.os.ParcelFileDescriptor
import com.google.android.gms.tasks.Tasks
import com.google.mlkit.vision.common.InputImage
import com.google.mlkit.vision.text.TextRecognition
import com.google.mlkit.vision.text.latin.TextRecognizerOptions
import org.json.JSONArray
import java.io.File
import java.util.concurrent.TimeUnit

/** Bundled recognition runs on the device; failures leave the original attachment available. */
internal object NativeDeviceOcr {
    fun readFiles(context: Context, paths: JSONArray): String {
        check(Looper.myLooper() != Looper.getMainLooper())
        val root = BotNativeStore.sharedDirectory(context).canonicalFile
        val output = StringBuilder()
        val deadline = android.os.SystemClock.elapsedRealtime() + 60_000
        try { TextRecognition.getClient(TextRecognizerOptions.DEFAULT_OPTIONS).use { recognizer ->
            fun recognize(bitmap: Bitmap) {
                var processing: com.google.android.gms.tasks.Task<com.google.mlkit.vision.text.Text>? = null
                try {
                    val remaining = deadline - android.os.SystemClock.elapsedRealtime()
                    if (remaining <= 0 || output.length >= 32_000) return
                    val task = recognizer.process(InputImage.fromBitmap(bitmap, 0))
                    processing = task
                    val text = Tasks.await(task, minOf(remaining, 10_000), TimeUnit.MILLISECONDS).text
                    if (text.isNotBlank()) { if (output.isNotEmpty()) output.append("\n\n"); output.append(text.take(32_000 - output.length)) }
                } finally {
                    val task = processing
                    if (task == null || task.isComplete) bitmap.recycle()
                    else task.addOnCompleteListener { bitmap.recycle() }
                }
            }
            for (index in 0 until minOf(paths.length(), 10)) {
                if (android.os.SystemClock.elapsedRealtime() >= deadline || output.length >= 32_000) break
                try {
                    val file = File(paths.getString(index)).canonicalFile
                    require(file.parentFile == root && file.isFile && file.length() in 1..20L * 1024 * 1024)
                    if (file.extension.equals("pdf", true)) {
                        PdfRenderer(ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)).use { renderer ->
                            for (page in 0 until minOf(renderer.pageCount, 10)) {
                                if (android.os.SystemClock.elapsedRealtime() >= deadline || output.length >= 32_000) break
                                renderer.openPage(page).use { source ->
                                    val scale = minOf(1.0, 1800.0 / maxOf(source.width, source.height))
                                    val image = Bitmap.createBitmap(maxOf(1, (source.width * scale).toInt()), maxOf(1, (source.height * scale).toInt()), Bitmap.Config.ARGB_8888)
                                    image.eraseColor(android.graphics.Color.WHITE)
                                    source.render(image, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                                    recognize(image)
                                }
                            }
                        }
                    } else {
                        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                        BitmapFactory.decodeFile(file.path, bounds)
                        if (bounds.outWidth !in 1..20_000 || bounds.outHeight !in 1..20_000) continue
                        var sample = 1
                        while (maxOf(bounds.outWidth, bounds.outHeight) / sample > 1800) sample *= 2
                        var image = BitmapFactory.decodeFile(file.path, BitmapFactory.Options().apply { inSampleSize = sample }) ?: continue
                        val rotation = try { when (ExifInterface(file.path).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)) {
                            ExifInterface.ORIENTATION_ROTATE_90 -> 90f
                            ExifInterface.ORIENTATION_ROTATE_180 -> 180f
                            ExifInterface.ORIENTATION_ROTATE_270 -> 270f
                            else -> 0f
                        } } catch (_: Exception) { 0f }
                        if (rotation != 0f) {
                            val rotated = Bitmap.createBitmap(image, 0, 0, image.width, image.height, Matrix().apply { postRotate(rotation) }, true)
                            if (rotated !== image) image.recycle()
                            image = rotated
                        }
                        recognize(image)
                    }
                } catch (_: Exception) { }
            }
        } } catch (_: Exception) { }
        return output.take(32_000).toString()
    }
}

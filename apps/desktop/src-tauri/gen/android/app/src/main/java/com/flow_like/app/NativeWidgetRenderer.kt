package com.flow_like.app

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.Typeface
import android.util.Base64
import org.json.JSONObject
import kotlin.math.max
import kotlin.math.min

/** Renders the bounded native projection, never arbitrary HTML, scripts, or network images. */
internal object NativeWidgetRenderer {
    private val colors = intArrayOf(0xffff9b52.toInt(), 0xff60a5fa.toInt(), 0xff2dd4bf.toInt(), 0xffc084fc.toInt(), 0xfffacc15.toInt(), 0xfffb7185.toInt())
    private fun text(canvas: Canvas, value: String, x: Float, y: Float, width: Float, size: Float = 16f, color: Int = Color.WHITE, bold: Boolean = false) {
        val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { this.color = color; textSize = size; typeface = if (bold) Typeface.DEFAULT_BOLD else Typeface.DEFAULT }
        val clean = value.replace('\n', ' ')
        val count = paint.breakText(clean, true, max(1f, width), null)
        canvas.drawText(if (count < clean.length) clean.take(max(0, count - 1)) + "…" else clean, x, y, paint)
    }
    fun render(widget: JSONObject, width: Int, height: Int): Bitmap {
        val bitmap = Bitmap.createBitmap(width.coerceIn(120, 480), height.coerceIn(80, 280), Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        canvas.drawColor(Color.TRANSPARENT)
        val bounds = RectF(8f, 4f, bitmap.width - 8f, bitmap.height - 4f)
        val selected = when (widget.optString("accent")) { "blue" -> 1; "teal" -> 2; "purple" -> 3; else -> 0 }
        val palette = colors.copyOf().apply { this[0] = colors[selected]; this[selected] = colors[0] }
        if (widget.optString("kind") == "chart") widget.optJSONObject("chart")?.let { chart(canvas, it, bounds, palette) }
        else widget.optJSONObject("page")?.let { page(canvas, it, bounds, 0, intArrayOf(0), palette) }
        return bitmap
    }
    fun description(widget: JSONObject): String {
        val result = mutableListOf<String>()
        fun collect(node: JSONObject, depth: Int) {
            if (depth > 8 || result.size > 100) return
            for (key in listOf("text", "value", "formattedValue")) node.optString(key).takeIf { it.isNotBlank() }?.let { result.add(it) }
            NativeSurfacePolicy.array(node.optJSONArray("points")).forEach { result.add(it.optString("label") + ": " + it.optString("formattedValue", it.optString("value"))) }
            node.optJSONObject("table")?.let { table ->
                table.optJSONArray("columns")?.let { columns -> for (i in 0 until min(6, columns.length())) result.add(columns.optString(i)) }
                table.optJSONArray("rows")?.let { rows -> for (i in 0 until min(8, rows.length())) rows.optJSONArray(i)?.let { row ->
                    result.add((0 until min(6, row.length())).joinToString(", ") { row.optString(it) })
                } }
            }
            NativeSurfacePolicy.array(node.optJSONArray("children")).forEach { collect(it, depth + 1) }
            node.optJSONObject("chart")?.let { collect(it, depth + 1) }
        }
        collect(widget.optJSONObject("chart") ?: widget.optJSONObject("page") ?: JSONObject(), 0)
        return result.joinToString(". ").take(4000)
    }
    private fun chart(canvas: Canvas, chart: JSONObject, bounds: RectF, palette: IntArray) {
        val points = NativeSurfacePolicy.array(chart.optJSONArray("points")).take(120).filter { it.optDouble("value").isFinite() }
        val type = chart.optString("type")
        val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        if (type in listOf("stat", "progress", "gauge")) {
            val value = chart.optDouble("value", points.firstOrNull()?.optDouble("value") ?: 0.0).takeIf { it.isFinite() } ?: 0.0
            val formatted = chart.optString("formattedValue", value.toString())
            if (type != "stat") {
                val target = chart.optDouble("target", 100.0).takeIf { it.isFinite() && it > 0 } ?: 100.0
                val progress = (value / target).coerceIn(0.0, 1.0).toFloat()
                if (type == "gauge") {
                    val diameter = min(bounds.width() - 30, bounds.height() * 1.5f)
                    val arc = RectF(bounds.centerX() - diameter / 2, bounds.top + 15, bounds.centerX() + diameter / 2, bounds.top + diameter + 15)
                    paint.style = Paint.Style.STROKE; paint.strokeWidth = 18f; paint.strokeCap = Paint.Cap.ROUND
                    paint.color = 0xff41414a.toInt(); canvas.drawArc(arc, 180f, 180f, false, paint)
                    paint.color = palette[0]; canvas.drawArc(arc, 180f, progress * 180f, false, paint)
                    text(canvas, formatted, bounds.left + 25, bounds.top + diameter / 2 - 4, bounds.width() - 50, 34f, bold = true)
                    return
                }
                val bar = RectF(bounds.left, bounds.centerY() + 24f, bounds.right, bounds.centerY() + 40f)
                paint.color = 0xff41414a.toInt(); canvas.drawRoundRect(bar, 8f, 8f, paint)
                bar.right = bar.left + bounds.width() * progress; paint.color = palette[0]; canvas.drawRoundRect(bar, 8f, 8f, paint)
            }
            text(canvas, formatted, bounds.left, bounds.centerY(), bounds.width(), 40f, bold = true)
            return
        }
        if (points.isEmpty()) { text(canvas, "No chart data", bounds.left, bounds.centerY(), bounds.width()); return }
        if (type in listOf("pie", "donut")) {
            val positive = points.filter { it.optDouble("value") >= 0 }
            val sum = positive.sumOf { it.optDouble("value") }
            if (sum <= 0) return
            val diameter = min(bounds.height(), bounds.width() * .6f)
            val pie = RectF(bounds.left, bounds.top, bounds.left + diameter, bounds.top + diameter)
            var start = -90f
            positive.forEachIndexed { index, point ->
                val sweep = (point.optDouble("value") / sum * 360).toFloat()
                paint.color = palette[index % palette.size]; canvas.drawArc(pie, start, sweep, true, paint); start += sweep
                if (index < 6) text(canvas, point.optString("label") + " " + point.optString("formattedValue"), pie.right + 12, bounds.top + 22 + index * 28f, bounds.right - pie.right - 12, 14f, paint.color)
            }
            if (type == "donut") { paint.color = 0xff202026.toInt(); canvas.drawCircle(pie.centerX(), pie.centerY(), diameter * .28f, paint) }
            return
        }
        val series = points.map { it.optString("series") }.distinct().take(6)
        val labels = points.map { it.optString("label") }.distinct().take(30)
        val shown = points.filter { it.optString("series") in series && it.optString("label") in labels }
        val domain = NativeChartDomain.calculate(shown, type == "stacked")
        val area = RectF(bounds.left + 40, bounds.top + 8, bounds.right - 4, bounds.bottom - 32)
        fun y(value: Double) = area.bottom - domain.fraction(value) * area.height()
        val baseline = y(0.0)
        paint.color = 0xff676772.toInt(); canvas.drawLine(area.left, baseline, area.right, baseline, paint)
        text(canvas, "0", bounds.left, baseline.coerceIn(bounds.top + 12, area.bottom), 35f, 11f, paint.color)
        text(canvas, "%.2g".format(domain.maximum), bounds.left, area.top + 10, 35f, 11f, paint.color)
        if (domain.minimum < 0) text(canvas, "%.2g".format(domain.minimum), bounds.left, area.bottom, 35f, 11f, paint.color)
        if (type == "horizontal") {
            val rows = shown.take(8); val height = area.height() / max(1, rows.size)
            val left = area.left + area.width() * .32f; val width = area.width() * .67f
            val zero = left + domain.fraction(0.0) * width
            rows.forEachIndexed { index, point ->
                val top = area.top + index * height
                text(canvas, point.optString("label"), area.left, top + 15, area.width() * .3f, 13f)
                paint.color = palette[index % palette.size]
                val endpoint = left + domain.fraction(point.optDouble("value")) * width
                canvas.drawRect(min(zero, endpoint), top + 3, max(zero, endpoint), top + min(height - 5, 20f), paint)
            }
            paint.color = 0xff676772.toInt(); canvas.drawLine(zero, area.top, zero, area.bottom, paint)
        } else if (type in listOf("line", "area")) {
            series.forEachIndexed { s, name ->
                val path = Path(); val values = shown.filter { it.optString("series") == name }.sortedBy { labels.indexOf(it.optString("label")) }
                fun x(point: JSONObject) = area.left + area.width() * labels.indexOf(point.optString("label")) / max(1, labels.size - 1)
                values.forEachIndexed { index, point ->
                    if (index == 0) path.moveTo(x(point), y(point.optDouble("value"))) else path.lineTo(x(point), y(point.optDouble("value")))
                }
                if (type == "area" && values.isNotEmpty()) {
                    val fill = Path(path); fill.lineTo(x(values.last()), baseline); fill.lineTo(x(values.first()), baseline); fill.close()
                    paint.color = palette[s]; paint.alpha = 60; paint.style = Paint.Style.FILL; canvas.drawPath(fill, paint)
                }
                paint.color = palette[s]; paint.alpha = 255; paint.style = Paint.Style.STROKE; paint.strokeWidth = 3f; canvas.drawPath(path, paint); paint.style = Paint.Style.FILL
                values.forEach { canvas.drawCircle(x(it), y(it.optDouble("value")), 3f, paint) }
            }
        } else {
            val stacked = type == "stacked"
            val step = area.width() / max(1, labels.size)
            labels.forEachIndexed { index, label ->
                var positive = 0.0; var negative = 0.0
                shown.filter { it.optString("label") == label }.forEach { point ->
                    val value = point.optDouble("value")
                    val s = series.indexOf(point.optString("series")).coerceAtLeast(0)
                    val start = if (!stacked) 0.0 else if (value >= 0) positive else negative
                    val width = if (stacked) step * .75f else step * .75f / max(1, series.size)
                    val left = area.left + index * step + if (stacked) 0f else s * width
                    paint.color = palette[s % palette.size]
                    canvas.drawRect(left, min(y(start), y(start + value)), left + width, max(y(start), y(start + value)), paint)
                    if (stacked) { if (value >= 0) positive += value else negative += value }
                }
            }
        }
        text(canvas, labels.firstOrNull() ?: "", area.left, bounds.bottom - 6, area.width() * .45f, 12f, 0xffbfc1c8.toInt())
        if (labels.size > 1) text(canvas, labels.last(), area.centerX(), bounds.bottom - 6, area.width() * .5f, 12f, 0xffbfc1c8.toInt())
    }
    private fun page(canvas: Canvas, node: JSONObject, bounds: RectF, depth: Int, count: IntArray, palette: IntArray) {
        if (depth > 8 || ++count[0] > 60 || bounds.height() < 4 || bounds.width() < 4) return
        val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        val children = NativeSurfacePolicy.array(node.optJSONArray("children"))
        when (node.optString("kind")) {
            "row", "grid" -> {
                val columns = if (node.optString("kind") == "row") max(1, children.size) else node.optInt("columns", 2).coerceIn(1, 6)
                val rows = max(1, (children.size + columns - 1) / columns)
                val width = bounds.width() / columns; val height = bounds.height() / rows
                children.forEachIndexed { index, child -> page(canvas, child, RectF(bounds.left + index % columns * width, bounds.top + index / columns * height, bounds.left + (index % columns + 1) * width - 6, bounds.top + (index / columns + 1) * height - 6), depth + 1, count, palette) }
            }
            "column", "stack", "card" -> {
                if (node.optString("kind") == "card") { paint.color = 0xff2e2e38.toInt(); canvas.drawRoundRect(bounds, 12f, 12f, paint) }
                val height = bounds.height() / max(1, children.size)
                children.forEachIndexed { index, child -> page(canvas, child, RectF(bounds.left + 4, bounds.top + index * height + 3, bounds.right - 4, bounds.top + (index + 1) * height - 3), depth + 1, count, palette) }
            }
            "chart" -> node.optJSONObject("chart")?.let { chart(canvas, it, bounds, palette) }
            "image", "icon" -> {
                val image = node.optJSONObject("image")
                val encoded = image?.optString("png")
                if (!encoded.isNullOrEmpty() && encoded.length <= 43692) {
                    try {
                        val bytes = Base64.decode(encoded, Base64.DEFAULT)
                        val probe = BitmapFactory.Options().apply { inJustDecodeBounds = true }; BitmapFactory.decodeByteArray(bytes, 0, bytes.size, probe)
                        if (probe.outWidth in 1..1024 && probe.outHeight in 1..1024) {
                            val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
                            val scale = min(bounds.width() / bitmap.width, bounds.height() / bitmap.height)
                            canvas.drawBitmap(bitmap, null, RectF(bounds.left, bounds.top, bounds.left + bitmap.width * scale, bounds.top + bitmap.height * scale), paint)
                            bitmap.recycle()
                        }
                    } catch (_: Exception) { }
                } else text(canvas, image?.optString("text") ?: node.optString("text"), bounds.left, bounds.top + min(28f, bounds.height()), bounds.width(), 24f)
            }
            "progress" -> {
                val progress = node.optDouble("progress", 0.0).takeIf { it.isFinite() }?.coerceIn(0.0, 1.0)?.toFloat() ?: 0f
                paint.color = 0xff41414a.toInt(); val bar = RectF(bounds.left, bounds.centerY(), bounds.right, bounds.centerY() + 10); canvas.drawRoundRect(bar, 5f, 5f, paint)
                bar.right = bar.left + bar.width() * progress; paint.color = palette[0]; canvas.drawRoundRect(bar, 5f, 5f, paint)
                text(canvas, node.optString("value"), bounds.left, bounds.centerY() - 5, bounds.width(), 13f)
            }
            "table" -> node.optJSONObject("table")?.let { table ->
                val columns = table.optJSONArray("columns"); val rows = table.optJSONArray("rows")
                val lines = mutableListOf<String>()
                if (columns != null) lines.add((0 until min(6, columns.length())).joinToString("  |  ") { columns.optString(it) })
                if (rows != null) for (index in 0 until min(8, rows.length())) {
                    val row = rows.optJSONArray(index) ?: continue
                    lines.add((0 until min(6, row.length())).joinToString("  |  ") { row.optString(it) })
                }
                lines.take(max(1, (bounds.height() / 18).toInt())).forEachIndexed { index, line -> text(canvas, line, bounds.left, bounds.top + 16 + index * 18, bounds.width(), 13f, bold = index == 0) }
            }
            "divider" -> { paint.color = 0xff555560.toInt(); canvas.drawLine(bounds.left, bounds.centerY(), bounds.right, bounds.centerY(), paint) }
            "spacer" -> Unit
            else -> {
                val size = when (node.optString("role")) { "title" -> 24f; "headline" -> 20f; "caption" -> 12f; else -> 16f }
                val color = when (node.optString("tone")) { "muted" -> 0xffbfc1c8.toInt(); "accent" -> palette[0]; "success" -> colors[2]; "danger" -> colors[5]; "warning" -> colors[4]; else -> Color.WHITE }
                text(canvas, node.optString("text", node.optString("value")), bounds.left, bounds.top + min(size + 3, bounds.height()), bounds.width(), min(size, bounds.height()), color, node.optString("role") in listOf("title", "headline"))
            }
        }
    }
}

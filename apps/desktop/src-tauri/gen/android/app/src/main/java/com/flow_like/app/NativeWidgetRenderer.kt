package com.flow_like.app

import android.content.Context
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.util.Base64
import android.util.TypedValue
import androidx.core.content.ContextCompat
import androidx.core.content.res.ResourcesCompat
import org.json.JSONObject
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/** Renders the bounded native projection, never arbitrary HTML, scripts, or network images. */
internal object NativeWidgetRenderer {
    /** Width and height are the available display size in dp. */
    fun render(context: Context, widget: JSONObject, width: Int, height: Int): Bitmap =
        Renderer(context).render(widget, width, height)

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

    private class Renderer(context: Context) {
        private val metrics = context.resources.displayMetrics
        private val regular = ResourcesCompat.getFont(context, R.font.flow_inter_regular)
        private val medium = ResourcesCompat.getFont(context, R.font.flow_inter_medium)
        private val foreground = ContextCompat.getColor(context, R.color.setup_text)
        private val muted = ContextCompat.getColor(context, R.color.setup_muted)
        private val surface = ContextCompat.getColor(context, R.color.setup_surface)
        private val card = ContextCompat.getColor(context, R.color.setup_background)
        private val track = ContextCompat.getColor(context, R.color.setup_track)
        private val border = ContextCompat.getColor(context, R.color.setup_border)
        private val night = (context.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES
        private val colors = if (night) intArrayOf(0xffff824e.toInt(), 0xff93baff.toInt(), 0xff62d6c4.toInt(), 0xffd0a6ff.toInt(), 0xffeac76d.toInt(), 0xffff8fa3.toInt())
            else intArrayOf(0xffc2410c.toInt(), 0xff1d4ed8.toInt(), 0xff0f766e.toInt(), 0xff7e22ce.toInt(), 0xff936100.toInt(), 0xffbe123c.toInt())
        private fun sp(size: Float) = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, size, metrics) / metrics.density
        private fun textPaint(size: Float, color: Int = foreground, bold: Boolean = false) = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            this.color = color; textSize = sp(size); typeface = if (bold) medium else regular
        }
        private fun text(canvas: Canvas, value: String, x: Float, y: Float, width: Float, size: Float = 16f, color: Int = foreground, bold: Boolean = false) {
            val paint = textPaint(size, color, bold)
            val clean = value.replace('\n', ' ')
            if (paint.measureText(clean) <= width) canvas.drawText(clean, x, y, paint)
            else {
                val ellipsis = paint.measureText("…")
                if (width < ellipsis) return
                val count = paint.breakText(clean, true, width - ellipsis, null)
                canvas.drawText(clean.take(count) + "…", x, y, paint)
            }
        }
        fun render(widget: JSONObject, width: Int, height: Int): Bitmap {
            val displayWidth = width.coerceIn(32, 1024).toFloat()
            val displayHeight = height.coerceIn(32, 640).toFloat()
            // Lay out in dp/sp, then rasterize at device density within a 2.1 MiB bitmap budget.
            val scale = min(metrics.density, min(960f / displayWidth, 560f / displayHeight))
            val bitmap = Bitmap.createBitmap((displayWidth * scale).roundToInt().coerceAtLeast(1),
                (displayHeight * scale).roundToInt().coerceAtLeast(1), Bitmap.Config.ARGB_8888)
            val canvas = Canvas(bitmap)
            canvas.scale(bitmap.width / displayWidth, bitmap.height / displayHeight)
            // Keep chart text legible if the launcher changes theme before the next widget refresh.
            canvas.drawColor(surface)
            val bounds = RectF(4f, 4f, displayWidth - 4f, displayHeight - 4f)
            val selected = when (widget.optString("accent")) { "blue" -> 1; "teal" -> 2; "purple" -> 3; else -> 0 }
            val palette = colors.copyOf().apply { this[0] = colors[selected]; this[selected] = colors[0] }
            if (widget.optString("kind") == "chart") widget.optJSONObject("chart")?.let { chart(canvas, it, bounds, palette) }
            else widget.optJSONObject("page")?.let { page(canvas, it, bounds, 0, intArrayOf(0), palette) }
            return bitmap
        }
        private fun chart(canvas: Canvas, chart: JSONObject, bounds: RectF, palette: IntArray) {
            val points = NativeSurfacePolicy.array(chart.optJSONArray("points")).take(120).filter { it.optDouble("value").isFinite() }
            val type = chart.optString("type")
            val paint = Paint(Paint.ANTI_ALIAS_FLAG)
            if (bounds.width() < sp(120f) || bounds.height() < sp(72f)) {
                text(canvas, "Open app to view chart", bounds.left, bounds.top + sp(14f), bounds.width(), 12f, muted)
                return
            }
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
                        paint.color = track; canvas.drawArc(arc, 180f, 180f, false, paint)
                        paint.color = palette[0]; canvas.drawArc(arc, 180f, progress * 180f, false, paint)
                        text(canvas, formatted, bounds.left + 25, bounds.top + diameter / 2 - 4, bounds.width() - 50, 34f, bold = true)
                        return
                    }
                    val bar = RectF(bounds.left, bounds.centerY() + 24f, bounds.right, bounds.centerY() + 40f)
                    paint.color = track; canvas.drawRoundRect(bar, 8f, 8f, paint)
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
                    val legendLine = sp(18f) + 6f
                    if (index < 6 && (index + 1) * legendLine <= bounds.height()) text(canvas,
                        point.optString("label") + " " + point.optString("formattedValue"), pie.right + 12,
                        bounds.top + sp(14f) + index * legendLine, bounds.right - pie.right - 12, 12f, paint.color)
                }
                if (type == "donut") { paint.color = surface; canvas.drawCircle(pie.centerX(), pie.centerY(), diameter * .28f, paint) }
                return
            }
            val series = points.map { it.optString("series") }.distinct().take(6)
            val labels = points.map { it.optString("label") }.distinct().take(30)
            val shown = points.filter { it.optString("series") in series && it.optString("label") in labels }
            val domain = NativeChartDomain.calculate(shown, type == "stacked")
            val tickSize = 12f
            val tickHeight = sp(tickSize) * 1.25f
            val maximumLabel = "%.2g".format(domain.maximum)
            val minimumLabel = "%.2g".format(domain.minimum)
            val tickPaint = textPaint(tickSize, muted)
            val gutter = min(bounds.width() * .35f,
                max(28f, max(tickPaint.measureText(maximumLabel), tickPaint.measureText(minimumLabel)) + 8f))
            val area = RectF(bounds.left + gutter, bounds.top + tickHeight, bounds.right - 4, bounds.bottom - tickHeight - 10)
            fun y(value: Double) = area.bottom - domain.fraction(value) * area.height()
            val baseline = y(0.0)
            paint.color = muted; canvas.drawLine(area.left, baseline, area.right, baseline, paint)
            val topTick = area.top + tickHeight * .3f
            val bottomTick = area.bottom
            text(canvas, maximumLabel, bounds.left, topTick, gutter - 6, tickSize, muted)
            if (domain.minimum < 0) text(canvas, minimumLabel, bounds.left, bottomTick, gutter - 6, tickSize, muted)
            val zeroTick = baseline.coerceIn(topTick, bottomTick)
            if (domain.maximum != 0.0 && zeroTick - topTick >= tickHeight &&
                (domain.minimum == 0.0 || bottomTick - zeroTick >= tickHeight)) {
                text(canvas, "0", bounds.left, zeroTick, gutter - 6, tickSize, muted)
            }
            if (type == "horizontal") {
                val rows = shown.take(8); val height = area.height() / max(1, rows.size)
                val left = area.left + area.width() * .32f; val width = area.width() * .67f
                val zero = left + domain.fraction(0.0) * width
                rows.forEachIndexed { index, point ->
                    val top = area.top + index * height
                    val labelEvery = max(1, kotlin.math.ceil((sp(14f) + 4) / height).toInt())
                    if (index % labelEvery == 0) text(canvas, point.optString("label"), area.left,
                        top + min(sp(12f), height), area.width() * .3f, 12f)
                    paint.color = palette[index % palette.size]
                    val endpoint = left + domain.fraction(point.optDouble("value")) * width
                    val inset = min(3f, height * .15f)
                    canvas.drawRect(min(zero, endpoint), top + inset, max(zero, endpoint),
                        top + inset + min(20f, max(1f, height - 2 * inset)), paint)
                }
                paint.color = muted; canvas.drawLine(zero, area.top, zero, area.bottom, paint)
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
            if (type != "horizontal") {
                val baselineText = bounds.bottom - tickPaint.fontMetrics.descent
                val labelWidth = if (labels.size > 1) area.width() * .45f else area.width()
                text(canvas, labels.firstOrNull() ?: "", area.left, baselineText, labelWidth, 12f, muted)
                if (labels.size > 1) {
                    val lastWidth = min(labelWidth, tickPaint.measureText(labels.last()))
                    text(canvas, labels.last(), area.right - lastWidth, baselineText, lastWidth, 12f, muted)
                }
            }
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
                    if (node.optString("kind") == "card") { paint.color = card; canvas.drawRoundRect(bounds, 12f, 12f, paint) }
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
                    } else text(canvas, image?.optString("text") ?: node.optString("text"), bounds.left, bounds.top + min(sp(28f), bounds.height()), bounds.width(), 24f)
                }
                "progress" -> {
                    val progress = node.optDouble("progress", 0.0).takeIf { it.isFinite() }?.coerceIn(0.0, 1.0)?.toFloat() ?: 0f
                    paint.color = track; val bar = RectF(bounds.left, bounds.centerY(), bounds.right, bounds.centerY() + 10); canvas.drawRoundRect(bar, 5f, 5f, paint)
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
                    val lineHeight = sp(18f)
                    lines.take((bounds.height() / lineHeight).toInt()).forEachIndexed { index, line ->
                        text(canvas, line, bounds.left, bounds.top + sp(14f) + index * lineHeight, bounds.width(), 13f, bold = index == 0)
                    }
                }
                "divider" -> { paint.color = border; canvas.drawLine(bounds.left, bounds.centerY(), bounds.right, bounds.centerY(), paint) }
                "spacer" -> Unit
                else -> {
                    val size = when (node.optString("role")) { "title" -> 24f; "headline" -> 20f; "caption" -> 12f; else -> 16f }
                    val color = when (node.optString("tone")) { "muted" -> muted; "accent" -> palette[0]; "success" -> colors[2]; "danger" -> colors[5]; "warning" -> colors[4]; else -> foreground }
                    if (bounds.height() >= sp(size)) text(canvas, node.optString("text", node.optString("value")),
                        bounds.left, bounds.top + sp(size), bounds.width(), size, color, node.optString("role") in listOf("title", "headline"))
                }
            }
        }
    }
}

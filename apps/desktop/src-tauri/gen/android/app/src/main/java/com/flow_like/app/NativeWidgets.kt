package com.flow_like.app

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.util.TypedValue
import android.view.View
import android.widget.RemoteViews
import org.json.JSONObject
import kotlin.math.max

internal object NativeWidgets {
    fun update(context: Context) {
        val manager = AppWidgetManager.getInstance(context)
        val snapshot = BotNativeStore(context).snapshot()
        val store = NativeSurfaceStore(context)
        for (id in manager.getAppWidgetIds(ComponentName(context, BotWidgetProvider::class.java))) {
            manager.updateAppWidget(id, createView(context, id, manager.getAppWidgetOptions(id), snapshot, store.get(id)))
        }
    }

    fun createView(context: Context, id: Int, options: Bundle, snapshot: JSONObject?, config: JSONObject?): RemoteViews {
        val view = RemoteViews(context.packageName, R.layout.bot_widget)
        val configure = PendingIntent.getActivity(context, id, Intent(context, NativeWidgetConfigureActivity::class.java)
            .setData(Uri.parse("flow-like://widget/$id"))
            .putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        view.setOnClickPendingIntent(R.id.bot_widget_configure, configure)
        view.removeAllViews(R.id.bot_widget_rows)
        view.setViewVisibility(R.id.bot_widget_image, View.GONE)
        if (options.getInt(AppWidgetManager.OPTION_APPWIDGET_MIN_HEIGHT, 180) < 180) {
            view.setInt(R.id.bot_widget_title, "setMaxLines", 1)
            view.setInt(R.id.bot_widget_status, "setMaxLines", 1)
        }
        if (snapshot == null || config == null || config.optString("scope") != snapshot.optString("scope")) {
            view.setTextViewText(R.id.bot_widget_title, "Flow-Like")
            view.setTextViewText(R.id.bot_widget_status, if (snapshot == null) "Open Flow-Like to refresh" else "Choose widget content")
            view.setOnClickPendingIntent(R.id.bot_widget_root, configure)
            return view
        }
        val scope = snapshot.getString("scope")
        val choice = NativeSurfaceChoice(config.getString("kind"), config.optString("id"), config.optString("title", "Flow-Like"))
        val rows = visibleRows(context, options)
        var title = choice.title
        var age = NativeSurfacePolicy.observedAge(snapshot)
        val launch = NativeSurfaceLauncher.intent(context, choice, scope)
        config.optString("path").takeIf { it.isNotBlank() }?.let { launch.putExtra("path", it) }
        view.setOnClickPendingIntent(R.id.bot_widget_root, PendingIntent.getActivity(context, id, launch,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
        val custom = if (choice.kind == "custom") NativeSurfacePolicy.custom(snapshot, choice.id) else null
        if (choice.kind == "custom") {
            if (custom != null) {
                title = custom.optString("title", title)
                age = NativeSurfacePolicy.observedAge(snapshot, custom)
                if (custom.optString("state") == "ready") {
                    if ((custom.optJSONArray("warnings")?.length() ?: 0) > 0 && age.isEmpty()) age = "Open app for complete content"
                    val width = options.getInt(AppWidgetManager.OPTION_APPWIDGET_MIN_WIDTH, 220) - 32
                    val height = contentHeight(context, options, age.isNotEmpty()).toInt()
                    if (width >= sp(context, 144f) && height >= sp(context, 88f)) {
                        val image = NativeWidgetRenderer.render(context, custom, width, height)
                        view.setImageViewBitmap(R.id.bot_widget_image, image)
                        view.setContentDescription(R.id.bot_widget_image, NativeWidgetRenderer.description(custom))
                        view.setViewVisibility(R.id.bot_widget_image, View.VISIBLE)
                    } else if (age.isEmpty()) {
                        age = if (custom.optString("kind") == "chart") "Open app to view chart" else "Open app to view content"
                    }
                } else age = custom.optString("message", "Open the app to refresh this widget")
            } else age = "This widget is unavailable. Choose it again."
        } else if (choice.kind == "flowpilot") {
            if (age.isEmpty()) age = "Tap to ask FlowPilot"
        } else if (choice.kind in NativeSurfacePolicy.sections) {
            val section = NativeSurfacePolicy.section(snapshot, choice.kind)
            title = section?.optString("title", title) ?: title
            age = if (section == null) age else NativeSurfacePolicy.observedAge(snapshot, section)
            val expired = section?.has("expiresAt") == true && BotNativeStore.expired(section.getString("expiresAt"))
            if (section?.optString("state") != "ready" || expired) age = "Open Flow-Like to refresh"
            else {
                val items = NativeSurfacePolicy.array(section.optJSONArray("items"))
                for (item in items.take(rows)) {
                    val row = RemoteViews(context.packageName, R.layout.native_widget_row)
                    row.setTextViewText(R.id.native_widget_row_title, item.optString("title", "Open"))
                    row.setTextViewText(R.id.native_widget_row_detail, listOf(item.optString("value"), item.optString("status"), item.optString("subtitle"))
                        .filter { it.isNotBlank() }.distinct().joinToString(" · ").take(180))
                    row.setOnClickPendingIntent(R.id.native_widget_row, NativeSurfaceLauncher.pending(context, choice, scope, item.optString("id")))
                    view.addView(R.id.bot_widget_rows, row)
                }
                if (items.isEmpty()) age = if (choice.kind == "flowpilot") "Tap to ask FlowPilot" else "Nothing to show yet"
                else if (rows == 0 && age.isEmpty()) age = "${items.size} ${if (items.size == 1) "item" else "items"} · Tap to open"
            }
        } else {
            val current = NativeSurfacePolicy.choices(snapshot).firstOrNull { it.kind == choice.kind && it.id == choice.id }
            if (current == null) age = "This entry is unavailable. Choose it again."
            else {
                title = current.title
                if (age.isEmpty()) age = when (choice.kind) { "event" -> "Tap to open this Event"; "app" -> config.optString("path").ifBlank { "Tap to open app" }; else -> "Tap to open bot" }
            }
        }
        view.setTextViewText(R.id.bot_widget_title, title.take(100))
        view.setTextViewText(R.id.bot_widget_status, age)
        view.setViewVisibility(R.id.bot_widget_status, if (age.isEmpty()) View.GONE else View.VISIBLE)
        return view
    }

    private fun sp(context: Context, size: Float): Float = TypedValue.applyDimension(
        TypedValue.COMPLEX_UNIT_SP, size, context.resources.displayMetrics) / context.resources.displayMetrics.density

    private fun contentHeight(context: Context, options: Bundle, hasStatus: Boolean): Float {
        val height = options.getInt(AppWidgetManager.OPTION_APPWIDGET_MIN_HEIGHT, 180)
        val lines = if (height < 180) 1 else 2
        val header = max(48f, sp(context, 16f) * 1.3f * lines)
        val footer = if (hasStatus) sp(context, 12f) * 1.3f * lines + 8 else 0f
        return height - 32 - header - footer
    }

    private fun visibleRows(context: Context, options: Bundle): Int {
        val row = max(56f, (sp(context, 14f) + sp(context, 12f)) * 1.3f + 19) + 6
        // Existing small widgets keep a readable header and status instead of clipping rows.
        return (contentHeight(context, options, hasStatus = true) / row).toInt().coerceIn(0, 6)
    }
}

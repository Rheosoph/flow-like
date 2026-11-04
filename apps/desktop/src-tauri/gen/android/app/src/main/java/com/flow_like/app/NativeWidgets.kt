package com.flow_like.app

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.view.View
import android.widget.RemoteViews
import org.json.JSONObject

internal object NativeWidgets {
    fun update(context: Context) {
        val manager = AppWidgetManager.getInstance(context)
        val snapshot = BotNativeStore(context).snapshot()
        val store = NativeSurfaceStore(context)
        for (id in manager.getAppWidgetIds(ComponentName(context, BotWidgetProvider::class.java))) {
            val view = RemoteViews(context.packageName, R.layout.bot_widget)
            val configure = PendingIntent.getActivity(context, id, Intent(context, NativeWidgetConfigureActivity::class.java)
                .setData(Uri.parse("flow-like://widget/$id"))
                .putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            view.setOnClickPendingIntent(R.id.bot_widget_configure, configure)
            view.removeAllViews(R.id.bot_widget_rows)
            view.setViewVisibility(R.id.bot_widget_image, View.GONE)
            val config = store.get(id)
            if (snapshot == null || config == null || config.optString("scope") != snapshot.optString("scope")) {
                view.setTextViewText(R.id.bot_widget_title, "Flow-Like")
                view.setTextViewText(R.id.bot_widget_status, if (snapshot == null) "Open Flow-Like to refresh" else "Choose widget content")
                view.setOnClickPendingIntent(R.id.bot_widget_root, configure)
                manager.updateAppWidget(id, view)
                continue
            }
            val scope = snapshot.getString("scope")
            val choice = NativeSurfaceChoice(config.getString("kind"), config.optString("id"), config.optString("title", "Flow-Like"))
            val options = manager.getAppWidgetOptions(id)
            val rows = ((options.getInt(AppWidgetManager.OPTION_APPWIDGET_MIN_HEIGHT, 180) - 70) / 42).coerceIn(1, 6)
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
                        val image = NativeWidgetRenderer.render(custom, 480, 280)
                        view.setImageViewBitmap(R.id.bot_widget_image, image)
                        view.setContentDescription(R.id.bot_widget_image, NativeWidgetRenderer.description(custom))
                        view.setViewVisibility(R.id.bot_widget_image, View.VISIBLE)
                        if ((custom.optJSONArray("warnings")?.length() ?: 0) > 0 && age.isEmpty()) age = "Open app for complete content"
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
            manager.updateAppWidget(id, view)
        }
    }
}

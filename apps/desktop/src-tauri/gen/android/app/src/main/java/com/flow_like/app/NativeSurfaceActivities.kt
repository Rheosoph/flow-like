package com.flow_like.app

import android.app.Activity
import android.app.AlertDialog
import android.app.KeyguardManager
import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.widget.LinearLayout
import android.widget.Toast
import org.json.JSONObject

/** All exported entry points reach current, scoped catalog data only after device unlock. */
abstract class NativeUnlockedActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (getSystemService(KeyguardManager::class.java).isDeviceLocked) {
            if (Build.VERSION.SDK_INT >= 26) getSystemService(KeyguardManager::class.java).requestDismissKeyguard(this,
                object : KeyguardManager.KeyguardDismissCallback() {
                    override fun onDismissSucceeded() { unlocked() }
                    override fun onDismissCancelled() { finish() }
                    override fun onDismissError() { finish() }
                })
            else {
                @Suppress("DEPRECATION")
                val unlock = getSystemService(KeyguardManager::class.java).createConfirmDeviceCredentialIntent("Unlock Flow-Like", "Unlock to open your workspace")
                if (unlock != null) startActivityForResult(unlock, 52) else finish()
            }
        } else unlocked()
    }
    @Deprecated("Android 7 credential flow")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == 52 && resultCode == RESULT_OK) unlocked() else finish()
    }
    protected abstract fun unlocked()
    protected fun unavailable() {
        Toast.makeText(this, "Open Flow-Like in this workspace to refresh the action.", Toast.LENGTH_LONG).show()
        finish()
    }
}

internal object NativeSurfaceLauncher {
    fun intent(context: Context, choice: NativeSurfaceChoice, scope: String, itemId: String? = null): Intent =
        Intent(context, NativeSurfaceActionActivity::class.java).putExtra("kind", choice.kind).putExtra("entity_id", choice.id)
            .putExtra("scope", scope).putExtra("item_id", itemId)
            .setData(Uri.parse("flow-like://surface/${NativeSurfacePolicy.key(scope, choice.kind, choice.id + (itemId ?: ""))}"))
    fun pending(context: Context, choice: NativeSurfaceChoice, scope: String, itemId: String? = null): PendingIntent =
        PendingIntent.getActivity(context, 0, intent(context, choice, scope, itemId), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    fun dispatch(activity: Activity, snapshot: JSONObject, action: JSONObject) {
        require(NativeSurfacePolicy.validAction(action, snapshot) || action.optString("kind") == "resume_page")
        BotNativeStore(activity).enqueue(action, snapshot.getString("scope"))
        activity.startActivity(Intent(activity, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP))
        activity.finish()
    }
    fun shareCurrentPage(activity: Activity) {
        val snapshot = BotNativeStore(activity).snapshot() ?: throw IllegalStateException()
        val page = snapshot.optJSONObject("activePage") ?: throw IllegalStateException()
        val url = page.optString("url")
        require(NativeSurfacePolicy.continuation(snapshot, url))
        val continuation = Uri.Builder().scheme("flow-like").authority("continue")
            .appendQueryParameter("scope", snapshot.getString("scope")).appendQueryParameter("url", url).build()
        activity.startActivity(Intent.createChooser(Intent(Intent.ACTION_SEND).setType("text/plain")
            .putExtra(Intent.EXTRA_SUBJECT, page.optString("title", "Continue in Flow-Like"))
            .putExtra(Intent.EXTRA_TEXT, continuation.toString()), "Continue on another device"))
    }
}

class NativeSurfaceActionActivity : NativeUnlockedActivity() {
    override fun unlocked() {
        try {
            val snapshot = BotNativeStore(this).snapshot() ?: return unavailable()
            if (snapshot.getString("scope") != intent.getStringExtra("scope")) return unavailable()
            val choice = NativeSurfaceChoice(intent.getStringExtra("kind") ?: "", intent.getStringExtra("entity_id") ?: "", "")
            val action = NativeSurfacePolicy.action(snapshot, choice, intent.getStringExtra("item_id")) ?: return unavailable()
            intent.getStringExtra("path")?.let { path ->
                require(choice.kind == "app" && NativeSurfacePolicy.validPath(path)); action.put("path", path)
            }
            NativeSurfaceLauncher.dispatch(this, snapshot, action)
        } catch (_: Exception) { unavailable() }
    }
}

class NativeWidgetConfigureActivity : NativeUnlockedActivity() {
    private var widgetId = AppWidgetManager.INVALID_APPWIDGET_ID
    override fun unlocked() {
        widgetId = intent.getIntExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID)
        setResult(RESULT_CANCELED, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, widgetId))
        try {
            val snapshot = BotNativeStore(this).snapshot() ?: return unavailable()
            val page = NativeUi.page(this, "Choose widget content", "Choose what appears on your home screen. You can change it from the widget later.")
            page.root.tag = "widget_configuration_page"
        val search = NativeUi.input(this, "Search content").apply {
                tag = "widget_content_search"
                setSingleLine()
            }
            NativeUi.add(page.content, search, top = 0)
            val count = NativeUi.text(this, "", size = 12f, muted = true)
            NativeUi.add(page.content, count)
            val choices = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
            NativeUi.add(page.content, choices, top = 0)
            val all = NativeSurfacePolicy.choices(snapshot)
            fun populate(filter: String) {
                choices.removeAllViews()
                val matches = all.filter { it.title.contains(filter, true) || it.kind.contains(filter, true) }
                count.text = if (matches.size == 1) "1 option" else "${matches.size} options"
                if (matches.isEmpty()) {
                    NativeUi.add(choices, NativeUi.text(this@NativeWidgetConfigureActivity, "No matching content", emphasized = true))
                    NativeUi.add(choices, NativeUi.text(this@NativeWidgetConfigureActivity, "Try another name or search for a bot, app, Event, or widget.", muted = true), top = 8)
                }
                for (choice in matches) {
                    val type = when (choice.kind) {
                        "bot" -> "Bot"
                        "app" -> "App"
                        "event" -> "Event"
                        "custom" -> "App widget"
                        else -> "Workspace view"
                    }
                    NativeUi.add(choices, NativeUi.button(this@NativeWidgetConfigureActivity, "${choice.title} · $type") {
                        if (choice.kind == "app") choosePath(snapshot, choice) else save(snapshot, choice)
                    })
                }
            }
            search.addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
                override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) { populate(s?.toString() ?: "") }
                override fun afterTextChanged(s: Editable?) {}
            })
            NativeUi.add(page.content, NativeUi.button(this, "Continue on another device") {
                try { NativeSurfaceLauncher.shareCurrentPage(this) }
                catch (_: Exception) { Toast.makeText(this, "Open a bot conversation or app page first.", Toast.LENGTH_LONG).show() }
            }, top = 24)
            NativeUi.add(page.content, NativeUi.button(this, "Cancel") { finish() })
            populate("")
        } catch (_: Exception) { unavailable() }
    }
    private fun choosePath(snapshot: JSONObject, choice: NativeSurfaceChoice) {
        val path = NativeUi.input(this, "/orders?status=open").apply {
            tag = "widget_app_path"
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
            contentDescription = "App path (optional)"
            setSingleLine()
        }
        val dialog = NativeUi.dialog(this).setTitle(choice.title)
            .setMessage("Open a specific page in this app, or leave the path empty to open its home page.")
            .setView(path).setPositiveButton("Use app", null).setNegativeButton("Cancel", null).create()
        dialog.setOnShowListener { dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener {
            val value = path.text.toString().trim()
            if (value.isEmpty() || NativeSurfacePolicy.validPath(value)) { dialog.dismiss(); save(snapshot, choice, value.takeIf { it.isNotEmpty() }) }
            else path.error = "Use a path inside the app, such as /orders?status=open"
        } }
        dialog.show()
    }
    private fun save(snapshot: JSONObject, choice: NativeSurfaceChoice, path: String? = null) {
        try {
            require(widgetId != AppWidgetManager.INVALID_APPWIDGET_ID)
            require(AppWidgetManager.getInstance(this).getAppWidgetInfo(widgetId)?.provider == android.content.ComponentName(this, BotWidgetProvider::class.java))
            require(BotNativeStore(this).snapshot()?.optString("scope") == snapshot.getString("scope"))
            NativeSurfaceStore(this).put(widgetId, snapshot.getString("scope"), choice, path)
            NativeWidgets.update(this)
            setResult(RESULT_OK, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, widgetId)); finish()
        } catch (_: Exception) { unavailable() }
    }
}

class NativeContinueActivity : NativeUnlockedActivity() {
    override fun unlocked() {
        try {
            val snapshot = BotNativeStore(this).snapshot() ?: return unavailable()
            val url = NativeSurfacePolicy.continuationRequest(snapshot, intent.data?.toString() ?: "") ?: return unavailable()
            NativeSurfaceLauncher.dispatch(this, snapshot, JSONObject().put("kind", "resume_page").put("url", url))
        } catch (_: Exception) { unavailable() }
    }
}

class NativeSearchActivity : NativeUnlockedActivity() {
    override fun unlocked() {
        try {
            val snapshot = BotNativeStore(this).snapshot() ?: return unavailable()
            val choices = NativeSurfacePolicy.choices(snapshot, true)
            val scope = snapshot.getString("scope")
            val key = intent.data?.getQueryParameter("key")
            if (key != null) {
                val choice = choices.firstOrNull { NativeSurfacePolicy.key(scope, it.kind, it.id) == key } ?: return unavailable()
                val action = NativeSurfacePolicy.action(snapshot, choice) ?: return unavailable()
                NativeSurfaceLauncher.dispatch(this, snapshot, action); return
            }
            val kind = intent.getStringExtra("selection_kind")
            val eligible = NativeSurfacePolicy.choices(snapshot).filter { it.kind in listOf("app", "bot", "event") && (kind == null || it.kind == kind) }
            val title = if (kind == "event") "Choose an Event" else if (kind == "bot") "Choose a bot" else "Open in Flow-Like"
            val page = NativeUi.page(this, title, "Choose from the current workspace.")
            page.root.tag = "native_search_page"
            val search = NativeUi.input(this, if (kind == "event") "Find an Event" else if (kind == "bot") "Find a bot" else "Find an app, bot, or Event").apply {
                tag = "native_search_input"
                setSingleLine()
            }
            NativeUi.add(page.content, search, top = 0)
            val results = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
            NativeUi.add(page.content, results, top = 0)
            fun populate(filter: String) {
                results.removeAllViews()
                val matches = eligible.filter { it.title.contains(filter, true) }
                if (matches.isEmpty()) {
                    NativeUi.add(results, NativeUi.text(this@NativeSearchActivity,
                        if (filter.isBlank()) "Nothing available yet" else "No matches found", emphasized = true))
                    NativeUi.add(results, NativeUi.text(this@NativeSearchActivity,
                        if (filter.isBlank()) "Open Flow-Like to review the bots, apps, and Events in this workspace." else "Try a different name.", muted = true), top = 8)
                }
                matches.forEach { choice ->
                    NativeUi.add(results, NativeUi.button(this@NativeSearchActivity, choice.title) {
                        try {
                            val latest = BotNativeStore(this@NativeSearchActivity).snapshot() ?: return@button unavailable()
                            require(latest.getString("scope") == scope)
                            val action = NativeSurfacePolicy.action(latest, choice) ?: return@button unavailable()
                            NativeSurfaceLauncher.dispatch(this@NativeSearchActivity, latest, action)
                        } catch (_: Exception) { unavailable() }
                    })
                }
            }
            search.addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
                override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) { populate(s?.toString() ?: "") }
                override fun afterTextChanged(s: Editable?) {}
            })
            NativeUi.add(page.content, NativeUi.button(this, "Continue on another device") {
                try {
                    NativeSurfaceLauncher.shareCurrentPage(this)
                    finish()
                } catch (_: Exception) { unavailable() }
            }, top = 24)
            NativeUi.add(page.content, NativeUi.button(this, "Cancel") { finish() })
            populate("")
        } catch (_: Exception) { unavailable() }
    }
}

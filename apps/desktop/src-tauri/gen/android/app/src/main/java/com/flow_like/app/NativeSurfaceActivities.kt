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
import android.text.TextWatcher
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
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
            val layout = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(28, 32, 28, 24) }
            layout.addView(TextView(this).apply { text = "Choose widget content"; textSize = 24f })
            val search = EditText(this).apply { hint = "Find a bot, app, Event, or widget"; setSingleLine() }
            layout.addView(search)
            val choices = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
            val scroll = ScrollView(this).apply { addView(choices) }
            layout.addView(scroll, LinearLayout.LayoutParams(-1, 0, 1f))
            val share = Button(this).apply { text = "Continue on another device"; setOnClickListener {
                try { NativeSurfaceLauncher.shareCurrentPage(this@NativeWidgetConfigureActivity) }
                catch (_: Exception) { Toast.makeText(context, "Open a bot conversation or app page first.", Toast.LENGTH_LONG).show() }
            } }
            layout.addView(share)
            val all = NativeSurfacePolicy.choices(snapshot)
            fun populate(filter: String) {
                choices.removeAllViews()
                for (choice in all.filter { it.title.contains(filter, true) || it.kind.contains(filter, true) }) {
                    choices.addView(Button(this).apply {
                        text = "${choice.title} · ${choice.kind.replace('_', ' ')}"
                        setOnClickListener { if (choice.kind == "app") choosePath(snapshot, choice) else save(snapshot, choice) }
                    })
                }
            }
            search.addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
                override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) { populate(s?.toString() ?: "") }
                override fun afterTextChanged(s: Editable?) {}
            })
            populate(""); setContentView(layout)
        } catch (_: Exception) { unavailable() }
    }
    private fun choosePath(snapshot: JSONObject, choice: NativeSurfaceChoice) {
        val path = EditText(this).apply { hint = "Optional path, for example /orders?status=open"; setSingleLine() }
        val dialog = AlertDialog.Builder(this).setTitle(choice.title).setView(path).setPositiveButton("Use app", null).setNegativeButton("Cancel", null).create()
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
            val items = eligible.map { it.title } + listOf("Continue on another device")
            AlertDialog.Builder(this).setTitle(if (kind == "event") "Choose an Event" else if (kind == "bot") "Choose a bot" else "Open in Flow-Like")
                .setItems(items.toTypedArray()) { _, index ->
                    try {
                        if (index == eligible.size) { NativeSurfaceLauncher.shareCurrentPage(this); finish() }
                        else {
                            val latest = BotNativeStore(this).snapshot() ?: return@setItems unavailable()
                            require(latest.getString("scope") == scope)
                            val action = NativeSurfacePolicy.action(latest, eligible[index]) ?: return@setItems unavailable()
                            NativeSurfaceLauncher.dispatch(this, latest, action)
                        }
                    } catch (_: Exception) { unavailable() }
                }.setNegativeButton("Cancel") { _, _ -> finish() }.setOnCancelListener { finish() }.show()
        } catch (_: Exception) { unavailable() }
    }
}

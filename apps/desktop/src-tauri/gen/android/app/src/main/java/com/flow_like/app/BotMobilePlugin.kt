package com.flow_like.app

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import android.provider.MediaStore
import android.content.ClipData
import android.speech.RecognizerIntent
import android.util.Base64
import android.webkit.WebView
import android.widget.Toast
import android.widget.EditText
import androidx.activity.result.ActivityResult
import androidx.core.content.FileProvider
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.ConcurrentHashMap

@TauriPlugin
class BotMobilePlugin(private val activity: Activity) : Plugin(activity) {
    private val store = BotNativeStore(activity)
    private val secure = BotSecureStore(activity)
    private val worker = Executors.newSingleThreadExecutor()
    private val modelWorker = Executors.newSingleThreadExecutor()
    private val captureWorker = Executors.newSingleThreadExecutor()
    private var view: WebView? = null
    private val cancelled = ConcurrentHashMap.newKeySet<String>()
    private val cameraFiles = ConcurrentHashMap<String, File>()

    override fun load(webView: WebView) {
        super.load(webView)
        view = webView
        BotCredentialBroker.init(activity)
        NativeGeofences.install(activity)
        BotAutomationSignals.cancelled = { id, scope ->
            val detail = JSONObject().put("id", id).put("scope", scope)
            activity.runOnUiThread {
                view?.evaluateJavascript("window.dispatchEvent(new CustomEvent('flow-like-native-action-cancel',{detail:$detail}));", null)
            }
        }
        handleIntent(activity.intent)
    }

    override fun onNewIntent(intent: Intent) { handleIntent(intent); ready() }

    private fun ready() = activity.runOnUiThread {
        view?.evaluateJavascript("window.dispatchEvent(new Event('flow-like-native-action'));", null)
    }

    private fun execute(invoke: Invoke, block: () -> JSONObject) {
        worker.execute {
            try { invoke.resolve(JSObject(block().toString())) }
            catch (_: Exception) { invoke.reject("The native bot operation could not complete. Unlock the device and retry.") }
        }
    }

    @Command fun capabilities(invoke: Invoke) = execute(invoke) {
        BotCapabilities.create(
            Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).resolveActivity(activity.packageManager) != null,
            BotDocumentActivity.scannerAvailable(activity), BotLocalAi.availability(activity),
            BuildConfig.BOT_APP_FUNCTIONS && android.os.Build.VERSION.SDK_INT >= 36)
            .put("privateClipboard", true).put("clipboard", NativeDeviceClipboard.capabilities())
            .put("localOcr", true).put("nativeLocation", true).put("geofences", true)
            .put("automationResults", true).put("quickSettings", true)
    }

    // These commands are called only by Rust. No frontend command exposes secret values.
    @Command fun secureGet(invoke: Invoke) = execute(invoke) {
        val value = secure.read("credential:" + invoke.getArgs().getString("key"))
        JSONObject().put("value", value?.let { Base64.encodeToString(it, Base64.NO_WRAP) } ?: JSONObject.NULL)
    }
    @Command fun secureSet(invoke: Invoke) = execute(invoke) {
        val args = invoke.getArgs()
        secure.write("credential:" + args.getString("key"), Base64.decode(args.getString("value"), Base64.NO_WRAP))
        JSONObject()
    }
    @Command fun secureDelete(invoke: Invoke) = execute(invoke) {
        secure.delete("credential:" + invoke.getArgs().getString("key")); JSONObject()
    }
    @Command fun publishSnapshot(invoke: Invoke) = execute(invoke) {
        store.publish(invoke.getArgs().getJSONObject("snapshot"))
        NativeGeofences.dispatch { NativeGeofences.reconcile(activity) }
        BotSurfaces.refresh(activity)
        BotStatusSync.schedule(activity)
        ready()
        JSONObject()
    }
    @Command fun clearSnapshot(invoke: Invoke) = execute(invoke) {
        store.clear(); BotSurfaces.clear(activity)
        NativeGeofences.clear(activity); NativeDeviceClipboard.clearPrivate(activity)
        JSONObject()
    }
    @Command fun pendingActions(invoke: Invoke) = execute(invoke) { JSONObject().put("actions", store.foregroundActions()) }
    @Command fun acknowledgeAction(invoke: Invoke) = execute(invoke) {
        val args = invoke.getArgs()
        store.acknowledge(args.getString("id"), args.getString("scope")); JSONObject()
    }
    @Command fun completeAction(invoke: Invoke) = execute(invoke) {
        store.complete(invoke.getArgs().getJSONObject("result")); JSONObject()
    }
    @Command fun sharedFilesDirectory(invoke: Invoke) = execute(invoke) {
        JSONObject().put("path", BotNativeStore.sharedDirectory(activity).absolutePath)
    }

    @Command fun automationCurrent(invoke: Invoke) = execute(invoke) {
        val args = invoke.getArgs()
        JSONObject().put("active", store.automationCurrent(args.getString("id"), args.getString("scope")))
    }
    @Command fun locationRequest(invoke: Invoke) {
        val args = invoke.getArgs()
        activity.runOnUiThread {
            NativeDeviceLocation.request(activity, args.getString("requestId"), args.getJSONObject("options")) {
                invoke.resolve(JSObject(it.toString()))
            }
        }
    }
    @Command fun locationCancel(invoke: Invoke) = execute(invoke) {
        NativeDeviceLocation.cancel(invoke.getArgs().getString("requestId")); JSONObject()
    }
    @Command fun geofencePermission(invoke: Invoke) {
        activity.runOnUiThread {
            NativeGeofencePermissions.request(activity, invoke.getArgs().getString("mode")) {
                invoke.resolve(JSObject(it.toString()))
            }
        }
    }
    @Command fun geofenceScope(invoke: Invoke) = execute(invoke) { JSONObject().put("scope", NativeGeofences.scope(activity)) }
    @Command fun geofenceSync(invoke: Invoke) = execute(invoke) { NativeGeofences.sync(activity, invoke.getArgs().getJSONObject("config")) }
    @Command fun geofenceEvents(invoke: Invoke) = execute(invoke) { JSONObject().put("events", NativeGeofences.events(activity)) }
    @Command fun geofenceAck(invoke: Invoke) = execute(invoke) { NativeGeofences.ack(activity, invoke.getArgs().getJSONArray("ids")); JSONObject() }
    @Command fun geofenceRuntimeState(invoke: Invoke) = execute(invoke) { NativeGeofences.runtimeState(activity) }
    @Command fun writeClipboard(invoke: Invoke) {
        activity.runOnUiThread { invoke.resolve(JSObject(NativeDeviceClipboard.write(activity, invoke.getArgs().getJSONObject("payload")).toString())) }
    }
    @Command fun readPrivateClipboard(invoke: Invoke) = execute(invoke) { NativeDeviceClipboard.readPrivate(activity) }

    @Command fun botRequest(invoke: Invoke) {
        try {
            UUID.fromString(invoke.getArgs().getString("requestId"))
            val options = invoke.getArgs().getJSONObject("options")
            require(store.snapshot()?.optString("scope") == options.getString("scope")) { "scope_changed" }
            when (options.getString("operation")) {
                "voice" -> captureVoice(invoke)
                "document" -> captureDocument(invoke)
                "photo" -> capturePhoto(invoke)
                "summarize", "extract" -> modelWorker.execute {
                    try {
                        val result = BotLocalAi.process(activity, options)
                        if (result.optBoolean("downloadable") && !options.optBoolean("downloadModel")) {
                            activity.runOnUiThread {
                                AlertDialog.Builder(activity).setTitle("Download the on-device model?")
                                    .setMessage("This feature needs a model download. Your text is processed on this device.")
                                    .setPositiveButton("Download") { _, _ -> modelWorker.execute {
                                        try {
                                            options.put("downloadModel", true)
                                            finishRequest(invoke, BotLocalAi.process(activity, options))
                                        } catch (_: Exception) { invoke.reject("unavailable: The model download or processing failed") }
                                    } }
                                    .setNegativeButton("Cancel") { _, _ -> finishRequest(invoke, JSONObject().put("cancelled", true)) }
                                    .setOnCancelListener { finishRequest(invoke, JSONObject().put("cancelled", true)) }.show()
                            }
                        } else finishRequest(invoke, result)
                    }
                    catch (_: Exception) { invoke.reject("unavailable: The on-device model could not process this text") }
                }
                else -> invoke.reject("unsupported: Unknown native bot operation")
            }
        } catch (_: Exception) { invoke.reject("invalid_input: The bot request is invalid or its account changed") }
    }

    @Command fun botCancelRequest(invoke: Invoke) {
        val id = invoke.getArgs().optString("requestId")
        if (cancelled.size > 128) cancelled.clear()
        cancelled.add(id)
        invoke.resolve(JSObject())
    }

    private fun finishRequest(invoke: Invoke, result: JSONObject) {
        val args = invoke.getArgs()
        val requestId = args.optString("requestId")
        val scope = args.optJSONObject("options")?.optString("scope")
        if (cancelled.remove(requestId) || scope != store.snapshot()?.optString("scope")) {
            result.optJSONArray("files")?.let { deleteFiles(it) }
            invoke.reject("cancelled: The request was cancelled or its account changed")
        } else invoke.resolve(JSObject(result.toString()))
    }

    @Command fun captureVoice(invoke: Invoke) {
        val intent = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
            .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            .putExtra(RecognizerIntent.EXTRA_PROMPT, "Ask your bot")
        if (intent.resolveActivity(activity.packageManager) == null) {
            invoke.reject("Speech recognition is unavailable on this device")
            return
        }
        startActivityForResult(invoke, intent, "voiceResult")
    }

    @ActivityCallback fun voiceResult(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode != Activity.RESULT_OK) { finishRequest(invoke, JSONObject().put("cancelled", true)); return }
        val text = result.data?.getStringArrayListExtra(RecognizerIntent.EXTRA_RESULTS)?.firstOrNull()
        val input = EditText(activity).apply { setText(text ?: ""); minLines = 2 }
        AlertDialog.Builder(activity).setTitle("Use this message?").setView(input)
            .setPositiveButton("Use message") { _, _ -> finishRequest(invoke, JSONObject().put("text", input.text.toString())) }
            .setNegativeButton("Cancel") { _, _ -> finishRequest(invoke, JSONObject().put("cancelled", true)) }
            .setOnCancelListener { finishRequest(invoke, JSONObject().put("cancelled", true)) }.show()
    }

    @Command fun captureDocument(invoke: Invoke) {
        if (invoke.getArgs().optJSONObject("options")?.optString("operation") == "document" && BotDocumentActivity.scannerAvailable(activity)) {
            AlertDialog.Builder(activity).setTitle("Add a document")
                .setItems(arrayOf("Scan a document", "Choose a document")) { _, which ->
                    if (which == 0) startActivityForResult(invoke, Intent(activity, BotDocumentActivity::class.java), "documentResult")
                    else chooseDocument(invoke)
                }.setOnCancelListener { finishRequest(invoke, JSONObject().put("cancelled", true)) }.show()
        } else chooseDocument(invoke)
    }

    private fun chooseDocument(invoke: Invoke) {
        startActivityForResult(invoke, Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = if (invoke.getArgs().optJSONObject("options")?.optString("operation") == "photo") "image/*" else "*/*"
            putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }, "documentResult")
    }

    private fun capturePhoto(invoke: Invoke) {
        if (Intent(MediaStore.ACTION_IMAGE_CAPTURE).resolveActivity(activity.packageManager) == null) {
            captureDocument(invoke)
            return
        }
        AlertDialog.Builder(activity).setTitle("Add a photo")
            .setItems(arrayOf("Take a photo", "Choose a photo")) { _, which ->
                if (which == 1) captureDocument(invoke) else {
                    try {
                        val id = invoke.getArgs().getString("requestId")
                        val directory = File(activity.cacheDir, "bot-camera").apply { mkdirs() }
                        directory.listFiles().orEmpty().filter { System.currentTimeMillis() - it.lastModified() > 3600000 }.forEach { it.delete() }
                        val file = File.createTempFile("capture-", ".jpg", directory)
                        cameraFiles[id] = file
                        val uri = FileProvider.getUriForFile(activity, activity.packageName + ".fileprovider", file)
                        val intent = Intent(MediaStore.ACTION_IMAGE_CAPTURE).putExtra(MediaStore.EXTRA_OUTPUT, uri)
                            .addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_READ_URI_PERMISSION)
                        intent.clipData = ClipData.newUri(activity.contentResolver, "Photo", uri)
                        startActivityForResult(invoke, intent, "photoResult")
                    } catch (_: Exception) { invoke.reject("unavailable: The camera could not be opened") }
                }
            }.setOnCancelListener { finishRequest(invoke, JSONObject().put("cancelled", true)) }.show()
    }

    @ActivityCallback fun photoResult(invoke: Invoke, result: ActivityResult) {
        val file = cameraFiles.remove(invoke.getArgs().getString("requestId"))
        if (result.resultCode != Activity.RESULT_OK || file == null) {
            file?.delete()
            finishRequest(invoke, JSONObject().put("cancelled", true))
            return
        }
        captureWorker.execute {
            try {
                require(file.length() in 1..20L * 1024 * 1024)
                val imported = File(BotNativeStore.sharedDirectory(activity), "${UUID.randomUUID()}-photo.jpg")
                file.copyTo(imported)
                val paths = JSONArray().put(imported.absolutePath)
                finishRequest(invoke, JSONObject().put("files", paths).put("text", NativeDeviceOcr.readFiles(activity, paths)))
            } catch (_: Exception) { invoke.reject("unavailable: The photo could not be imported") }
            finally { file.delete() }
        }
    }

    @ActivityCallback fun documentResult(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode != Activity.RESULT_OK) { finishRequest(invoke, JSONObject().put("cancelled", true)); return }
        captureWorker.execute {
            try {
                val paths = copyFiles(result.data)
                finishRequest(invoke, JSONObject().put("files", paths).put("text", NativeDeviceOcr.readFiles(activity, paths)))
            }
            catch (_: Exception) { invoke.reject("unavailable: The selected documents could not be read") }
        }
    }

    private fun copyFiles(intent: Intent?): JSONArray {
        val uris = mutableListOf<Uri>()
        intent?.data?.let { uris.add(it) }
        intent?.clipData?.let { clip -> for (index in 0 until clip.itemCount) clip.getItemAt(index).uri?.let { uris.add(it) } }
        @Suppress("DEPRECATION")
        if (intent?.action == Intent.ACTION_SEND_MULTIPLE) {
            intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)?.let { uris.addAll(it) }
        } else {
            @Suppress("DEPRECATION")
            intent?.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)?.let { uris.add(it) }
        }
        val unique = uris.distinct()
        require(unique.size <= 10) { "Share at most 10 files" }
        val copied = mutableListOf<File>()
        try {
            var total = 0L
            for (uri in unique) {
                require(uri.scheme == "content") { "Only content provider files may be shared" }
                val displayName = activity.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
                    if (it.moveToFirst()) it.getString(0) else null
                } ?: "document"
                val safeName = displayName.replace(Regex("[^\\p{L}\\p{N}._ -]"), "_").take(160).trim('.').ifEmpty { "document" }
                val file = File(BotNativeStore.sharedDirectory(activity), "${UUID.randomUUID()}-$safeName")
                copied.add(file)
                var size = 0L
                activity.contentResolver.openInputStream(uri).use { input ->
                    requireNotNull(input) { "Shared file is unavailable" }
                    file.outputStream().use { output ->
                        val buffer = ByteArray(8192)
                        while (true) {
                            val count = input.read(buffer)
                            if (count < 0) break
                            size += count; total += count
                            require(size <= 20 * 1024 * 1024 && total <= 40 * 1024 * 1024) { "Shared files exceed the size limit" }
                            output.write(buffer, 0, count)
                        }
                    }
                }
            }
            return JSONArray(copied.map { it.absolutePath })
        } catch (error: Exception) {
            copied.forEach { it.delete() }
            throw error
        }
    }

    private fun handleIntent(intent: Intent?) {
        if (intent == null) return
        if (intent.action == "com.flow_like.app.AUTOMATION_WAKE") { ready(); return }
        if (intent.action == BotSurfaces.OPEN_SHORTCUT) {
            val id = intent.getStringExtra("bot_id") ?: return
            worker.execute {
                try {
                    val snapshot = store.snapshot() ?: return@execute
                    if (snapshot.optString("scope") != intent.getStringExtra("scope")) return@execute
                    val bots = snapshot.optJSONArray("bots") ?: return@execute
                    for (index in 0 until bots.length()) {
                        val bot = bots.getJSONObject(index)
                        if (bot.optString("id") == id) {
                            store.enqueue(JSONObject().put("kind", "open_bot").put("botId", id), snapshot.getString("scope"))
                            ready(); break
                        }
                    }
                } catch (_: Exception) { }
            }
            return
        }
        if (intent.action != Intent.ACTION_SEND && intent.action != Intent.ACTION_SEND_MULTIPLE) return
        // Remove the launch payload after capturing it so activity recreation cannot send twice.
        if (activity.intent === intent) activity.intent = Intent(activity, MainActivity::class.java)
        worker.execute {
            try {
                val snapshot = store.snapshot() ?: throw IllegalStateException("Open your bot workspace before sharing")
                val scope = snapshot.getString("scope")
                val files = copyFiles(intent)
                val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()?.take(64000) ?: ""
                val action = JSONObject().put("kind", "ask_bot").put("text", text).put("files", files)
                val bots = snapshot.optJSONArray("bots") ?: JSONArray()
                val directId = intent.getStringExtra(Intent.EXTRA_SHORTCUT_ID)
                val direct = (0 until bots.length()).map { bots.getJSONObject(it) }.firstOrNull { "bot:${it.optString("id")}" == directId }
                if (direct != null) {
                    action.put("botId", direct.getString("id"))
                    store.enqueue(action, scope); ready()
                } else activity.runOnUiThread {
                    if (bots.length() == 0) {
                        Toast.makeText(activity, "Create a bot before sharing", Toast.LENGTH_LONG).show()
                        deleteFiles(files)
                    } else AlertDialog.Builder(activity).setTitle("Send to bot")
                        .setItems(Array(bots.length()) { bots.getJSONObject(it).getString("title") }) { _, which ->
                            worker.execute {
                                try {
                                    action.put("botId", bots.getJSONObject(which).getString("id"))
                                    store.enqueue(action, scope); ready()
                                } catch (_: Exception) { deleteFiles(files) }
                            }
                        }.setOnCancelListener { deleteFiles(files) }.show()
                }
            } catch (_: Exception) { activity.runOnUiThread { Toast.makeText(activity, "Open your bot workspace and share again", Toast.LENGTH_LONG).show() } }
        }
    }

    private fun deleteFiles(files: JSONArray) {
        for (index in 0 until files.length()) File(files.getString(index)).delete()
    }
}

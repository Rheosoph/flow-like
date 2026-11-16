package com.flow_like.app

import android.app.Activity
import android.app.KeyguardManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.View
import android.widget.LinearLayout
import android.widget.Toast
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

internal object BotAutomation {
    const val ACTION = "com.flow_like.app.AUTOMATE"
    const val WAKE = "com.flow_like.app.AUTOMATION_WAKE"
    fun intent(context: Context, action: JSONObject, responseMode: String = "text"): Intent =
        Intent(context, BotAutomationActivity::class.java).setAction(ACTION)
            .putExtra("action_json", action.toString()).putExtra("response_mode", responseMode)
            .putExtra("scope", BotNativeStore(context).snapshot()?.optString("scope"))
    fun pendingIntent(context: Context, action: JSONObject, responseMode: String = "text"): PendingIntent =
        PendingIntent.getActivity(context, UUID.randomUUID().hashCode(), intent(context, action, responseMode)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_MULTIPLE_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
}

/** A visible consent and result boundary for Android automation callers. */
class BotAutomationActivity : Activity() {
    private val handler = Handler(Looper.getMainLooper())
    private lateinit var store: BotNativeStore
    private lateinit var scope: String
    private var requestId: String? = null
    private var returned = false
    private var destroyed = false
    private lateinit var page: NativePage
    private lateinit var controls: LinearLayout
    private val poll = object : Runnable {
        override fun run() {
            val id = requestId ?: return
            if (destroyed || returned) return
            if (store.snapshot()?.optString("scope") != scope) {
                finishOutcome(JSONObject().put("status","error").put("error","The active workspace changed.")); return
            }
            val result = store.automationResult(id, scope)
            if (result != null) { finishOutcome(result); return }
            if (!store.automationCurrent(id, scope)) {
                store.cancelAutomation(id,scope,"The automation timed out after 90 seconds.")
                finishOutcome(store.automationResult(id,scope) ?: JSONObject().put("status","error").put("error","The automation timed out.")); return
            }
            handler.postDelayed(this,300)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        store = BotNativeStore(this)
        page = NativeUi.page(this, "Run an automation", "Choose an action, then review it before running.")
        page.root.tag = "automation_page"
        controls = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        NativeUi.add(page.content, controls, top = 0)
        if (getSystemService(KeyguardManager::class.java).isDeviceLocked) {
            showScreen("Unlock your device", "Unlock the device, then open this automation again.")
            addButton("Close") { finish() }; return
        }
        val snapshot = store.snapshot()
        if (snapshot == null) {
            showScreen("Choose a workspace first", "Open Flow-Like and select the workspace for this automation.")
            addButton("Open Flow-Like", primary = true) { startActivity(Intent(this, MainActivity::class.java)); finish() }
            addButton("Close") { finish() }; return
        }
        scope = snapshot.getString("scope")
        val expected = savedInstanceState?.getString("scope") ?: intent.getStringExtra("scope")
        if (expected != null && expected != scope) {
            showScreen("Workspace changed", "This action belongs to another workspace. Open it there to continue.")
            addButton("Close") { finish() }; return
        }
        requestId = savedInstanceState?.getString("request_id")
        if (requestId != null) { showWaiting(); handler.post(poll); return }
        val supplied = intent.getStringExtra("action_json")
        if (supplied != null) {
            try {
                require(supplied.toByteArray().size <= 128 * 1024)
                showInput(snapshot, JSONObject(supplied), intent.getStringExtra("response_mode") ?: "text")
            } catch (_: Exception) {
                showScreen("Cannot open this automation", "This automation request is invalid.")
                addButton("Close") { finish() }
            }
        } else showChoices(snapshot)
    }

    private fun entries(snapshot: JSONObject, key: String): List<JSONObject> {
        val entries = snapshot.optJSONArray(key) ?: JSONArray()
        return (0 until entries.length()).map { entries.getJSONObject(it) }
    }

    private fun showChoices(snapshot: JSONObject) {
        showScreen("Run an automation", "Choose an action. You can review its input before running it.")
        showWorkspace()
        section("Assistant")
        addButton("Ask FlowPilot") { showInput(snapshot,JSONObject().put("kind","flowpilot"),"text") }
        val bots = entries(snapshot,"bots").filter { it.optBoolean("enabled",true) }
        if (bots.isNotEmpty()) section("Bots")
        bots.forEach { bot ->
            addButton("Ask ${bot.optString("title","Bot")}") { showInput(snapshot,JSONObject().put("kind","ask_bot").put("botId",bot.getString("id")),"text") }
        }
        val tasks = entries(snapshot,"botTasks").filter { it.optBoolean("enabled") }
        if (tasks.isNotEmpty()) section("Bot tasks")
        tasks.forEach { task ->
            addButton("Run task: ${task.optString("title","Task")}") { showInput(snapshot,JSONObject().put("kind","run_bot_task").put("botId",task.getString("botId")).put("taskId",task.getString("id")),"text") }
        }
        val events = entries(snapshot,"events").filter { event ->
            val surfaces = event.optJSONArray("surfaces") ?: JSONArray()
            (0 until surfaces.length()).any { surfaces.optString(it) in listOf("siri","shortcuts") }
        }
        if (events.isNotEmpty()) section("Events")
        events.forEach { event ->
            val action = event.optJSONObject("action") ?: return@forEach
            addButton("${if(event.optString("eventType")=="simple_chat") "Ask app" else "Run Event"}: ${event.optString("title")}") { showInput(snapshot,JSONObject(action.toString()),"text") }
            if (event.optString("eventType") != "simple_chat") addButton("Event JSON: ${event.optString("title")}") { showInput(snapshot,JSONObject(action.toString()),"result") }
        }
        addButton("Cancel") { finish() }
    }

    private fun showInput(snapshot: JSONObject, action: JSONObject, mode: String) {
        val title = when(action.optString("kind")) {
            "ask_bot", "run_bot_task" -> entries(snapshot,"bots").firstOrNull { it.optString("id")==action.optString("botId") }?.optString("title") ?: "Bot"
            "flowpilot" -> "FlowPilot"
            else -> entries(snapshot,"events").firstOrNull { it.optString("eventId")==action.optString("eventId") && it.optString("appId")==action.optString("appId") }?.optString("title") ?: "Event"
        }
        val isTask = action.optString("kind") == "run_bot_task"
        showScreen(title, if (isTask) "Review this task before running it." else "Review the message before sending it to this automation.")
        showWorkspace()
        val input = NativeUi.input(this, "Question or input",
            text = action.optString("prompt").ifBlank { action.optString("text") }, minLines = 3).apply {
            tag = "automation_input"
        }
        if (!isTask) {
            NativeUi.add(controls, NativeUi.text(this, "Question or input", size = 14f, emphasized = true), top = 20)
            NativeUi.add(controls, input, top = 8)
        } else {
            val taskTitle = entries(snapshot, "botTasks").firstOrNull {
                it.optString("botId") == action.optString("botId") && it.optString("id") == action.optString("taskId")
            }?.optString("title")
            if (!taskTitle.isNullOrBlank()) NativeUi.add(controls, NativeUi.text(this, taskTitle, emphasized = true), top = 20)
        }
        addButton(if (isTask) "Run task" else "Run and return ${if(mode=="result") "Text / JSON" else "text"}", primary = true) {
            try {
                if (action.optString("kind") != "run_bot_task") action.put("text",input.text.toString()).remove("prompt")
                val request=store.enqueueAutomation(action,scope,mode)
                requestId=request.getString("id")
                showWaiting()
                startActivity(Intent(this,MainActivity::class.java).setAction(BotAutomation.WAKE)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_REORDER_TO_FRONT))
                handler.post(poll)
            } catch (error: Exception) { Toast.makeText(this,error.message ?: "The automation could not start",Toast.LENGTH_LONG).show() }
        }
        addButton("Cancel") { finish() }
    }

    private fun showWaiting() {
        showScreen("Waiting for Flow-Like", "This request expires after 90 seconds. You can cancel it while waiting.")
        showWorkspace()
        addButton("Cancel automation") { cancelAndFinish() }
    }

    private fun showScreen(title: String, description: String) {
        page.heading.text = title
        page.description.text = description
        page.description.visibility = View.VISIBLE
        controls.removeAllViews()
        page.root.scrollTo(0, 0)
    }

    private fun showWorkspace() {
        NativeUi.add(controls, NativeUi.text(this, "Workspace", size = 12f, muted = true), top = 0)
        NativeUi.add(controls, NativeUi.text(this, JSONArray(scope).optString(0), size = 14f, emphasized = true), top = 4)
        callingPackage?.let {
            NativeUi.add(controls, NativeUi.text(this, "The result will be returned to $it.", size = 14f, muted = true))
        }
    }

    private fun section(title: String) = NativeUi.add(controls,
        NativeUi.text(this, title, size = 14f, emphasized = true), top = 24)

    private fun addButton(title: String, primary: Boolean = false, onClick: () -> Unit) =
        NativeUi.add(controls, NativeUi.button(this, title, primary = primary, onClick = onClick))
    private fun finishOutcome(result: JSONObject) {
        if (returned) return
        returned=true
        handler.removeCallbacks(poll)
        val data=Intent().putExtra("status",result.optString("status"))
        for(key in listOf("text","json","error")) if(result.has(key)) data.putExtra(key,result.optString(key))
        setResult(if(result.optString("status")=="success") RESULT_OK else RESULT_CANCELED,data)
        if(callingPackage!=null) { requestId?.let { store.automationResult(it,scope,true) }; finish(); return }
        val success = result.optString("status") == "success"
        showScreen(if (success) "Automation finished" else "Automation stopped",
            if (success) "Your result is ready." else "The automation could not finish.")
        val response = if (success) result.optString("text").ifBlank { result.optString("json") }
            .ifBlank { "Completed without a text response." }
            else result.optString("error", "This request needs attention in Flow-Like.")
        NativeUi.add(controls, NativeUi.text(this, response).apply {
            tag = "automation_result"
            setTextIsSelectable(true)
            accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        }, top = 0)
        addButton("Close") { requestId?.let { store.automationResult(it,scope,true) }; finish() }
    }
    private fun cancelAndFinish() {
        requestId?.let { store.cancelAutomation(it,scope) }
        finishOutcome(JSONObject().put("status","error").put("error","The automation was cancelled."))
    }
    @Deprecated("Deprecated in Java") override fun onBackPressed() { if(requestId!=null&&!returned) cancelAndFinish() else super.onBackPressed() }
    override fun onSaveInstanceState(outState: Bundle) { super.onSaveInstanceState(outState); if(::scope.isInitialized) outState.putString("scope",scope); outState.putString("request_id",requestId) }
    override fun onDestroy() { destroyed=true; handler.removeCallbacks(poll); if(isFinishing && !returned && requestId!=null && ::scope.isInitialized) store.cancelAutomation(requestId!!,scope); super.onDestroy() }
}

package com.flow_like.app

import android.app.Activity
import android.app.KeyguardManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.widget.*
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
    private lateinit var status: TextView
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
        controls = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(32,32,32,32) }
        status = TextView(this).apply { textSize = 18f }
        controls.addView(status)
        setContentView(ScrollView(this).apply { addView(controls) })
        if (getSystemService(KeyguardManager::class.java).isDeviceLocked) {
            status.text = "Unlock the device to run an automation."
            addButton("Close") { finish() }; return
        }
        val snapshot = store.snapshot()
        if (snapshot == null) { status.text = "Open Flow Like and select a workspace first."; addButton("Open Flow Like") { startActivity(Intent(this,MainActivity::class.java)); finish() }; return }
        scope = snapshot.getString("scope")
        val expected = savedInstanceState?.getString("scope") ?: intent.getStringExtra("scope")
        if (expected != null && expected != scope) { status.text = "This action belongs to another workspace."; addButton("Close") { finish() }; return }
        requestId = savedInstanceState?.getString("request_id")
        if (requestId != null) { showWaiting(); handler.post(poll); return }
        status.text = "Run in this workspace\n${JSONArray(scope).optString(0)}"
        callingPackage?.let { controls.addView(TextView(this).apply { text = "The result will be returned to $it." }) }
        val supplied = intent.getStringExtra("action_json")
        if (supplied != null) {
            try {
                require(supplied.toByteArray().size <= 128 * 1024)
                showInput(snapshot, JSONObject(supplied), intent.getStringExtra("response_mode") ?: "text")
            } catch (_: Exception) { status.text = "This automation request is invalid."; addButton("Close") { finish() } }
        } else showChoices(snapshot)
    }

    private fun entries(snapshot: JSONObject, key: String): List<JSONObject> {
        val entries = snapshot.optJSONArray(key) ?: JSONArray()
        return (0 until entries.length()).map { entries.getJSONObject(it) }
    }

    private fun showChoices(snapshot: JSONObject) {
        addButton("Ask FlowPilot") { showInput(snapshot,JSONObject().put("kind","flowpilot"),"text") }
        entries(snapshot,"bots").filter { it.optBoolean("enabled",true) }.forEach { bot ->
            addButton("Ask ${bot.optString("title","Bot")}") { showInput(snapshot,JSONObject().put("kind","ask_bot").put("botId",bot.getString("id")),"text") }
        }
        entries(snapshot,"botTasks").filter { it.optBoolean("enabled") }.forEach { task ->
            addButton("Run task: ${task.optString("title","Task")}") { showInput(snapshot,JSONObject().put("kind","run_bot_task").put("botId",task.getString("botId")).put("taskId",task.getString("id")),"text") }
        }
        entries(snapshot,"events").filter { event ->
            val surfaces = event.optJSONArray("surfaces") ?: JSONArray()
            (0 until surfaces.length()).any { surfaces.optString(it) in listOf("siri","shortcuts") }
        }.forEach { event ->
            val action = event.optJSONObject("action") ?: return@forEach
            addButton("${if(event.optString("eventType")=="simple_chat") "Ask app" else "Run Event"}: ${event.optString("title")}") { showInput(snapshot,JSONObject(action.toString()),"text") }
            if (event.optString("eventType") != "simple_chat") addButton("Event JSON: ${event.optString("title")}") { showInput(snapshot,JSONObject(action.toString()),"result") }
        }
    }

    private fun showInput(snapshot: JSONObject, action: JSONObject, mode: String) {
        while (controls.childCount > 1) controls.removeViewAt(1)
        val title = when(action.optString("kind")) {
            "ask_bot", "run_bot_task" -> entries(snapshot,"bots").firstOrNull { it.optString("id")==action.optString("botId") }?.optString("title") ?: "Bot"
            "flowpilot" -> "FlowPilot"
            else -> entries(snapshot,"events").firstOrNull { it.optString("eventId")==action.optString("eventId") && it.optString("appId")==action.optString("appId") }?.optString("title") ?: "Event"
        }
        status.text = "$title\n${JSONArray(scope).optString(0)}"
        callingPackage?.let { controls.addView(TextView(this).apply { text = "Return the result to $it." }) }
        val input = EditText(this).apply { minLines=3; hint="Question or input"; setText(action.optString("prompt").ifBlank { action.optString("text") }) }
        if (action.optString("kind") != "run_bot_task") controls.addView(input)
        addButton(if(action.optString("kind")=="run_bot_task") "Run task" else "Run and return ${if(mode=="result") "Text / JSON" else "text"}") {
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
        while(controls.childCount>1) controls.removeViewAt(1)
        status.text="Waiting for Flow Like. This request expires after 90 seconds."
        addButton("Cancel automation") { cancelAndFinish() }
    }

    private fun addButton(title: String, onClick: ()->Unit) = controls.addView(Button(this).apply { text=title; setOnClickListener { onClick() } })
    private fun finishOutcome(result: JSONObject) {
        if (returned) return
        returned=true
        handler.removeCallbacks(poll)
        val data=Intent().putExtra("status",result.optString("status"))
        for(key in listOf("text","json","error")) if(result.has(key)) data.putExtra(key,result.optString(key))
        setResult(if(result.optString("status")=="success") RESULT_OK else RESULT_CANCELED,data)
        if(callingPackage!=null) { requestId?.let { store.automationResult(it,scope,true) }; finish(); return }
        while(controls.childCount>1) controls.removeViewAt(1)
        status.text=when(result.optString("status")) { "success" -> result.optString("text").ifBlank { result.optString("json") }; else -> result.optString("error","This request needs attention in Flow Like.") }
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

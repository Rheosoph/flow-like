package com.flow_like.app

import android.app.KeyguardManager
import android.app.PendingIntent
import androidx.annotation.RequiresApi
import androidx.appfunctions.AppFunctionDeclaration
import androidx.appfunctions.AppFunctionElementNotFoundException
import androidx.appfunctions.AppFunctionInvalidArgumentException
import androidx.appfunctions.AppFunctionSerializable
import androidx.appfunctions.AppFunctionService
import androidx.appfunctions.AppFunctionServiceEntryPoint
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** A bot available in the active Flow-Like workspace. */
@AppFunctionSerializable(isDescribedByKDoc = true)
data class BotFunctionEntity(
    /** Stable bot identifier. */ val id: String,
    /** The bot's display name. */ val title: String,
)

/** The last published task status, with its observation time. */
@AppFunctionSerializable(isDescribedByKDoc = true)
data class BotFunctionTaskStatus(
    /** Stable task identifier. */ val id: String,
    /** Task display name. */ val title: String,
    /** Last observed status; unknown means no status was published. */ val status: String,
    /** Time of the cached observation in RFC 3339 format. */ val observedAt: String,
)

@RequiresApi(36)
@AppFunctionServiceEntryPoint(serviceName = "BotAppFunctionService", appFunctionXmlFileName = "bot_app_functions")
abstract class BotAppFunctions : AppFunctionService() {
    private fun snapshot(): JSONObject {
        if (getSystemService(KeyguardManager::class.java).isDeviceLocked)
            throw AppFunctionInvalidArgumentException("Unlock the device to use bots")
        return try {
            val snapshot = BotNativeStore(this).snapshot() ?: throw AppFunctionElementNotFoundException("Open Flow-Like to select a workspace")
            val scope = snapshot.getString("scope")
            if (BotNativeStore.subject(scope) != "local" && BotCredentialBroker.accessToken(this, scope) == null)
                throw AppFunctionElementNotFoundException("Open Flow-Like to refresh sign-in")
            snapshot
        } catch (error: androidx.appfunctions.AppFunctionException) { throw error }
        catch (_: Exception) { throw AppFunctionElementNotFoundException("The bot workspace is unavailable") }
    }

    private fun bot(snapshot: JSONObject, id: String): JSONObject {
        val bots = snapshot.optJSONArray("bots")
        for (index in 0 until (bots?.length() ?: 0)) {
            val value = bots!!.getJSONObject(index)
            if (value.optString("id") == id && !value.optBoolean("encrypted")) return value
        }
        throw AppFunctionElementNotFoundException("The bot is unavailable in the active workspace")
    }

    /** Lists the visible, unencrypted bots in the active workspace. */
    @AppFunctionDeclaration(isDescribedByKDoc = true)
    suspend fun listBots(): List<BotFunctionEntity> = withContext(Dispatchers.IO) {
        val bots = snapshot().optJSONArray("bots")
        (0 until (bots?.length() ?: 0)).map { bots!!.getJSONObject(it) }
            .filter { !it.optBoolean("encrypted") }
            .map { BotFunctionEntity(it.getString("id"), it.getString("title")) }
    }

    /**
     * Returns a screen-opening action for a bot in the active workspace.
     * @param botId Identifier returned by listBots.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true)
    suspend fun openBot(botId: String): PendingIntent = withContext(Dispatchers.IO) {
        val snapshot = snapshot()
        bot(snapshot, botId)
        val id = BotNativeStore(this@BotAppFunctions).rememberAction(
            JSONObject().put("kind", "open_bot").put("botId", botId), snapshot.getString("scope"))
        BotSurfaces.actionIntent(this@BotAppFunctions, id)
    }

    /**
     * Opens an editable message draft for a bot. The person reviews and sends it in Flow-Like.
     * @param botId Identifier returned by listBots.
     * @param message Text to place in the draft, up to 16000 characters.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true)
    suspend fun prepareBotMessage(botId: String, message: String): PendingIntent = withContext(Dispatchers.IO) {
        if (message.isBlank() || message.length > 16000) throw AppFunctionInvalidArgumentException("Use between 1 and 16000 characters")
        val snapshot = snapshot()
        bot(snapshot, botId)
        val id = BotNativeStore(this@BotAppFunctions).rememberAction(
            JSONObject().put("kind", "ask_bot").put("botId", botId).put("text", message).put("requiresReview", true), snapshot.getString("scope"))
        BotSurfaces.actionIntent(this@BotAppFunctions, id)
    }

    /**
     * Reads a task's cached status from the active workspace. It does not start the task.
     * @param botId Identifier returned by listBots.
     * @param taskId Identifier of the bot task.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true)
    suspend fun getBotTaskStatus(botId: String, taskId: String): BotFunctionTaskStatus = withContext(Dispatchers.IO) {
        val snapshot = snapshot()
        bot(snapshot, botId)
        val tasks = snapshot.optJSONArray("botTasks")
        val task = (0 until (tasks?.length() ?: 0)).map { tasks!!.getJSONObject(it) }
            .firstOrNull { it.optString("id") == taskId && it.optString("botId") == botId }
            ?: throw AppFunctionElementNotFoundException("The task is not in the current snapshot")
        BotFunctionTaskStatus(taskId, task.getString("title"), task.optString("status", "unknown"), snapshot.getString("generatedAt"))
    }

    private fun automation(action: JSONObject, mode: String): PendingIntent {
        val snapshot = snapshot()
        try { BotAutomationPolicy.validate(snapshot, action, mode) }
        catch (_: Exception) { throw AppFunctionInvalidArgumentException("The automation input or selected item is unavailable") }
        return BotAutomation.pendingIntent(this, action, mode)
    }

    /** Opens confirmation, runs the question in Flow Like, and displays the actual answer text. */
    @AppFunctionDeclaration(isDescribedByKDoc = true)
    suspend fun askBot(botId: String, question: String): PendingIntent = withContext(Dispatchers.IO) {
        automation(JSONObject().put("kind","ask_bot").put("botId",botId).put("prompt",question),"text")
    }

    /** Opens confirmation for an enabled task and returns its actual bot response in the result screen. */
    @AppFunctionDeclaration(isDescribedByKDoc = true)
    suspend fun runBotTask(botId: String, taskId: String): PendingIntent = withContext(Dispatchers.IO) {
        automation(JSONObject().put("kind","run_bot_task").put("botId",botId).put("taskId",taskId),"text")
    }

    private fun eventAction(appId: String, eventId: String, input: String, chatOnly: Boolean = false): JSONObject {
        val events = snapshot().optJSONArray("events")
        val event = (0 until (events?.length() ?: 0)).map { events!!.getJSONObject(it) }.firstOrNull {
            it.optString("appId") == appId && it.optString("eventId") == eventId && (!chatOnly || it.optString("eventType") == "simple_chat")
        } ?: throw AppFunctionElementNotFoundException("The Event is unavailable")
        return JSONObject(event.getJSONObject("action").toString()).put("text",input)
    }

    /** Opens confirmation for an exposed Event and displays the completed text result. */
    @AppFunctionDeclaration(isDescribedByKDoc = true)
    suspend fun runEventText(appId: String, eventId: String, input: String): PendingIntent = withContext(Dispatchers.IO) {
        automation(eventAction(appId,eventId,input),"text")
    }

    /** Opens confirmation for an exposed Event and displays its completed structured JSON result. */
    @AppFunctionDeclaration(isDescribedByKDoc = true)
    suspend fun runEventJson(appId: String, eventId: String, input: String): PendingIntent = withContext(Dispatchers.IO) {
        automation(eventAction(appId,eventId,input),"result")
    }

    /** Opens an exposed app chat, asks the confirmed question, and displays its answer text. */
    @AppFunctionDeclaration(isDescribedByKDoc = true)
    suspend fun askAppChat(appId: String, eventId: String, question: String): PendingIntent = withContext(Dispatchers.IO) {
        automation(eventAction(appId,eventId,question,true),"text")
    }

    /** Opens confirmation for FlowPilot and displays its completed answer text. */
    @AppFunctionDeclaration(isDescribedByKDoc = true)
    suspend fun askFlowPilot(question: String): PendingIntent = withContext(Dispatchers.IO) {
        automation(JSONObject().put("kind","flowpilot").put("prompt",question),"text")
    }
}

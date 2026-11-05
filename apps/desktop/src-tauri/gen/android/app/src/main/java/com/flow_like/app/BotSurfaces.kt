package com.flow_like.app

import android.app.Activity
import android.app.AlertDialog
import android.app.KeyguardManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.os.Build
import android.os.Bundle
import android.widget.EditText
import android.widget.Toast
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.Person
import androidx.core.app.RemoteInput
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat
import androidx.work.BackoffPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import androidx.work.Constraints
import androidx.work.NetworkType
import app.tauri.remotepush.PushNotificationPlugin
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import org.json.JSONObject
import java.util.concurrent.TimeUnit

internal object BotSurfaces {
    const val OPEN_SHORTCUT = "com.flow_like.app.OPEN_BOT"
    private const val CHANNEL = "bot_messages"
    private const val APPROVAL_CHANNEL = "bot_approvals"
    private const val DELIVERY_CHANNEL = "bot_pending_delivery"

    private fun channels(context: Context) {
        if (Build.VERSION.SDK_INT >= 26) {
            val manager = context.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannels(listOf(
                NotificationChannel(CHANNEL, "Bot messages", NotificationManager.IMPORTANCE_DEFAULT),
                NotificationChannel(APPROVAL_CHANNEL, "Bot approvals", NotificationManager.IMPORTANCE_HIGH),
                NotificationChannel(DELIVERY_CHANNEL, "Pending bot actions", NotificationManager.IMPORTANCE_LOW)))
        }
    }

    private fun openBot(context: Context, botId: String, scope: String) =
        Intent(context, MainActivity::class.java).setAction(OPEN_SHORTCUT).putExtra("bot_id", botId).putExtra("scope", scope)

    private fun icon(context: Context, avatar: Bitmap?) =
        avatar?.let { IconCompat.createWithBitmap(it) } ?: IconCompat.createWithResource(context, R.drawable.ic_notification)

    /** One shortcut per bot (`bot:<id>`): the share target, and the conversation its notifications belong to. */
    private fun botShortcut(context: Context, botId: String, title: String, scope: String, avatar: Bitmap?, person: Person? = null) =
        ShortcutInfoCompat.Builder(context, "bot:$botId")
            .setShortLabel(title.take(40))
            .setLongLived(true)
            .setCategories(setOf("com.flow_like.app.BOT_SHARE"))
            .setIcon(icon(context, avatar))
            .setIntent(openBot(context, botId, scope))
            .apply { if (person != null) setPerson(person) }
            .build()

    fun refresh(context: Context) {
        val snapshot = BotNativeStore(context).snapshot()
        val bots = snapshot?.optJSONArray("bots")
        val shortcuts = (0 until minOf(bots?.length() ?: 0, 4)).map { index ->
            val bot = bots!!.getJSONObject(index)
            val id = bot.getString("id")
            botShortcut(context, id, bot.getString("title"), snapshot.getString("scope"), BotAvatars.cached(context, id))
        }
        ShortcutManagerCompat.setDynamicShortcuts(context, shortcuts)
        updateWidgets(context)
        updateProgress(context, snapshot)
        NativeSearchIndex.refresh(context)
    }

    private fun updateProgress(context: Context, snapshot: JSONObject?) {
        channels(context)
        val manager = context.getSystemService(NotificationManager::class.java)
        val activeTags = mutableSetOf<String>()
        val sections = snapshot?.optJSONArray("sections")
        if (sections != null) for (index in 0 until sections.length()) {
            val section = sections.getJSONObject(index)
            if (section.optString("kind") != "bot_tasks") continue
            if (section.has("expiresAt") && BotNativeStore.expired(section.getString("expiresAt"))) continue
            val items = section.optJSONArray("items") ?: continue
            for (itemIndex in 0 until minOf(items.length(), 4)) {
                val item = items.getJSONObject(itemIndex)
                if (!item.optBoolean("live")) continue
                val tag = "flow-bot:progress:" + item.getString("id")
                activeTags.add(tag)
                val launch = NativeSurfaceLauncher.pending(context,
                    NativeSurfaceChoice("bot_tasks", title = "Bot tasks"), snapshot.getString("scope"), item.getString("id"))
                val progress = item.optDouble("progress", Double.NaN)
                val notification = NotificationCompat.Builder(context, CHANNEL).setSmallIcon(R.drawable.ic_notification)
                    .setContentTitle(item.optString("title", "Bot task"))
                    .setContentText(item.optString("status", "Working"))
                    .setProgress(100, if (progress.isFinite()) (progress.coerceIn(0.0, 1.0) * 100).toInt() else 0, !progress.isFinite())
                    .setOngoing(true).setOnlyAlertOnce(true).setSilent(true).setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
                    .setContentIntent(launch).build()
                try { manager.notify(tag, tag.hashCode(), notification) } catch (_: SecurityException) { }
            }
        }
        manager.activeNotifications.filter { it.tag?.startsWith("flow-bot:progress:") == true && it.tag !in activeTags }
            .forEach { manager.cancel(it.tag, it.id) }
    }

    fun clearProgress(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.activeNotifications.filter { it.tag?.startsWith("flow-bot:progress:") == true }
            .forEach { manager.cancel(it.tag, it.id) }
    }

    fun clear(context: Context) {
        val ids = ShortcutManagerCompat.getDynamicShortcuts(context).map { it.id }.filter { it.startsWith("bot:") }
        ShortcutManagerCompat.removeLongLivedShortcuts(context, ids)
        ShortcutManagerCompat.removeDynamicShortcuts(context, ids)
        WorkManager.getInstance(context).cancelUniqueWork("bot-pending-actions")
        BotStatusSync.cancel(context)
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.activeNotifications.filter { it.tag?.startsWith("flow-bot:") == true }.forEach { manager.cancel(it.tag, it.id) }
        BotAvatars.clear(context)
        NativeSurfaceStore(context).clear()
        NativeSearchIndex.clear(context)
        updateWidgets(context)
    }

    fun updateWidgets(context: Context) = NativeWidgets.update(context)

    fun actionIntent(context: Context, id: String, reply: Boolean = false): PendingIntent {
        val intent = Intent(context, BotActionActivity::class.java).setAction("bot-action:$id").putExtra("action_id", id)
        val mutability = if (reply && Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else if (reply) 0 else PendingIntent.FLAG_IMMUTABLE
        return PendingIntent.getActivity(context, 0, intent, PendingIntent.FLAG_UPDATE_CURRENT or mutability)
    }

    fun push(context: Context, data: Map<String, String>): Boolean {
        val category = data["category"] ?: return false
        if (category !in listOf("flow_like_bot_message", "flow_like_bot_approval", "flow_like_bot_review")) return false
        val store = BotNativeStore(context)
        val snapshot = store.snapshot() ?: return true
        val scope = snapshot.getString("scope")
        val subject = snapshot.optString("userId").ifBlank { BotNativeStore.subject(scope) }
        if (data["user_id"].isNullOrBlank() || data["user_id"] != subject) return true
        val botId = data["bot_id"]?.takeIf { it.isNotBlank() } ?: return true
        // A message to an owner whose chat is end-to-end encrypted has no conversation the hub could name.
        val conversationId = data["conversation_id"]?.takeIf { it.isNotBlank() }
        val expiration = data["expires_at"]
        if (expiration != null && BotNativeStore.expired(expiration)) return true
        channels(context)
        val approval = category == "flow_like_bot_approval"
        val scoped = data["native_scope"] == scope
        val open = if (scoped && conversationId != null) JSONObject().put("kind", "open_conversation").put("botId", botId).put("conversationId", conversationId)
            else JSONObject().put("kind", "open_home")
        val openId = store.rememberAction(open, scope, expiration)
        val sender = data["app_name"]?.takeIf { it.isNotBlank() } ?: data["title"]?.takeIf { it.isNotBlank() } ?: "Your bot"
        val group = data["thread_id"]?.takeIf { it.isNotBlank() } ?: "bot:$botId"
        val avatar = BotAvatars.load(context, botId, data["app_icon"])
        val bot = Person.Builder().setKey(group).setName(sender).setBot(true)
            .apply { if (avatar != null) setIcon(IconCompat.createWithBitmap(avatar)) }.build()
        val text = BotSenderPolicy.messageText(data["title"], data["body"], sender)
            .ifBlank { if (approval) "Your approval is needed" else "New message" }
        val channel = if (approval) APPROVAL_CHANNEL else CHANNEL
        val builder = NotificationCompat.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(sender)
            .setContentText(text)
            .setStyle(NotificationCompat.MessagingStyle(Person.Builder().setName("You").build())
                .addMessage(text, System.currentTimeMillis(), bot))
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setGroup(group)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setContentIntent(actionIntent(context, openId))
            .setAutoCancel(true)
            .setOnlyAlertOnce(true)
        if (avatar != null && Build.VERSION.SDK_INT < 28) builder.setLargeIcon(avatar)
        if (scoped) {
            try { ShortcutManagerCompat.pushDynamicShortcut(context, botShortcut(context, botId, sender, scope, avatar, bot)) }
            catch (_: Exception) { /* Without the shortcut the push still shows, just not as a conversation. */ }
            builder.setShortcutId("bot:$botId")
        }
        val background = scoped && data["conversation_encryption"] == "plain"
        if (scoped && conversationId != null && approval && !data["approval_id"].isNullOrBlank() && !data["turn_id"].isNullOrBlank()) {
            for ((label, decision) in listOf("Approve once" to "once", "Deny" to "deny")) {
                val action = JSONObject().put("kind", "respond_bot_approval").put("botId", botId)
                    .put("conversationId", conversationId).put("turnId", data["turn_id"])
                    .put("approvalId", data["approval_id"]).put("decision", decision)
                val id = store.rememberAction(action, scope, expiration, background)
                builder.addAction(NotificationCompat.Action.Builder(0, label, actionIntent(context, id))
                    .setAuthenticationRequired(true).build())
            }
        } else if (scoped && conversationId != null && category == "flow_like_bot_message") {
            val id = store.rememberAction(JSONObject().put("kind", "ask_bot").put("botId", botId).put("conversationId", conversationId), scope, expiration, background)
            builder.addAction(NotificationCompat.Action.Builder(0, "Reply", actionIntent(context, id, true))
                .addRemoteInput(RemoteInput.Builder("reply").setLabel("Reply to bot").build())
                .setAuthenticationRequired(true).setAllowGeneratedReplies(false).build())
        }
        val notificationId = data["message_id"] ?: data["approval_id"] ?: conversationId ?: data["notification_id"] ?: group
        try {
            NotificationManagerCompat.from(context).notify("flow-bot:$notificationId", notificationId.hashCode(), builder.build())
            stack(context, channel, group, sender, avatar, if (scoped) openBot(context, botId, scope) else Intent(context, MainActivity::class.java))
        } catch (_: SecurityException) { /* The inbox retains messages if notifications are disabled. */ }
        return true
    }

    /** The summary that bundles one bot's notifications, like a chat thread. */
    private fun stack(context: Context, channel: String, group: String, sender: String, avatar: Bitmap?, open: Intent) {
        val summary = NotificationCompat.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(sender)
            .setGroup(group)
            .setGroupSummary(true)
            .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_CHILDREN)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setContentIntent(PendingIntent.getActivity(context, group.hashCode(), open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
            .setAutoCancel(true)
            .apply { if (avatar != null) setLargeIcon(avatar) }
            .build()
        NotificationManagerCompat.from(context).notify("flow-bot:group:$group", group.hashCode(), summary)
    }

    fun scheduleDeliveryReminder(context: Context) {
        WorkManager.getInstance(context).enqueueUniqueWork("bot-pending-actions", ExistingWorkPolicy.APPEND_OR_REPLACE,
            OneTimeWorkRequestBuilder<BotPendingWorker>()
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS).build())
    }

    fun pendingReminder(context: Context) {
        channels(context)
        val intent = PendingIntent.getActivity(context, 2, Intent(context, MainActivity::class.java), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        try {
            NotificationManagerCompat.from(context).notify("flow-bot:pending", 2,
                NotificationCompat.Builder(context, DELIVERY_CHANNEL).setSmallIcon(R.drawable.ic_notification)
                    .setContentTitle("Bot action waiting to send").setContentText("Open Flow-Like to finish sending.")
                    .setContentIntent(intent).setAutoCancel(true).setOnlyAlertOnce(true).build())
        } catch (_: SecurityException) { }
    }

    fun deliveryFailure(context: Context) {
        channels(context)
        val intent = PendingIntent.getActivity(context, 3, Intent(context, MainActivity::class.java), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        try { NotificationManagerCompat.from(context).notify("flow-bot:delivery-error", 3,
            NotificationCompat.Builder(context, DELIVERY_CHANNEL).setSmallIcon(R.drawable.ic_notification)
                .setContentTitle("Bot action could not be sent").setContentText("Open Flow-Like to review the conversation.")
                .setContentIntent(intent).setAutoCancel(true).build()) } catch (_: SecurityException) { }
    }
}

class BotWidgetProvider : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        try { BotSurfaces.updateWidgets(context) } catch (_: Exception) { }
    }
    override fun onAppWidgetOptionsChanged(context: Context, manager: AppWidgetManager, id: Int, options: Bundle) {
        try { BotSurfaces.updateWidgets(context) } catch (_: Exception) { }
    }
    override fun onDeleted(context: Context, ids: IntArray) {
        val store = NativeSurfaceStore(context)
        store.delete(ids)
    }
}

/** The activity is private; every action is an opaque reference to encrypted, scoped state. */
class BotActionActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val keyguard = getSystemService(KeyguardManager::class.java)
        if (keyguard.isDeviceLocked) {
            if (Build.VERSION.SDK_INT >= 26) keyguard.requestDismissKeyguard(this, object : KeyguardManager.KeyguardDismissCallback() {
                override fun onDismissSucceeded() { dispatch() }
                override fun onDismissCancelled() { finish() }
                override fun onDismissError() { finish() }
            }) else {
                @Suppress("DEPRECATION")
                val unlock = keyguard.createConfirmDeviceCredentialIntent("Unlock Flow-Like", "Unlock to respond to your bot")
                if (unlock != null) startActivityForResult(unlock, 1) else finish()
            }
        } else dispatch()
    }

    @Deprecated("Legacy device credential flow for Android 7")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == 1 && resultCode == RESULT_OK) dispatch() else finish()
    }

    private fun dispatch() {
        try {
            if (getSystemService(KeyguardManager::class.java).isDeviceLocked) { finish(); return }
            val store = BotNativeStore(this)
            val id = intent.getStringExtra("action_id") ?: throw IllegalArgumentException()
            val value = store.resolveAction(id) ?: throw IllegalStateException()
            val action = value.getJSONObject("action")
            if (action.optBoolean("requiresReview")) {
                val input = EditText(this).apply {
                    setText(action.optString("text"))
                    minLines = 3
                    maxLines = 10
                    filters = arrayOf(android.text.InputFilter.LengthFilter(16000))
                }
                val dialog = AlertDialog.Builder(this).setTitle("Review bot message")
                    .setMessage("Review the message before sending it to your bot.")
                    .setView(input).setNegativeButton("Cancel") { _, _ -> finish() }
                    .setPositiveButton("Send", null).setOnCancelListener { finish() }.create()
                dialog.setOnShowListener {
                    dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener {
                        if (input.text.toString().isBlank()) { input.error = "Enter a message"; return@setOnClickListener }
                        try {
                            require(!getSystemService(KeyguardManager::class.java).isDeviceLocked)
                            val current = store.resolveAction(id) ?: throw IllegalStateException()
                            current.getJSONObject("action").remove("requiresReview")
                            current.getJSONObject("action").put("text", input.text.toString())
                            enqueue(store, id, current)
                        } catch (_: Exception) { expired() }
                        dialog.dismiss()
                        finish()
                    }
                }
                dialog.show()
                return
            }
            RemoteInput.getResultsFromIntent(intent)?.getCharSequence("reply")?.toString()?.let {
                require(it.isNotBlank() && it.length <= 64000)
                action.put("text", it)
            }
            enqueue(store, id, value)
        } catch (_: Exception) { expired() }
        finish()
    }

    private fun enqueue(store: BotNativeStore, id: String, value: JSONObject) {
        val background = value.optBoolean("backgroundDelivery")
        store.enqueue(value.getJSONObject("action"), value.getString("scope"), id, background, value.optString("expiresAt").takeIf { it.isNotBlank() })
        BotSurfaces.scheduleDeliveryReminder(this)
        if (!background) startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP))
    }

    private fun expired() = Toast.makeText(this, "This bot action expired. Open Flow-Like to review it.", Toast.LENGTH_LONG).show()
}

class BotMessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) { PushNotificationPlugin.dispatchNewToken(token) }
    override fun onMessageReceived(message: RemoteMessage) {
        try {
            if (!BotSurfaces.push(this, message.data)) PushNotificationPlugin.dispatchMessage(message)
        } catch (_: Exception) { /* A locked credential store must never disclose cached content. */ }
    }
}

class BotPendingWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result = try {
        val store = BotNativeStore(applicationContext)
        val pending = store.pending()
        var retry = false
        var foreground = false
        for (index in 0 until pending.length()) {
            if (isStopped) break
            val item = pending.getJSONObject(index)
            if (!item.optBoolean("backgroundDelivery")) continue
            when (BotDelivery(applicationContext).deliver(item)) {
                BotDelivery.Outcome.COMPLETE -> Unit
                BotDelivery.Outcome.RETRY -> retry = true
                BotDelivery.Outcome.FOREGROUND -> {
                    store.pauseDelivery(item.getString("id"), item.getString("scope"))
                    foreground = true
                }
            }
        }
        if (foreground) BotSurfaces.pendingReminder(applicationContext)
        if (!isStopped) try { BotStatusSync.refresh(applicationContext) } catch (_: Exception) { }
        if (retry) Result.retry() else Result.success()
    } catch (_: java.io.IOException) { Result.retry() }
      catch (_: Exception) {
          if (runAttemptCount < 5) Result.retry() else {
              try {
                  val store = BotNativeStore(applicationContext)
                  val pending = store.pending()
                  for (index in 0 until pending.length()) {
                      val item = pending.getJSONObject(index)
                      store.pauseDelivery(item.getString("id"), item.getString("scope"))
                  }
                  BotSurfaces.pendingReminder(applicationContext)
              } catch (_: Exception) { }
              Result.failure()
          }
      }
}

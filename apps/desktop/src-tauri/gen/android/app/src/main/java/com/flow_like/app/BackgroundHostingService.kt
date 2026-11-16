package com.flow_like.app

import android.annotation.SuppressLint
import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import android.provider.Settings
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import org.json.JSONObject
import java.util.UUID

/** A stale start or destroy callback must not change a newer hosting session. */
internal class BackgroundHostingSession {
    private var token: String? = null
    private var active = false

    @Synchronized fun request(): String = token ?: UUID.randomUUID().toString().also { token = it }
    @Synchronized fun accepts(value: String?): Boolean = value != null && value == token
    @Synchronized fun started(value: String): Boolean {
        if (!accepts(value)) return false
        active = true
        return true
    }
    @Synchronized fun stop(value: String? = token): Boolean {
        if (!accepts(value)) return false
        token = null
        active = false
        return true
    }
    @Synchronized fun state(): Pair<Boolean, Boolean> = Pair(token != null, active)
}

/** Keeps the existing app runtime available for a user-started session. */
class BackgroundHostingService : Service() {
    private var sessionToken: String? = null
    private var wakeLock: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    @SuppressLint("WakelockTimeout") // The explicit hosting session owns the lock until stop/destroy.
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == TURN_OFF) {
            stop(this)
            return START_NOT_STICKY
        }
        val token = intent?.getStringExtra(SESSION)
        if (token == null || !session.accepts(token)) {
            if (!session.state().first) stopSelf(startId)
            return START_NOT_STICKY
        }
        sessionToken = token
        try {
            check(notificationsEnabled(this)) {
                "Turn on Flow-Like notifications in Android settings before enabling background hosting."
            }
            createChannel(this)
            val open = PendingIntent.getActivity(this, 0,
                Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            val turnOff = PendingIntent.getService(this, 1,
                Intent(this, BackgroundHostingService::class.java).setAction(TURN_OFF),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            val notification = NotificationCompat.Builder(this, CHANNEL)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(getString(R.string.background_hosting_title))
                .setContentText(getString(R.string.background_hosting_description))
                .setContentIntent(open)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setSilent(true)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .setCategory(NotificationCompat.CATEGORY_SERVICE)
                .addAction(0, getString(R.string.background_hosting_turn_off), turnOff)
                .build()
            ServiceCompat.startForeground(this, NOTIFICATION, notification,
                if (Build.VERSION.SDK_INT >= 34) ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE else 0)
            if (wakeLock == null) {
                wakeLock = getSystemService(PowerManager::class.java)
                    .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "FlowLike:BackgroundHosting")
                    .apply { setReferenceCounted(false); acquire() }
            }
            if (session.started(token)) finishStart(this, token, null)
        } catch (error: Exception) {
            session.stop(token)
            finishStart(this, token, startFailure(error))
            releaseResources()
            stopSelf(startId)
        }
        return START_NOT_STICKY
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        stop(this)
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        val token = sessionToken
        if (token != null) {
            session.stop(token)
            finishStart(this, token, "Background hosting stopped. Open Flow-Like to turn it on again.")
        }
        releaseResources()
        super.onDestroy()
    }

    private fun releaseResources() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
    }

    companion object {
        private const val CHANNEL = "background_hosting"
        private const val NOTIFICATION = 42011
        private const val SESSION = "hosting_session"
        private const val TURN_OFF = "com.flow_like.app.TURN_OFF_BACKGROUND_HOSTING"
        private val session = BackgroundHostingSession()
        private val pending = mutableMapOf<String, MutableList<(JSONObject?, String?) -> Unit>>()
        private val main = Handler(Looper.getMainLooper())

        fun status(context: Context): JSONObject {
            val (enabled, running) = session.state()
            val power = context.getSystemService(PowerManager::class.java)
            return JSONObject().put("supported", true).put("enabled", enabled).put("running", running)
                .put("batteryOptimized", !power.isIgnoringBatteryOptimizations(context.packageName))
                .put("notificationsEnabled", notificationsEnabled(context))
        }

        fun start(activity: Activity, complete: (JSONObject?, String?) -> Unit) {
            if (activity !is RuntimeActivity || !activity.backgroundHostingVisible || activity.isFinishing || activity.isDestroyed) {
                complete(null, "Open Flow-Like before turning on background hosting.")
                return
            }
            if (!notificationsEnabled(activity)) {
                complete(null, "Turn on Flow-Like notifications in Android settings before enabling background hosting.")
                return
            }
            if (session.state().second) { complete(status(activity), null); return }
            val token = session.request()
            val waiting = pending.getOrPut(token) { mutableListOf() }
            waiting.add(complete)
            if (waiting.size > 1) return
            try {
                ContextCompat.startForegroundService(activity,
                    Intent(activity, BackgroundHostingService::class.java).putExtra(SESSION, token))
                val context = activity.applicationContext
                main.postDelayed({
                    if (session.accepts(token) && !session.state().second) {
                        session.stop(token)
                        finishStart(context, token, "Android did not start background hosting. Keep Flow-Like open and try again.")
                        context.stopService(Intent(context, BackgroundHostingService::class.java))
                    }
                }, 10_000)
            } catch (error: Exception) {
                session.stop(token)
                finishStart(activity, token, startFailure(error))
            }
        }

        fun stop(context: Context) {
            session.stop()
            val callbacks = pending.values.flatMap { it.toList() }
            pending.clear()
            callbacks.forEach { it(null, "Background hosting was turned off before it started.") }
            context.stopService(Intent(context, BackgroundHostingService::class.java))
        }

        fun openBatterySettings(activity: Activity) {
            openSettings(activity, Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
        }

        fun openNotificationSettings(activity: Activity) {
            val intent = if (Build.VERSION.SDK_INT >= 26) {
                Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, activity.packageName)
            } else appSettings(activity)
            openSettings(activity, intent)
        }

        private fun appSettings(context: Context) = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
            Uri.parse("package:${context.packageName}"))

        private fun openSettings(activity: Activity, intent: Intent) {
            if (intent.resolveActivity(activity.packageManager) != null) activity.startActivity(intent)
            else activity.startActivity(appSettings(activity))
        }

        private fun createChannel(context: Context) {
            if (Build.VERSION.SDK_INT >= 26) context.getSystemService(NotificationManager::class.java)
                .createNotificationChannel(NotificationChannel(CHANNEL,
                    context.getString(R.string.background_hosting_channel), NotificationManager.IMPORTANCE_LOW))
        }

        private fun notificationsEnabled(context: Context): Boolean =
            NotificationManagerCompat.from(context).areNotificationsEnabled() &&
                (Build.VERSION.SDK_INT < 26 || context.getSystemService(NotificationManager::class.java)
                    .getNotificationChannel(CHANNEL)?.importance != NotificationManager.IMPORTANCE_NONE)

        private fun finishStart(context: Context, token: String, error: String?) {
            pending.remove(token)?.forEach { it(if (error == null) status(context) else null, error) }
        }

        private fun startFailure(error: Exception): String {
            if (Build.VERSION.SDK_INT >= 31 && error is android.app.ForegroundServiceStartNotAllowedException)
                return "Android requires Flow-Like to be open before background hosting can start. Open the app and try again."
            return when (error) {
                is SecurityException -> "Android blocked background hosting. Check Flow-Like notification permissions and try again."
                is IllegalStateException -> error.message
                    ?: "Open Flow-Like and try turning on background hosting again."
                else -> "Android could not start background hosting. Keep Flow-Like open and try again."
            }
        }
    }
}

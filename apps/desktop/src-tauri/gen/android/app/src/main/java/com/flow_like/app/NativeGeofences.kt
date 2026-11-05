package com.flow_like.app

import android.Manifest
import android.app.Activity
import android.app.Application
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Looper
import android.os.SystemClock
import androidx.core.content.ContextCompat
import androidx.work.BackoffPolicy
import androidx.work.Data
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import com.google.android.gms.location.Geofence
import com.google.android.gms.common.ConnectionResult
import com.google.android.gms.common.GoogleApiAvailabilityLight
import com.google.android.gms.location.GeofencingEvent
import com.google.android.gms.location.GeofencingRequest
import com.google.android.gms.location.LocationCallback
import com.google.android.gms.location.LocationRequest
import com.google.android.gms.location.LocationResult
import com.google.android.gms.location.LocationServices
import com.google.android.gms.location.Priority
import com.google.android.gms.tasks.Tasks
import org.json.JSONArray
import org.json.JSONObject
import java.net.URI
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import javax.net.ssl.HttpsURLConnection

internal object NativeGeofencePolicy {
    const val RETENTION = 3_600_000L
    const val REMOTE_RETENTION = 840_000L
    fun validate(config: JSONObject, scope: String) {
        require(scope.isNotEmpty() && config.getString("scope") == scope)
        val values = config.getJSONArray("registrations")
        require(values.length() <= 100)
        val ids = mutableSetOf<String>()
        for (index in 0 until values.length()) {
            val value = values.getJSONObject(index)
            require(value.getString("id").matches(Regex("[a-f0-9]{64}")) && ids.add(value.getString("id")))
            for (key in listOf("appId", "eventId")) require(value.getString(key).isNotEmpty() && value.getString(key).length <= 512)
            require(value.getDouble("latitude").isFinite() && value.getDouble("latitude") in -90.0..90.0)
            require(value.getDouble("longitude").isFinite() && value.getDouble("longitude") in -180.0..180.0)
            require(value.getDouble("radiusMeters").isFinite() && value.getDouble("radiusMeters") in 100.0..100_000.0)
            require(value.getString("transition") in listOf("enter", "exit", "both"))
            require(value.get("background") is Boolean)
        }
    }
    fun crossing(scope: String, region: JSONObject, transition: String, now: Long): JSONObject {
        require(transition in listOf("enter", "exit"))
        return JSONObject().put("id", UUID.randomUUID().toString()).put("registrationId", region.getString("id"))
            .put("scope", scope).put("appId", region.getString("appId")).put("eventId", region.getString("eventId"))
            .put("transition", transition).put("occurredAt", now).put("radiusMeters", region.getDouble("radiusMeters"))
            .put("geometry", JSONObject().put("type", "Point").put("coordinates", JSONArray().put(region.getDouble("longitude")).put(region.getDouble("latitude"))))
    }
    fun terminal(status: Int, code: String?): Boolean = status == 401 || (status == 409 && code == "EVENT_PAUSED") ||
        (status in 400..499 && status !in listOf(402, 408, 409, 425, 429))
}

internal object NativeGeofences {
    private const val CONFIG = "device-geofence-config"
    private const val QUEUE = "device-geofence-crossings"
    private const val STATES = "device-geofence-states"
    private const val REFUSALS = "device-geofence-refusals"
    private const val WORK = "device-geofence-delivery"
    private const val ACTION = "com.flow_like.app.GEOFENCE_TRANSITION"
    private val lock = Any()
    private val executor = Executors.newSingleThreadExecutor()
    @Volatile private var installed = false
    @Volatile private var visible = false
    @Volatile private var deadline = 0L
    private var foregroundCallback: LocationCallback? = null
    private var monitoringSignature: String? = null
    private var foregroundSignature: String? = null
    @Volatile private var lastError: JSONObject? = null

    fun dispatch(work: () -> Unit) {
        executor.execute { try { work() } catch (_: Exception) {
            synchronized(lock) { foregroundSignature = null; monitoringSignature = null }
            lastError = JSONObject().put("code", "unavailable").put("message", "Android could not update location Events. Check location permissions and Google Play services.")
        } }
    }

    private fun json(context: Context, key: String): JSONObject? = BotSecureStore(context).read(key)?.let { JSONObject(it.toString(Charsets.UTF_8)) }
    private fun save(context: Context, key: String, value: JSONObject) = BotSecureStore(context).write(key, value.toString().toByteArray())
    fun scope(context: Context): String = BotNativeStore(context).snapshot()?.optString("scope") ?: ""
    private fun configuration(context: Context): JSONObject? = json(context, CONFIG)?.takeIf { it.optString("scope") == scope(context) }
    private fun regions(config: JSONObject?) = config?.optJSONArray("registrations")?.let { values -> (0 until values.length()).map { values.getJSONObject(it) } } ?: emptyList()
    private fun precise(context: Context) = ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED
    private fun background(context: Context) = Build.VERSION.SDK_INT < 29 || ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_BACKGROUND_LOCATION) == PackageManager.PERMISSION_GRANTED

    fun install(activity: Activity) {
        if (installed) return
        installed = true
        visible = true
        activity.application.registerActivityLifecycleCallbacks(object : Application.ActivityLifecycleCallbacks {
            private val resumed = mutableSetOf<Activity>()
            override fun onActivityResumed(activity: Activity) { synchronized(resumed) { resumed.add(activity); visible = true }; dispatch { reconcile(activity.applicationContext) } }
            override fun onActivityPaused(activity: Activity) { synchronized(resumed) { resumed.remove(activity); visible = resumed.isNotEmpty() }; dispatch { reconcile(activity.applicationContext) } }
            override fun onActivityCreated(activity: Activity, state: Bundle?) {}
            override fun onActivityStarted(activity: Activity) {}
            override fun onActivityStopped(activity: Activity) {}
            override fun onActivitySaveInstanceState(activity: Activity, state: Bundle) {}
            override fun onActivityDestroyed(activity: Activity) { synchronized(resumed) { resumed.remove(activity) } }
        })
        dispatch { reconcile(activity.applicationContext) }
    }

    fun status(context: Context): JSONObject = synchronized(lock) {
        val fine = precise(context)
        val coarse = ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED
        val asked = context.getSharedPreferences("native-device-permissions", Context.MODE_PRIVATE).getBoolean("locationAsked", false)
        val result = JSONObject().put("authorization", if (!fine && !coarse) { if (asked) "denied" else "not_determined" } else if (background(context)) "always" else "when_in_use")
            .put("accuracyAuthorization", if (fine) "full" else "reduced")
            .put("backgroundSupported", GoogleApiAvailabilityLight.getInstance().isGooglePlayServicesAvailable(context) == ConnectionResult.SUCCESS)
            .put("monitoredRegionCount", regions(configuration(context)).count { fine && (if (it.optBoolean("background")) background(context) else visible) })
            .put("maxMonitoredRegions", 100).put("backgroundDelivery", "system_monitored")
        val refusals = json(context, REFUSALS)
        lastError?.let { result.put("error", it) }
        if (refusals != null && refusals.length() > 0) result.put("error", JSONObject().put("code", "trigger_refused").put("message", "A location Event was refused by its hub. Open its trigger settings to enable it again."))
        result
    }

    fun runtimeState(context: Context): JSONObject = JSONObject().put("foreground", visible)
        .put("remaining", if (visible) -1.0 else maxOf(0.0, (deadline - SystemClock.elapsedRealtime()) / 1000.0))

    fun sync(context: Context, config: JSONObject): JSONObject { return try {
        check(Looper.myLooper() != Looper.getMainLooper())
        synchronized(lock) {
            NativeGeofencePolicy.validate(config, scope(context))
            if (regions(config).isNotEmpty() && !precise(context)) return deviceFailure("precise_location_required", "Allow precise location for location Events.")
            if (regions(config).any { it.optBoolean("background") } && !background(context)) return deviceFailure("background_permission_required", "Allow location access all the time for background location Events.")
            val old = json(context, CONFIG)
            if (old?.optString("scope") != config.getString("scope")) clearRecords(context)
            save(context, CONFIG, config)
            val ids = regions(config).map { it.getString("id") }.toSet()
            for (key in listOf(STATES, REFUSALS)) {
                val previous = json(context, key) ?: continue
                val current = JSONObject()
                for (id in ids) if (previous.has(id)) current.put(id, previous.get(id))
                save(context, key, current)
            }
        }
        reconcile(context)
        deviceSuccess(status(context))
    } catch (_: SecurityException) { deviceFailure("denied", "Android location permission was revoked.") }
      catch (_: Exception) { deviceFailure("unavailable", "The location regions could not be registered.") }
    }

    private fun pendingIntent(context: Context): PendingIntent = PendingIntent.getBroadcast(context, 7201,
        Intent(context, NativeGeofenceReceiver::class.java).setAction(ACTION), PendingIntent.FLAG_UPDATE_CURRENT or (if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0))

    fun reconcile(context: Context) = synchronized(lock) {
        val config = configuration(context)
        if (config == null) { clear(context); return@synchronized }
        val refused = json(context, REFUSALS) ?: JSONObject()
        val enabled = regions(config).filter { !refused.has(it.getString("id")) }
        val desired = enabled.filter { it.optBoolean("background") && precise(context) && background(context) }
        val signature = config.getString("scope") + desired.joinToString { it.getString("id") }
        if (signature != monitoringSignature) {
            val client = LocationServices.getGeofencingClient(context)
            Tasks.await(client.removeGeofences(pendingIntent(context)), 10, TimeUnit.SECONDS)
            if (desired.isNotEmpty()) {
                val request = GeofencingRequest.Builder().setInitialTrigger(0).addGeofences(desired.map { region ->
                    Geofence.Builder().setRequestId(region.getString("id"))
                        .setCircularRegion(region.getDouble("latitude"), region.getDouble("longitude"), region.getDouble("radiusMeters").toFloat())
                        .setExpirationDuration(Geofence.NEVER_EXPIRE).setTransitionTypes(Geofence.GEOFENCE_TRANSITION_ENTER or Geofence.GEOFENCE_TRANSITION_EXIT)
                        .setNotificationResponsiveness(60_000).build()
                }).build()
                Tasks.await(client.addGeofences(request, pendingIntent(context)), 10, TimeUnit.SECONDS)
            }
            monitoringSignature = signature
        }
        reconcileForeground(context, enabled.filter { !it.optBoolean("background") })
        schedule(context)
        lastError = null
    }

    private fun reconcileForeground(context: Context, regions: List<JSONObject>) {
        val signature = scope(context) + visible + precise(context) + regions.joinToString { it.getString("id") }
        if (signature == foregroundSignature) return
        val client = LocationServices.getFusedLocationProviderClient(context)
        foregroundCallback?.let { client.removeLocationUpdates(it) }
        foregroundCallback = null
        foregroundSignature = signature
        if (!visible || !precise(context) || regions.isEmpty()) {
            val states = json(context, STATES) ?: JSONObject()
            for (region in regions) states.remove(region.getString("id"))
            save(context, STATES, states)
            return
        }
        val callback = object : LocationCallback() {
            override fun onLocationResult(result: LocationResult) {
                val location = result.lastLocation ?: return
                if (System.currentTimeMillis() - location.time > 30_000) return
                dispatch {
                    for (region in regions) {
                        val distance = FloatArray(1)
                        android.location.Location.distanceBetween(location.latitude, location.longitude, region.getDouble("latitude"), region.getDouble("longitude"), distance)
                        record(context, region.getString("id"), if (distance[0] <= region.getDouble("radiusMeters")) "enter" else "exit", foregroundOnly = true)
                    }
                }
            }
        }
        foregroundCallback = callback
        client.requestLocationUpdates(LocationRequest.Builder(Priority.PRIORITY_BALANCED_POWER_ACCURACY, 10_000).setMinUpdateIntervalMillis(5_000).build(), callback, Looper.getMainLooper())
    }

    private fun clearRecords(context: Context) { for (key in listOf(CONFIG, QUEUE, STATES, REFUSALS)) BotSecureStore(context).delete(key) }
    fun clear(context: Context) = synchronized(lock) {
        clearRecords(context)
        monitoringSignature = null
        foregroundSignature = null
        try { LocationServices.getGeofencingClient(context).removeGeofences(pendingIntent(context)) } catch (_: Exception) { }
        try { foregroundCallback?.let { LocationServices.getFusedLocationProviderClient(context).removeLocationUpdates(it) } } catch (_: Exception) { }
        foregroundCallback = null
        try { WorkManager.getInstance(context).cancelUniqueWork(WORK) } catch (_: Exception) { }
    }

    fun record(context: Context, registrationId: String, transition: String, foregroundOnly: Boolean = false, occurredAt: Long = System.currentTimeMillis()) = synchronized(lock) {
        if (System.currentTimeMillis() - occurredAt !in 0..NativeGeofencePolicy.RETENTION) return@synchronized
        val config = configuration(context) ?: return@synchronized
        val region = regions(config).firstOrNull { it.optString("id") == registrationId } ?: return@synchronized
        if (!precise(context) || (region.optBoolean("background") && !background(context)) || (!region.optBoolean("background") && !visible)) return@synchronized
        if (foregroundOnly && region.optBoolean("background")) return@synchronized
        if (json(context, REFUSALS)?.has(registrationId) == true) return@synchronized
        val states = json(context, STATES) ?: JSONObject()
        val previous = states.optString(registrationId)
        if (previous == transition) return@synchronized
        states.put(registrationId, transition); save(context, STATES, states)
        // The first foreground location establishes a baseline without inventing a crossing.
        if (foregroundOnly && previous.isEmpty()) return@synchronized
        if (region.getString("transition") != "both" && region.getString("transition") != transition) return@synchronized
        val queue = queue(context)
        if (queue.length() >= 128) return@synchronized
        queue.put(NativeGeofencePolicy.crossing(config.getString("scope"), region, transition, occurredAt))
        save(context, QUEUE, JSONObject().put("events", queue))
        schedule(context)
        if (visible) BotCredentialBroker.geofenceReady(context)
    }

    private fun queue(context: Context): JSONArray {
        val values = json(context, QUEUE)?.optJSONArray("events") ?: JSONArray()
        val now = System.currentTimeMillis()
        val scope = scope(context)
        return JSONArray((0 until values.length()).map { values.getJSONObject(it) }.filter {
            it.optString("scope") == scope && now - it.optLong("occurredAt") in 0..NativeGeofencePolicy.RETENTION
        })
    }

    fun events(context: Context): JSONArray = synchronized(lock) {
        val regions = regions(configuration(context)).associateBy { it.getString("id") }
        val values = queue(context)
        JSONArray((0 until values.length()).map { values.getJSONObject(it) }.filter {
            visible || regions[it.optString("registrationId")]?.optJSONObject("delivery") == null || it.has("deliveryError")
        })
    }

    fun ack(context: Context, ids: JSONArray) = synchronized(lock) {
        val remove = (0 until ids.length()).map { ids.getString(it) }.toSet()
        val values = queue(context)
        save(context, QUEUE, JSONObject().put("events", JSONArray((0 until values.length()).map { values.getJSONObject(it) }.filter { it.getString("id") !in remove })))
    }

    fun schedule(context: Context) {
        if (synchronized(lock) { queue(context).length() == 0 }) return
        WorkManager.getInstance(context).enqueueUniqueWork(WORK, ExistingWorkPolicy.KEEP,
            OneTimeWorkRequestBuilder<NativeGeofenceWorker>().setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS).build())
    }

    fun runWorker(context: Context): Boolean {
        deadline = SystemClock.elapsedRealtime() + 25_000
        try {
            val config = synchronized(lock) { configuration(context) } ?: return true
            val scope = config.getString("scope")
            val values = synchronized(lock) { queue(context) }
            val regions = regions(config).associateBy { it.getString("id") }
            var retry = false
            var engineWoken = false
            for (index in 0 until values.length()) {
                if (SystemClock.elapsedRealtime() >= deadline - 1_000) { retry = true; break }
                val event = values.getJSONObject(index)
                val region = regions[event.optString("registrationId")]
                if (region == null) { ack(context, JSONArray().put(event.getString("id"))); continue }
                if (!precise(context) || (region.optBoolean("background") && !background(context))) { ack(context, JSONArray().put(event.getString("id"))); continue }
                if (!region.optBoolean("background") && !visible) continue
                val delivery = region.optJSONObject("delivery")
                if (delivery == null || visible || event.has("deliveryError")) { engineWoken = BotCredentialBroker.geofenceReady(context) || engineWoken; continue }
                if (System.currentTimeMillis() - event.getLong("occurredAt") > NativeGeofencePolicy.REMOTE_RETENTION) {
                    ack(context, JSONArray().put(event.getString("id"))); continue
                }
                if (BotCredentialBroker.accessToken(context, scope) == null) { retry = true; continue }
                if (scope(context) != scope || synchronized(lock) { regions(configuration(context)).none { it.optString("id") == region.getString("id") } }) continue
                val endpoint = URI(delivery.getString("endpoint"))
                val origin = URI(JSONArray(scope).getString(0))
                require(endpoint.scheme == "https" && endpoint.host == origin.host && endpoint.port == origin.port && endpoint.rawUserInfo == null && endpoint.fragment == null)
                val connection = endpoint.toURL().openConnection() as HttpsURLConnection
                try {
                    connection.instanceFollowRedirects = false; connection.requestMethod = "POST"
                    connection.connectTimeout = 5_000; connection.readTimeout = 5_000
                    connection.setRequestProperty("Authorization", delivery.getString("authorization"))
                    connection.setRequestProperty("Idempotency-Key", event.getString("id"))
                    connection.setRequestProperty("Content-Type", "application/json")
                    connection.doOutput = true
                    val body = JSONObject().put("transition", JSONObject(event.toString()).apply { remove("deliveryError") })
                    delivery.optJSONObject("oauthTokens")?.takeIf { it.length() > 0 }?.let { body.put("oauth_tokens", it) }
                    connection.outputStream.use { it.write(body.toString().toByteArray()) }
                    val status = connection.responseCode
                    val bytes = (if (status in 200..299) connection.inputStream else connection.errorStream)?.use { input ->
                        val output = java.io.ByteArrayOutputStream()
                        val buffer = ByteArray(8192)
                        while (true) {
                            val count = input.read(buffer)
                            if (count < 0) break
                            require(output.size() + count <= 1_048_576)
                            output.write(buffer, 0, count)
                        }
                        output.toByteArray()
                    } ?: ByteArray(0)
                    require(bytes.size <= 1_048_576)
                    val reply = try { JSONObject(bytes.toString(Charsets.UTF_8)) } catch (_: Exception) { JSONObject() }
                    if (status in 200..299 && reply.optString("run_id").isNotEmpty()) ack(context, JSONArray().put(event.getString("id")))
                    else if (NativeGeofencePolicy.terminal(status, reply.optJSONObject("error")?.optString("code"))) {
                        synchronized(lock) {
                            if (scope(context) == scope) {
                                val refusals = json(context, REFUSALS) ?: JSONObject()
                                refusals.put(region.getString("id"), true); save(context, REFUSALS, refusals)
                                val pending = queue(context)
                                for (i in 0 until pending.length()) if (pending.getJSONObject(i).getString("id") == event.getString("id")) pending.getJSONObject(i).put("deliveryError", "The hub refused this location trigger ($status).")
                                save(context, QUEUE, JSONObject().put("events", pending))
                            }
                        }
                        reconcile(context)
                    } else retry = true
                } finally { connection.disconnect() }
            }
            if (engineWoken && !visible) {
                val until = minOf(deadline - 1_000, SystemClock.elapsedRealtime() + 20_000)
                while (!visible && scope(context) == scope && SystemClock.elapsedRealtime() < until) Thread.sleep(200)
            }
            val pendingRemote = synchronized(lock) {
                val currentRegions = regions(configuration(context)).associateBy { it.getString("id") }
                val current = queue(context)
                (0 until current.length()).any { index ->
                    val item = current.getJSONObject(index)
                    val region = currentRegions[item.optString("registrationId")]
                    region?.optJSONObject("delivery") != null && region.optBoolean("background") && !item.has("deliveryError") &&
                        System.currentTimeMillis() - item.optLong("occurredAt") <= NativeGeofencePolicy.REMOTE_RETENTION
                }
            }
            return !retry && !pendingRemote
        } catch (_: Exception) { return false }
        finally { deadline = 0 }
    }

    fun receive(context: Context, intent: Intent) {
        if (intent.action != ACTION) return
        val event = GeofencingEvent.fromIntent(intent) ?: return
        if (event.hasError()) {
            monitoringSignature = null
            WorkManager.getInstance(context).enqueueUniqueWork("device-geofence-restore", ExistingWorkPolicy.REPLACE,
                OneTimeWorkRequestBuilder<NativeGeofenceRestoreWorker>().setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS).build())
            return
        }
        val transition = when (event.geofenceTransition) { Geofence.GEOFENCE_TRANSITION_ENTER -> "enter"; Geofence.GEOFENCE_TRANSITION_EXIT -> "exit"; else -> return }
        val ids = event.triggeringGeofences.orEmpty().map { it.requestId }.take(100).toTypedArray()
        if (ids.isEmpty()) return
        WorkManager.getInstance(context).enqueue(OneTimeWorkRequestBuilder<NativeGeofenceTransitionWorker>().setInputData(
            Data.Builder().putStringArray("registrations", ids).putString("transition", transition).putLong("occurredAt", System.currentTimeMillis()).build()).build())
    }
}

class NativeGeofenceReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        try { NativeGeofences.receive(context.applicationContext, intent) } catch (_: Exception) { }
    }
}

class NativeGeofenceTransitionWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result { return try {
        val transition = inputData.getString("transition") ?: return Result.failure()
        if (transition !in listOf("enter", "exit")) return Result.failure()
        for (id in inputData.getStringArray("registrations").orEmpty().take(100)) {
            NativeGeofences.record(applicationContext, id, transition, occurredAt = inputData.getLong("occurredAt", 0))
        }
        Result.success()
    } catch (_: Exception) { Result.retry() }
    }
}

class NativeGeofenceBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action !in listOf(Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED)) return
        WorkManager.getInstance(context).enqueueUniqueWork("device-geofence-restore", ExistingWorkPolicy.REPLACE,
            OneTimeWorkRequestBuilder<NativeGeofenceRestoreWorker>().setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS).build())
    }
}

class NativeGeofenceRestoreWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result = try { NativeGeofences.reconcile(applicationContext); Result.success() } catch (_: Exception) { Result.retry() }
}

class NativeGeofenceWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result = if (NativeGeofences.runWorker(applicationContext)) Result.success() else Result.retry()
}

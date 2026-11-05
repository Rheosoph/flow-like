package com.flow_like.app

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.location.LocationManager
import android.os.Handler
import android.os.Looper
import androidx.core.content.ContextCompat
import androidx.core.location.LocationManagerCompat
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import com.google.android.gms.location.CurrentLocationRequest
import com.google.android.gms.location.LocationServices
import com.google.android.gms.location.Priority
import com.google.android.gms.tasks.CancellationTokenSource
import org.json.JSONObject
import java.lang.ref.WeakReference
import java.util.concurrent.ConcurrentHashMap

internal fun deviceSuccess(value: JSONObject = JSONObject()): JSONObject = JSONObject().put("ok", true).put("value", value)
internal fun deviceFailure(code: String, message: String): JSONObject = JSONObject().put("ok", false).put("error", JSONObject().put("code", code).put("message", message))

internal object NativeDeviceLocation {
    private class Pending(
        val scope: String?, val activity: WeakReference<Activity>, val options: JSONObject,
        val deadline: Long, val callback: (JSONObject) -> Unit,
    ) {
        val cancellation = CancellationTokenSource()
        var observer: DefaultLifecycleObserver? = null
        var awaitingPermission = false
        var started = false
        var result: JSONObject? = null
        var timer: Runnable? = null
    }
    private val pending = ConcurrentHashMap<String, Pending>()
    private val main = Handler(Looper.getMainLooper())
    private var permissionOwner: String? = null
    private var permissionActivity: WeakReference<Activity>? = null

    fun request(activity: Activity, id: String, options: JSONObject, requestPermissions: () -> Unit, callback: (JSONObject) -> Unit) {
        check(Looper.myLooper() == Looper.getMainLooper())
        try {
            require(id.matches(Regex("[A-Za-z0-9_-]{1,128}")) && pending.size < 16 && !pending.containsKey(id))
            require(!options.has("highAccuracy") || options.get("highAccuracy") is Boolean)
            val age = options.optLong("maximumAgeMs", 0)
            val timeout = options.optLong("timeoutMs", 10_000)
            require(age in 0..300_000 && timeout in 100..120_000)
            val deadline = options.optDouble("requestDeadline", System.currentTimeMillis().toDouble() + timeout)
            require(deadline.isFinite() && deadline > System.currentTimeMillis())
            val owner = activity as? LifecycleOwner
            if (owner == null || !owner.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED) || activity.isFinishing) {
                callback(deviceFailure("foreground_required", "Keep this app screen open to use location.")); return
            }
            if (permissionActivity?.get()?.let { !it.isDestroyed && !it.isFinishing } != true) permissionOwner = null
            val needsPermission = !hasLocation(activity)
            if (needsPermission && permissionOwner != null) { callback(deviceFailure("busy", "A location permission request is already open.")); return }
            val request = Pending(BotNativeStore(activity).snapshot()?.optString("scope"), WeakReference(activity), JSONObject(options.toString()),
                minOf(deadline.toLong(), System.currentTimeMillis() + timeout), callback)
            check(pending.putIfAbsent(id, request) == null)
            request.awaitingPermission = needsPermission
            request.observer = object : DefaultLifecycleObserver {
                override fun onResume(owner: LifecycleOwner) { main.post { advance(id) } }
                override fun onStop(owner: LifecycleOwner) { finish(id, deviceFailure("foreground_required", "Location stopped because this app screen is no longer active.")) }
                override fun onDestroy(owner: LifecycleOwner) {
                    if (permissionOwner == id) permissionOwner = null
                    finish(id, deviceFailure("cancelled", "The location request ended."))
                }
            }
            owner.lifecycle.addObserver(request.observer!!)
            request.timer = Runnable { finish(id, deviceFailure("timeout", "The location request timed out.")) }
            main.postDelayed(request.timer!!, (request.deadline - System.currentTimeMillis()).coerceAtLeast(1))
            if (needsPermission) {
                permissionOwner = id
                permissionActivity = WeakReference(activity)
                activity.getSharedPreferences("native-device-permissions", Activity.MODE_PRIVATE).edit().putBoolean("locationAsked", true).apply()
                try { requestPermissions() }
                catch (_: Exception) { permissionOwner = null; finish(id, deviceFailure("unavailable", "Location permission could not be requested.")) }
            } else main.post { advance(id) }
        } catch (_: Exception) { callback(deviceFailure("invalid_input", "Invalid location request or too many pending requests.")) }
    }

    fun permissionResult(activity: Activity, id: String) {
        main.post {
            if (permissionOwner == id) permissionOwner = null
            val request = pending[id] ?: return@post
            if (request.activity.get() !== activity) { finish(id, deviceFailure("cancelled", "The location request ended.")); return@post }
            request.awaitingPermission = false
            if (!hasLocation(activity)) request.result = deviceFailure("denied", "Location permission was denied.")
            advance(id)
        }
    }

    fun cancel(id: String) { main.post { finish(id, deviceFailure("cancelled", "The location request was cancelled.")) } }

    private fun hasLocation(activity: Activity) = ContextCompat.checkSelfPermission(activity, Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED ||
        ContextCompat.checkSelfPermission(activity, Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED

    private fun advance(id: String) {
        val request = pending[id] ?: return
        val activity = request.activity.get()
        if (activity == null || activity.isFinishing || activity.isDestroyed) { finish(id, deviceFailure("cancelled", "The location request ended.")); return }
        if (request.scope != BotNativeStore(activity).snapshot()?.optString("scope")) { finish(id, deviceFailure("scope_changed", "The active workspace changed.")); return }
        if (System.currentTimeMillis() >= request.deadline) { finish(id, deviceFailure("timeout", "The location request timed out.")); return }
        // The post from onResume runs after Wry has resumed its WebView. Permission UI never opens a second app Activity.
        if (!(activity as LifecycleOwner).lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED) || request.awaitingPermission) return
        request.result?.let { finish(id, it); return }
        if (request.started) return
        request.started = true
        if (!hasLocation(activity)) { finish(id, deviceFailure("denied", "Location permission was revoked.")); return }
        if (!LocationManagerCompat.isLocationEnabled(activity.getSystemService(LocationManager::class.java))) {
            finish(id, deviceFailure("unavailable", "Turn on Location in Android settings.")); return
        }
        try {
            val query = CurrentLocationRequest.Builder().setMaxUpdateAgeMillis(request.options.optLong("maximumAgeMs", 0))
                .setDurationMillis((request.deadline - System.currentTimeMillis()).coerceAtLeast(1))
                .setPriority(if (request.options.optBoolean("highAccuracy")) Priority.PRIORITY_HIGH_ACCURACY else Priority.PRIORITY_BALANCED_POWER_ACCURACY).build()
            LocationServices.getFusedLocationProviderClient(activity).getCurrentLocation(query, request.cancellation.token)
                .addOnSuccessListener { location ->
                    if (pending[id] !== request) return@addOnSuccessListener
                    request.result = if (location == null) deviceFailure("timeout", "No current location was available.") else {
                        val value = JSONObject().put("latitude", location.latitude).put("longitude", location.longitude)
                            .put("geometry", JSONObject().put("type", "Point").put("coordinates", org.json.JSONArray().put(location.longitude).put(location.latitude)))
                            .put("accuracy", location.accuracy.toDouble()).put("timestamp", location.time)
                        if (location.hasAltitude()) value.put("altitude", location.altitude)
                        if (location.hasSpeed()) value.put("speed", location.speed.toDouble())
                        if (location.hasBearing()) value.put("heading", location.bearing.toDouble())
                        deviceSuccess(value)
                    }
                    main.post { advance(id) }
                }.addOnFailureListener {
                    if (pending[id] === request) { request.result = deviceFailure("unavailable", "Android could not obtain the location."); main.post { advance(id) } }
                }
        } catch (_: SecurityException) { finish(id, deviceFailure("denied", "Location permission was revoked.")) }
          catch (_: Exception) { finish(id, deviceFailure("unavailable", "Location services are unavailable.")) }
    }

    private fun finish(id: String, value: JSONObject) {
        val request = pending.remove(id) ?: return
        request.cancellation.cancel()
        request.timer?.let { main.removeCallbacks(it) }
        val activity = request.activity.get()
        request.observer?.let { (activity as? LifecycleOwner)?.lifecycle?.removeObserver(it) }
        val current = activity?.let { BotNativeStore(it).snapshot()?.optString("scope") }
        request.callback(if (request.scope == current) value else deviceFailure("scope_changed", "The active workspace changed."))
    }
}

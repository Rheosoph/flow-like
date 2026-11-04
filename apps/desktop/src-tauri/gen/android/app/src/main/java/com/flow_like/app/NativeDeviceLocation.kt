package com.flow_like.app

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.location.LocationManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import androidx.core.location.LocationManagerCompat
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
    private data class Pending(val scope: String?, val callback: (JSONObject) -> Unit, var activity: WeakReference<NativeLocationActivity>? = null)
    private val pending = ConcurrentHashMap<String, Pending>()
    private val main = Handler(Looper.getMainLooper())

    fun request(activity: Activity, id: String, options: JSONObject, callback: (JSONObject) -> Unit) {
        try {
            require(id.matches(Regex("[A-Za-z0-9_-]{1,128}")) && pending.size < 16 && !pending.containsKey(id))
            require(!options.has("highAccuracy") || options.get("highAccuracy") is Boolean)
            val age = options.optLong("maximumAgeMs", 0)
            val timeout = options.optLong("timeoutMs", 10_000)
            require(age in 0..300_000 && timeout in 100..120_000)
            val deadline = options.optDouble("requestDeadline", System.currentTimeMillis().toDouble() + timeout)
            require(deadline.isFinite() && deadline > System.currentTimeMillis())
            val scope = BotNativeStore(activity).snapshot()?.optString("scope")
            check(pending.putIfAbsent(id, Pending(scope, callback)) == null)
            val duration = minOf(timeout, (deadline - System.currentTimeMillis()).toLong()).coerceAtLeast(1)
            main.post {
                try {
                    if (!pending.containsKey(id)) return@post
                    activity.startActivity(Intent(activity, NativeLocationActivity::class.java).putExtra("requestId", id).putExtra("options", options.toString()))
                } catch (_: Exception) { finish(activity, id, deviceFailure("unavailable", "Location permission could not be requested.")) }
            }
            main.postDelayed({ finish(activity, id, deviceFailure("timeout", "The location request timed out.")) }, duration)
        } catch (_: Exception) { callback(deviceFailure("invalid_input", "Invalid location request or too many pending requests.")) }
    }

    fun attach(id: String, activity: NativeLocationActivity): Boolean {
        val request = pending[id] ?: return false
        request.activity = WeakReference(activity)
        return true
    }

    fun cancel(id: String) {
        val request = pending.remove(id) ?: return
        main.post { request.activity?.get()?.stopRequest(); request.callback(deviceFailure("cancelled", "The location request was cancelled.")) }
    }

    fun finish(context: android.content.Context, id: String, value: JSONObject) {
        val request = pending.remove(id) ?: return
        main.post {
            request.activity?.get()?.stopRequest()
            val current = BotNativeStore(context).snapshot()?.optString("scope")
            request.callback(if (request.scope == current) value else deviceFailure("scope_changed", "The active workspace changed."))
        }
    }
}

class NativeLocationActivity : ComponentActivity() {
    private val id get() = intent.getStringExtra("requestId") ?: ""
    private val cancellation = CancellationTokenSource()
    private var requested = false
    private val permission = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { startLocation() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (!NativeDeviceLocation.attach(id, this)) { finish(); return }
        if (hasLocation()) startLocation()
        else {
            getSharedPreferences("native-device-permissions", MODE_PRIVATE).edit().putBoolean("locationAsked", true).apply()
            permission.launch(arrayOf(Manifest.permission.ACCESS_COARSE_LOCATION, Manifest.permission.ACCESS_FINE_LOCATION))
        }
    }

    private fun hasLocation() = ContextCompat.checkSelfPermission(this, Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED ||
        ContextCompat.checkSelfPermission(this, Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED

    private fun startLocation() {
        if (requested || isFinishing) return
        requested = true
        if (!hasLocation()) { NativeDeviceLocation.finish(this, id, deviceFailure("denied", "Location permission was denied.")); return }
        if (!LocationManagerCompat.isLocationEnabled(getSystemService(LocationManager::class.java))) {
            NativeDeviceLocation.finish(this, id, deviceFailure("unavailable", "Turn on Location in Android settings.")); return
        }
        try {
            val options = JSONObject(intent.getStringExtra("options") ?: "{}")
            val query = CurrentLocationRequest.Builder().setMaxUpdateAgeMillis(options.optLong("maximumAgeMs", 0))
                .setDurationMillis(options.optLong("timeoutMs", 10_000))
                .setPriority(if (options.optBoolean("highAccuracy")) Priority.PRIORITY_HIGH_ACCURACY else Priority.PRIORITY_BALANCED_POWER_ACCURACY).build()
            LocationServices.getFusedLocationProviderClient(this).getCurrentLocation(query, cancellation.token)
                .addOnSuccessListener { location ->
                    if (location == null) NativeDeviceLocation.finish(this, id, deviceFailure("timeout", "No current location was available."))
                    else {
                        val value = JSONObject().put("latitude", location.latitude).put("longitude", location.longitude)
                            .put("geometry", JSONObject().put("type", "Point").put("coordinates", org.json.JSONArray().put(location.longitude).put(location.latitude)))
                            .put("accuracy", location.accuracy.toDouble()).put("timestamp", location.time)
                        if (location.hasAltitude()) value.put("altitude", location.altitude)
                        if (location.hasSpeed()) value.put("speed", location.speed.toDouble())
                        if (location.hasBearing()) value.put("heading", location.bearing.toDouble())
                        NativeDeviceLocation.finish(this, id, deviceSuccess(value))
                    }
                }.addOnFailureListener { NativeDeviceLocation.finish(this, id, deviceFailure("unavailable", "Android could not obtain the location.")) }
        } catch (_: SecurityException) { NativeDeviceLocation.finish(this, id, deviceFailure("denied", "Location permission was revoked.")) }
          catch (_: Exception) { NativeDeviceLocation.finish(this, id, deviceFailure("unavailable", "Location services are unavailable.")) }
    }

    internal fun stopRequest() { cancellation.cancel(); finish() }
    override fun onDestroy() { cancellation.cancel(); if (isFinishing) NativeDeviceLocation.cancel(id); super.onDestroy() }
}

package com.flow_like.app

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import java.lang.ref.WeakReference
import org.json.JSONObject
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

internal object NativeGeofencePermissions {
    private class Pending(val scope: String?, val activity: WeakReference<Activity>, val callback: (JSONObject) -> Unit) {
        var observer: DefaultLifecycleObserver? = null
        var result: JSONObject? = null
        var completing = false
        var timer: Runnable? = null
    }
    private val callbacks = ConcurrentHashMap<String, Pending>()
    private val main = Handler(Looper.getMainLooper())
    fun request(activity: Activity, mode: String, callback: (JSONObject) -> Unit) {
        if (mode == "status") {
            NativeGeofences.dispatch {
                val result = try { deviceSuccess(NativeGeofences.status(activity)) } catch (_: Exception) { deviceFailure("unavailable", "Location settings are unavailable.") }
                main.post { callback(result) }
            }
            return
        }
        if (mode !in listOf("foreground", "background")) { callback(deviceFailure("invalid_input", "Choose foreground or background location access.")); return }
        if (callbacks.isNotEmpty()) { callback(deviceFailure("busy", "A location permission request is already open.")); return }
        val owner = activity as? LifecycleOwner
        if (owner == null || !owner.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) { callback(deviceFailure("foreground_required", "Open this app screen to change location access.")); return }
        val id = UUID.randomUUID().toString()
        val request = Pending(BotNativeStore(activity).snapshot()?.optString("scope"), WeakReference(activity), callback)
        callbacks[id] = request
        request.observer = object : DefaultLifecycleObserver {
            override fun onResume(owner: LifecycleOwner) { main.post { deliver(id) } }
            override fun onDestroy(owner: LifecycleOwner) { finish(id, deviceFailure("cancelled", "The location permission request ended.")) }
        }
        owner.lifecycle.addObserver(request.observer!!)
        request.timer = Runnable { finish(id, deviceFailure("timeout", "The location permission request timed out.")) }
        main.postDelayed(request.timer!!, 600_000)
        try { activity.startActivity(Intent(activity, NativeGeofencePermissionActivity::class.java).putExtra("id", id).putExtra("mode", mode)) }
        catch (_: Exception) { finish(id, deviceFailure("unavailable", "Location settings could not be opened.")) }
    }
    fun current(activity: Activity, id: String): Boolean {
        val request = callbacks[id] ?: return false
        return request.scope == BotNativeStore(activity).snapshot()?.optString("scope")
    }
    fun complete(activity: Activity, id: String) {
        val request = callbacks[id] ?: return
        if (request.completing) return
        request.completing = true
        val context = activity.applicationContext
        NativeGeofences.dispatch {
            val result = try { deviceSuccess(NativeGeofences.status(context)) } catch (_: Exception) { deviceFailure("unavailable", "Location settings are unavailable.") }
            main.post { if (callbacks[id] === request) { request.result = result; deliver(id) } }
            NativeGeofences.reconcile(context)
        }
    }
    private fun deliver(id: String) {
        val request = callbacks[id] ?: return
        val activity = request.activity.get()
        if (activity == null || activity.isDestroyed || activity.isFinishing) { finish(id, deviceFailure("cancelled", "The location permission request ended.")); return }
        if (!(activity as LifecycleOwner).lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) return
        request.result?.let { finish(id, it) }
    }
    private fun finish(id: String, result: JSONObject) {
        val request = callbacks.remove(id) ?: return
        request.timer?.let { main.removeCallbacks(it) }
        val activity = request.activity.get()
        request.observer?.let { (activity as? LifecycleOwner)?.lifecycle?.removeObserver(it) }
        val current = activity?.let { BotNativeStore(it).snapshot()?.optString("scope") }
        request.callback(if (request.scope == current) result else deviceFailure("scope_changed", "The active workspace changed."))
    }
}

class NativeGeofencePermissionActivity : ComponentActivity() {
    private val id get() = intent.getStringExtra("id") ?: ""
    private val background get() = intent.getStringExtra("mode") == "background"
    private val foregroundPermission = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { requestBackgroundOrFinish() }
    private val backgroundPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { done() }
    private val settings = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { done() }
    private fun granted(permission: String) = ContextCompat.checkSelfPermission(this, permission) == PackageManager.PERMISSION_GRANTED
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        if (!NativeGeofencePermissions.current(this, id)) { done(); return }
        if (state != null) return
        if (!granted(Manifest.permission.ACCESS_FINE_LOCATION)) {
            getSharedPreferences("native-device-permissions", MODE_PRIVATE).edit().putBoolean("locationAsked", true).apply()
            foregroundPermission.launch(arrayOf(Manifest.permission.ACCESS_COARSE_LOCATION, Manifest.permission.ACCESS_FINE_LOCATION))
        }
        else requestBackgroundOrFinish()
    }
    private fun requestBackgroundOrFinish() {
        if (!NativeGeofencePermissions.current(this, id)) { done(); return }
        if (!background || !granted(Manifest.permission.ACCESS_FINE_LOCATION) || Build.VERSION.SDK_INT < 29 || granted(Manifest.permission.ACCESS_BACKGROUND_LOCATION)) { done(); return }
        if (Build.VERSION.SDK_INT == 29) { backgroundPermission.launch(Manifest.permission.ACCESS_BACKGROUND_LOCATION); return }
        AlertDialog.Builder(this).setTitle("Allow background location Events")
            .setMessage("To run the arrival and departure Events you enable while Flow Like is closed, choose Allow all the time in Android's location settings. You can turn these Events off at any time.")
            .setPositiveButton("Open settings") { _, _ ->
                if (NativeGeofencePermissions.current(this, id)) settings.launch(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName"))) else done()
            }
            .setNegativeButton("Cancel") { _, _ -> done() }.setOnCancelListener { done() }.show()
    }
    private fun done() { NativeGeofencePermissions.complete(this, id); finish() }
    override fun onDestroy() { if (isFinishing) NativeGeofencePermissions.complete(this, id); super.onDestroy() }
}

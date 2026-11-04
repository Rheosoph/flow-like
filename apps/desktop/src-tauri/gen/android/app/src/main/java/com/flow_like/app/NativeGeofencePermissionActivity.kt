package com.flow_like.app

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import org.json.JSONObject
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

internal object NativeGeofencePermissions {
    private val callbacks = ConcurrentHashMap<String, (JSONObject) -> Unit>()
    fun request(activity: Activity, mode: String, callback: (JSONObject) -> Unit) {
        if (mode == "status") { callback(deviceSuccess(NativeGeofences.status(activity))); return }
        if (mode !in listOf("foreground", "background")) { callback(deviceFailure("invalid_input", "Choose foreground or background location access.")); return }
        if (callbacks.size >= 4) { callback(deviceFailure("busy", "A location permission request is already open.")); return }
        val id = UUID.randomUUID().toString()
        callbacks[id] = callback
        activity.runOnUiThread {
            try { activity.startActivity(Intent(activity, NativeGeofencePermissionActivity::class.java).putExtra("id", id).putExtra("mode", mode)) }
            catch (_: Exception) { callbacks.remove(id)?.invoke(deviceFailure("unavailable", "Location settings could not be opened.")) }
        }
    }
    fun complete(activity: Activity, id: String) {
        callbacks.remove(id)?.invoke(deviceSuccess(NativeGeofences.status(activity)))
        NativeGeofences.dispatch { try { NativeGeofences.reconcile(activity.applicationContext) } catch (_: Exception) { } }
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
        if (state != null) return
        if (!granted(Manifest.permission.ACCESS_FINE_LOCATION)) {
            getSharedPreferences("native-device-permissions", MODE_PRIVATE).edit().putBoolean("locationAsked", true).apply()
            foregroundPermission.launch(arrayOf(Manifest.permission.ACCESS_COARSE_LOCATION, Manifest.permission.ACCESS_FINE_LOCATION))
        }
        else requestBackgroundOrFinish()
    }
    private fun requestBackgroundOrFinish() {
        if (!background || !granted(Manifest.permission.ACCESS_FINE_LOCATION) || Build.VERSION.SDK_INT < 29 || granted(Manifest.permission.ACCESS_BACKGROUND_LOCATION)) { done(); return }
        if (Build.VERSION.SDK_INT == 29) { backgroundPermission.launch(Manifest.permission.ACCESS_BACKGROUND_LOCATION); return }
        AlertDialog.Builder(this).setTitle("Allow background location Events")
            .setMessage("To run the arrival and departure Events you enable while Flow Like is closed, choose Allow all the time in Android's location settings. You can turn these Events off at any time.")
            .setPositiveButton("Open settings") { _, _ -> settings.launch(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName"))) }
            .setNegativeButton("Cancel") { _, _ -> done() }.setOnCancelListener { done() }.show()
    }
    private fun done() { NativeGeofencePermissions.complete(this, id); finish() }
    override fun onDestroy() { if (isFinishing) NativeGeofencePermissions.complete(this, id); super.onDestroy() }
}

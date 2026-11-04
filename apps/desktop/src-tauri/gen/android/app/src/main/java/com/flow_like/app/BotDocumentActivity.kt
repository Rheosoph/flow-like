package com.flow_like.app

import android.app.Activity
import android.app.ActivityManager
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import com.google.android.gms.common.ConnectionResult
import com.google.android.gms.common.GoogleApiAvailabilityLight
import com.google.mlkit.vision.documentscanner.GmsDocumentScannerOptions
import com.google.mlkit.vision.documentscanner.GmsDocumentScanning
import com.google.mlkit.vision.documentscanner.GmsDocumentScanningResult

class BotDocumentActivity : ComponentActivity() {
    companion object {
        fun scannerAvailable(context: Context): Boolean {
            val memory = ActivityManager.MemoryInfo()
            context.getSystemService(ActivityManager::class.java).getMemoryInfo(memory)
            return memory.totalMem >= 1_700_000_000L &&
                GoogleApiAvailabilityLight.getInstance().isGooglePlayServicesAvailable(context) == ConnectionResult.SUCCESS
        }
    }

    private val scanner = registerForActivityResult(ActivityResultContracts.StartIntentSenderForResult()) { result ->
        if (result.resultCode == Activity.RESULT_OK) {
            val pdf = GmsDocumentScanningResult.fromActivityResultIntent(result.data)?.pdf
            if (pdf != null && pdf.pageCount in 1..10) {
                setResult(Activity.RESULT_OK, Intent().setData(pdf.uri).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
            }
        }
        finish()
    }
    private val picker = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        setResult(result.resultCode, result.data)
        finish()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(TextView(this).apply { text = "Preparing document scanner…"; setPadding(32, 64, 32, 32) })
        if (savedInstanceState != null) return
        val options = GmsDocumentScannerOptions.Builder().setGalleryImportAllowed(true).setPageLimit(10)
            .setResultFormats(GmsDocumentScannerOptions.RESULT_FORMAT_PDF)
            .setScannerMode(GmsDocumentScannerOptions.SCANNER_MODE_FULL).build()
        GmsDocumentScanning.getClient(options).getStartScanIntent(this)
            .addOnSuccessListener { sender ->
                try { scanner.launch(IntentSenderRequest.Builder(sender).build()) }
                catch (_: Exception) { fallback() }
            }.addOnFailureListener { fallback() }
    }

    private fun fallback() {
        if (isFinishing || isDestroyed) return
        AlertDialog.Builder(this).setTitle("Document scanner unavailable")
            .setMessage("Choose an existing document to attach instead.")
            .setPositiveButton("Choose document") { _, _ -> picker.launch(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE); type = "*/*"
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }) }
            .setNegativeButton("Cancel") { _, _ -> finish() }.setOnCancelListener { finish() }.show()
    }
}

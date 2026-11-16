package com.flow_like.app

import android.content.Intent
import android.content.res.Configuration
import android.os.Bundle
import android.provider.Settings
import android.text.format.Formatter
import android.view.View
import android.widget.Button
import android.widget.ProgressBar
import android.widget.TextView
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import com.google.android.play.core.splitcompat.SplitCompat
import com.google.android.play.core.splitinstall.SplitInstallException
import com.google.android.play.core.splitinstall.SplitInstallManager
import com.google.android.play.core.splitinstall.SplitInstallManagerFactory
import com.google.android.play.core.splitinstall.SplitInstallRequest
import com.google.android.play.core.splitinstall.SplitInstallSessionState
import com.google.android.play.core.splitinstall.SplitInstallStateUpdatedListener
import com.google.android.play.core.splitinstall.model.SplitInstallErrorCode
import com.google.android.play.core.splitinstall.model.SplitInstallSessionStatus

/** The exported launcher stays usable before any Rust code is installed. */
class MainActivity : AppCompatActivity() {
    private lateinit var manager: SplitInstallManager
    private lateinit var setup: NativeSetupView
    private lateinit var message: TextView
    private lateinit var detail: TextView
    private lateinit var progress: ProgressBar
    private lateinit var primary: Button
    private lateinit var secondary: Button
    private lateinit var storage: Button
    private var sessionId = 0
    private var confirmationSession = 0
    private var autoRequest = true
    private var requesting = false
    private var loading = false
    private var prepared = false
    private var forwarded = false
    private var resumed = false
    private var lastState: SplitInstallSessionState? = null

    private val confirmation = registerForActivityResult(ActivityResultContracts.StartIntentSenderForResult()) { result ->
        confirmationSession = 0
        if (result.resultCode == RESULT_OK) refresh() else cancelDownload()
    }
    private val listener = SplitInstallStateUpdatedListener { state ->
        if (state.sessionId() == sessionId || (sessionId == 0 && relevant(state))) accept(state)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        SplitCompat.installActivity(this)
        updateSystemBars(resources.configuration)
        manager = SplitInstallManagerFactory.create(applicationContext)
        sessionId = savedInstanceState?.getInt("installSession") ?: 0
        confirmationSession = savedInstanceState?.getInt("confirmationSession") ?: 0
        autoRequest = savedInstanceState?.getBoolean("autoRequest") ?: true
        createContent()
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                if (lastState?.let { phase(it).active } == true) manager.cancelInstall(sessionId)
                finish()
            }
        })
    }

    override fun onStart() {
        super.onStart()
        manager.registerListener(listener)
        refresh()
    }

    override fun onResume() {
        super.onResume()
        resumed = true
        if (prepared) { forwardToRuntime(); return }
        lastState?.takeIf { phase(it) == NativeInstallPhase.CONFIRMATION }?.let { requestConfirmation(it) }
    }

    override fun onPause() { resumed = false; super.onPause() }
    override fun onStop() { manager.unregisterListener(listener); super.onStop() }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        updateSystemBars(newConfig)
        setup.applyPalette()
    }

    private fun updateSystemBars(configuration: Configuration) {
        NativeUi.systemBars(this, configuration)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        refresh()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putInt("installSession", sessionId)
        outState.putInt("confirmationSession", confirmationSession)
        outState.putBoolean("autoRequest", autoRequest)
        super.onSaveInstanceState(outState)
    }

    private fun createContent() {
        setup = NativeSetupView(this)
        message = setup.message
        detail = setup.detail
        progress = setup.progress
        primary = setup.primary
        secondary = setup.secondary.apply { setOnClickListener { finish() } }
        storage = setup.storage.apply {
            setOnClickListener {
                try { startActivity(Intent(Settings.ACTION_INTERNAL_STORAGE_SETTINGS)) }
                catch (_: Exception) { detail.text = "Open Android Settings to free up storage, then retry the download." }
            }
        }
        ViewCompat.setOnApplyWindowInsetsListener(setup) { view, insets ->
            val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
            view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
            insets
        }
        setContentView(setup)
    }

    private fun refresh() {
        if (isFinishing || isDestroyed || loading || forwarded || requesting) return
        val missing = try { NativeRuntimeInstaller.missingModules(this) }
        catch (_: Exception) { showError("Android could not check the installed tools. Close Flow-Like and try again."); return }
        if (missing.isEmpty()) { openRuntime(); return }
        manager.sessionStates.addOnSuccessListener(this) { states ->
            if (isFinishing || loading || forwarded) return@addOnSuccessListener
            val selected = NativeInstallPolicy.selectSession(states.map {
                NativeInstallSession(it.sessionId(), it.moduleNames().toSet(), phase(it))
            }, sessionId)
            val state = states.firstOrNull { it.sessionId() == selected?.id }
            if (state != null) accept(state)
            else if (autoRequest) requestDownload()
            else showPaused()
        }.addOnFailureListener(this) { error -> showPlayError(error) }
    }

    private fun requestDownload() {
        if (requesting || loading || forwarded) return
        autoRequest = false
        sessionId = 0
        confirmationSession = 0
        val missing = try { NativeRuntimeInstaller.missingModules(this) }
        catch (_: Exception) { showError("Android could not check the installed tools. Close Flow-Like and try again."); return }
        if (missing.isEmpty()) { openRuntime(); return }
        requesting = true
        lastState = null
        showWorking("Starting download…", "Google Play will download the tools required for this device.")
        secondary.isEnabled = false
        val request = SplitInstallRequest.newBuilder().apply { missing.forEach { addModule(it) } }.build()
        manager.startInstall(request).addOnSuccessListener { id ->
            requesting = false
            if (isFinishing) { manager.cancelInstall(id); return@addOnSuccessListener }
            if (isDestroyed) return@addOnSuccessListener
            sessionId = id
            if (id == 0) refresh()
            else manager.getSessionState(id).addOnSuccessListener(this) { accept(it) }
                .addOnFailureListener(this) { showPlayError(it) }
        }.addOnFailureListener { error ->
            requesting = false
            if (!isDestroyed && !isFinishing) showPlayError(error)
        }
    }

    private fun accept(state: SplitInstallSessionState) {
        if (isFinishing || isDestroyed || forwarded || loading) return
        sessionId = state.sessionId()
        lastState = state
        autoRequest = false
        when (phase(state)) {
            NativeInstallPhase.WAITING -> showWorking("Waiting for Google Play…", "The download will start when Google Play is ready.")
            NativeInstallPhase.CONFIRMATION -> {
                showWorking("Confirm the download", "Google Play needs your permission to download these tools.")
                primary.visibility = View.VISIBLE
                primary.text = "Continue"
                primary.setOnClickListener { confirmationSession = 0; requestConfirmation(state) }
                requestConfirmation(state)
            }
            NativeInstallPhase.DOWNLOADING -> {
                val total = state.totalBytesToDownload()
                val downloaded = state.bytesDownloaded()
                val percent = NativeInstallPolicy.progress(downloaded, total)
                val amount = if (total > 0)
                    "${Formatter.formatFileSize(this, downloaded)} of ${Formatter.formatFileSize(this, total)}"
                    else Formatter.formatFileSize(this, downloaded)
                showWorking(if (percent == 100) "Download complete" else "Downloading tools…",
                    if (percent == 100) "$amount\nWaiting for Google Play to begin installation." else amount,
                    percent = percent)
            }
            NativeInstallPhase.INSTALLING -> {
                showWorking("Installing tools…", "Android is preparing Flow-Like. This can take a few minutes.", stage = 1)
                secondary.visibility = View.GONE
            }
            NativeInstallPhase.INSTALLED -> {
                try {
                    if (NativeRuntimeInstaller.missingModules(this).isEmpty()) openRuntime() else requestDownload()
                } catch (_: Exception) { showError("Android could not open the downloaded tools. Restart Flow-Like and try again.") }
            }
            NativeInstallPhase.CANCELING -> {
                showWorking("Canceling download…", "Waiting for Google Play to stop the download.")
                secondary.isEnabled = false
            }
            NativeInstallPhase.CANCELED -> showPaused()
            NativeInstallPhase.FAILED -> showPlayError(state.errorCode())
            NativeInstallPhase.UNKNOWN -> showError("Google Play could not report download progress. Close Flow-Like and try again.")
        }
    }

    private fun requestConfirmation(state: SplitInstallSessionState) {
        if (!resumed || confirmationSession == state.sessionId() || isFinishing) return
        confirmationSession = state.sessionId()
        try {
            if (!manager.startConfirmationDialogForResult(state, confirmation)) {
                confirmationSession = 0
                showError("Google Play could not open the download confirmation. Retry the download.")
            }
        } catch (_: Exception) {
            confirmationSession = 0
            showError("Google Play could not open the download confirmation. Retry the download.")
        }
    }

    private fun cancelDownload() {
        autoRequest = false
        confirmationSession = 0
        if (sessionId == 0) { showPaused(); return }
        secondary.isEnabled = false
        manager.cancelInstall(sessionId).addOnSuccessListener(this) { showPaused() }
            .addOnFailureListener(this) { showPlayError(it) }
    }

    private fun showWorking(title: String, description: String, stage: Int = 0, percent: Int? = null) {
        if (message.text.toString() != title) message.text = title
        detail.accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_NONE
        detail.text = description
        progress.visibility = View.VISIBLE
        setup.showStage(stage)
        setup.showProgress(percent)
        primary.visibility = View.GONE
        storage.visibility = View.GONE
        secondary.visibility = View.VISIBLE
        secondary.isEnabled = true
        secondary.text = "Cancel download"
        secondary.setOnClickListener { cancelDownload() }
    }

    private fun showPaused() = showError("Setup is paused. Download the tools to start using Flow-Like.", "Continue setup")

    private fun showError(description: String, action: String = "Retry") {
        autoRequest = false
        message.text = "Finish setting up Flow-Like"
        detail.accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        detail.text = description
        setup.showStage(null)
        setup.showProgress(null)
        progress.visibility = View.GONE
        primary.visibility = View.VISIBLE
        primary.isEnabled = true
        primary.text = action
        primary.setOnClickListener { requestDownload() }
        storage.visibility = View.GONE
        secondary.visibility = View.VISIBLE
        secondary.isEnabled = true
        secondary.text = "Close"
        secondary.setOnClickListener { finish() }
    }

    private fun showPlayError(error: Exception) = showPlayError((error as? SplitInstallException)?.errorCode ?: SplitInstallErrorCode.INTERNAL_ERROR)

    private fun showPlayError(code: Int) {
        val description = when (code) {
            SplitInstallErrorCode.NETWORK_ERROR -> "Check your internet connection, then retry. The first setup needs a download from Google Play."
            SplitInstallErrorCode.INSUFFICIENT_STORAGE -> "Android needs more free storage to download and install these tools. Free up space, then retry."
            SplitInstallErrorCode.APP_NOT_OWNED -> "Install Flow-Like from Google Play with your current Google account, or use the complete APK. This small installer cannot download tools on its own."
            SplitInstallErrorCode.PLAY_STORE_NOT_FOUND -> "Google Play is needed to download these tools. Install or enable Google Play, or install the complete Flow-Like APK."
            SplitInstallErrorCode.MODULE_UNAVAILABLE -> "These tools are unavailable for this app version. Update Flow-Like in Google Play, or install the complete APK."
            SplitInstallErrorCode.API_NOT_AVAILABLE -> "Google Play cannot install these tools on this device. Update Google Play, or install the complete Flow-Like APK."
            SplitInstallErrorCode.ACTIVE_SESSIONS_LIMIT_EXCEEDED -> "Google Play is finishing another download. Wait a moment, then retry."
            SplitInstallErrorCode.ACCESS_DENIED -> "Google Play could not start the download. Keep Flow-Like open and retry."
            else -> "Google Play could not finish setup (error $code). Check your connection and available storage, then retry."
        }
        showError(description)
        storage.visibility = if (code == SplitInstallErrorCode.INSUFFICIENT_STORAGE) View.VISIBLE else View.GONE
    }

    private fun openRuntime() {
        if (loading || forwarded) return
        loading = true
        showWorking("Opening Flow-Like…", "Preparing the tools on this device.", stage = 2)
        secondary.visibility = View.GONE
        Thread({
            val error = try {
                NativeRuntimeInstaller.requireFrontendAssets(applicationContext)
                NativeRuntimeInstaller.ensureNativeLoaded(applicationContext)
                null
            }
                catch (failure: Exception) { failure.message ?: "Flow-Like could not open its runtime. Restart the app and try again." }
            runOnUiThread {
                loading = false
                if (isDestroyed || isFinishing) return@runOnUiThread
                if (error != null) { showError(error); return@runOnUiThread }
                prepared = true
                forwardToRuntime()
            }
        }, "flow-like-runtime-loader").start()
    }

    private fun forwardToRuntime() {
        if (!resumed || forwarded || isFinishing || isDestroyed) return
        forwarded = true
        // Copy the full payload, including share ClipData, MIME type, and URI grants.
        val launch = Intent(intent).apply {
            selector = null
            setClass(this@MainActivity, RuntimeActivity::class.java)
            flags = (flags and URI_GRANTS) or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
        }
        startActivity(launch)
        finish()
    }

    private fun relevant(state: SplitInstallSessionState): Boolean = state.moduleNames().any { it in NativeInstallPolicy.modules }

    private fun phase(state: SplitInstallSessionState): NativeInstallPhase = when (state.status()) {
        SplitInstallSessionStatus.PENDING -> NativeInstallPhase.WAITING
        SplitInstallSessionStatus.REQUIRES_USER_CONFIRMATION -> NativeInstallPhase.CONFIRMATION
        SplitInstallSessionStatus.DOWNLOADING -> NativeInstallPhase.DOWNLOADING
        SplitInstallSessionStatus.DOWNLOADED, SplitInstallSessionStatus.INSTALLING -> NativeInstallPhase.INSTALLING
        SplitInstallSessionStatus.INSTALLED -> NativeInstallPhase.INSTALLED
        SplitInstallSessionStatus.CANCELING -> NativeInstallPhase.CANCELING
        SplitInstallSessionStatus.CANCELED -> NativeInstallPhase.CANCELED
        SplitInstallSessionStatus.FAILED -> NativeInstallPhase.FAILED
        else -> NativeInstallPhase.UNKNOWN
    }

    companion object {
        private const val URI_GRANTS = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
            Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or Intent.FLAG_GRANT_PREFIX_URI_PERMISSION
    }
}

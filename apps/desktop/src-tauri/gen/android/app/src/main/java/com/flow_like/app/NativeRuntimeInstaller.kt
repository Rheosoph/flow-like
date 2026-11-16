package com.flow_like.app

import android.content.Context
import android.content.res.AssetManager
import android.util.Log
import com.getkeepsafe.relinker.ReLinker
import com.google.android.play.core.splitcompat.SplitCompat
import com.google.android.play.core.splitinstall.SplitInstallManagerFactory
import java.io.File

/** Loading is shared by the foreground runtime and background credential workers. */
object NativeRuntimeInstaller {
    @Volatile private var nativeLoaded = false
    private val nativeLibraries = listOf("c++_shared", "onnxruntime", "app")

    @Synchronized fun freshContext(context: Context): Context {
        val application = context.applicationContext
        SplitCompat.install(application)
        val fresh = application.createPackageContext(application.packageName, Context.CONTEXT_INCLUDE_CODE)
        SplitCompat.install(fresh)
        // Reapply the asset paths of splits already emulated in this process to the new context.
        SplitCompat.installActivity(fresh)
        // Validate and retain the same manager, even if Android later replaces the context's resources.
        val assets = fresh.assets
        if (!BuildConfig.PLAY_FEATURE_DELIVERY || NativeInstallPolicy.modules.all { hasMarker(assets, it) }) {
            (application as? FlowLikeApplication)?.publishAssetContext(fresh, assets)
        }
        return fresh
    }

    private fun hasMarker(assets: AssetManager, module: String): Boolean = try {
        assets.open("$module/install.marker").use { }
        true
    } catch (_: java.io.IOException) { false }

    fun missingModules(context: Context): Set<String> {
        if (!BuildConfig.PLAY_FEATURE_DELIVERY) return emptySet()
        val fresh = freshContext(context)
        val installed = SplitInstallManagerFactory.create(fresh).installedModules
        // Universal APKs contain the features even though Play reports no installed splits.
        val assets = fresh.assets
        val markers = NativeInstallPolicy.modules.filterTo(mutableSetOf()) { hasMarker(assets, it) }
        return NativeInstallPolicy.missingModules(true, installed, markers)
    }

    fun requireFrontendAssets(context: Context) {
        if (!BuildConfig.PLAY_FEATURE_DELIVERY) return
        freshContext(context)
        val assets = (context.applicationContext as FlowLikeApplication).getFrontendAssets()
        val missing = NativeInstallPolicy.modules.filterNot { hasMarker(assets, it) }
        if (missing.isNotEmpty()) {
            Log.w("FlowLikeDelivery", "Downloaded module assets are unavailable: ${missing.joinToString()}")
            throw IllegalStateException("Android could not open the downloaded tools. Retry setup. If this continues, close Flow-Like and open it again.")
        }
    }

    @JvmStatic @Synchronized fun ensureNativeLoaded(context: Context) {
        if (nativeLoaded) return
        check("runtime" !in missingModules(context)) {
            "Open Flow-Like and finish downloading its runtime before running bots or Events."
        }
        val fresh = freshContext(context)
        try {
            // Load dependencies first so libapp also works when Android extracts a split late.
            val loader = ReLinker.recursively()
            val version = BuildConfig.VERSION_CODE.toString()
            for (library in nativeLibraries) {
                try {
                    loader.loadLibrary(fresh, library, version)
                } catch (error: Throwable) {
                    if (error is LinkageError || error is RuntimeException) {
                        // ReLinker otherwise reuses a partial extraction on the next attempt.
                        try { removeFailedExtraction(fresh.getDir("lib", Context.MODE_PRIVATE), library, version) }
                        catch (_: Exception) { Log.w("FlowLikeDelivery", "Could not discard the failed extraction for $library") }
                    }
                    throw error
                }
            }
            nativeLoaded = true
        } catch (error: LinkageError) {
            throw IllegalStateException("Flow-Like could not load its runtime. Restart the app, or reinstall it from Google Play or the complete APK.", error)
        } catch (error: RuntimeException) {
            throw IllegalStateException("Flow-Like could not prepare its runtime. Check available storage and try again.", error)
        }
    }

    internal fun removeFailedExtraction(directory: File, library: String, version: String): Boolean {
        if (library !in nativeLibraries || version.isEmpty() || version.any { it !in '0'..'9' }) return false
        val extracted = File(directory, "lib$library.so.$version")
        return extracted.isFile && extracted.delete()
    }
}

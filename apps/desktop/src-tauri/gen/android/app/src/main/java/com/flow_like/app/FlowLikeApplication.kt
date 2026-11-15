package com.flow_like.app

import android.content.Context
import android.content.res.AssetManager
import androidx.annotation.Keep
import com.google.android.play.core.splitcompat.SplitCompatApplication

class FlowLikeApplication : SplitCompatApplication() {
    private class InstalledAssets(val context: Context, val assets: AssetManager)

    @Volatile private var installedAssets: InstalledAssets? = null

    // Native editor reads retain validated split assets while Android and WebView manage their own resources.
    @Keep
    fun getFrontendAssets(): AssetManager = installedAssets?.assets ?: super.getAssets()

    internal fun publishAssetContext(context: Context, assets: AssetManager) {
        installedAssets = InstalledAssets(context, assets)
    }

    override fun onCreate() {
        super.onCreate()
        application = this
    }

    companion object {
        @Volatile private var application: FlowLikeApplication? = null

        @JvmStatic fun context(): Context = NativeRuntimeInstaller.freshContext(
            checkNotNull(application) { "Flow-Like has not finished starting." }
        )
    }
}

package com.flow_like.app

import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.system.Os
import android.util.Log
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  private var lastTopPx = 0
  private var lastBottomPx = 0
  private var bridgeAttached = false

  override fun onCreate(savedInstanceState: Bundle?) {
    // Set HOME, TMPDIR, and CACHE_DIR *before* super.onCreate() triggers the
    // native Rust runtime.  Tauri/tao/wry do NOT set HOME on Android, so
    // dirs_next and our own path resolution would otherwise fall back to
    // relative paths rooted at "/" (read-only).
    try {
      val home = filesDir.absolutePath          // e.g. /data/user/0/com.flow_like.app/files
      val cache = cacheDir.absolutePath          // e.g. /data/user/0/com.flow_like.app/cache
      // TMPDIR must live under filesDir so that temp-file → data-file renames
      // stay on the same filesystem/SELinux context (LanceDB commits use
      // std::fs::rename via object_store, which fails across different dirs).
      val tmpDir = java.io.File(filesDir, "tmp")
      tmpDir.mkdirs()
      val tmp = tmpDir.absolutePath
      Os.setenv("HOME", home, true)
      Os.setenv("TMPDIR", tmp, true)
      Os.setenv("LANCE_CACHE_DIR", tmp, true)    // Lance internal temp/cache directory
      Os.setenv("CACHE_DIR", cache, true)        // picked up by get_cache_dir() in core
      Log.d("FlowLike", "setenv HOME=$home  TMPDIR=$tmp  LANCE_CACHE_DIR=$tmp  CACHE_DIR=$cache")
    } catch (e: Exception) {
      Log.e("FlowLike", "Failed to set env vars", e)
    }

    // Stable per-install device id used by the push-target dedup key on the
    // backend. ANDROID_ID is scoped to (signing key, user, device) since
    // Android 8.0 and survives uninstall/reinstall — exactly the property we
    // need so a reinstall reuses the existing PushNotificationTarget row
    // instead of superseding it.
    try {
      val androidId = Settings.Secure.getString(contentResolver, Settings.Secure.ANDROID_ID)
      if (!androidId.isNullOrBlank()) {
        Os.setenv("FL_STABLE_DEVICE_ID", androidId, true)
      }
    } catch (e: Exception) {
      Log.e("FlowLike", "Failed to read ANDROID_ID", e)
    }

    enableEdgeToEdge()
    super.onCreate(savedInstanceState)

    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
      window.attributes.layoutInDisplayCutoutMode =
        android.view.WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
    }

    val rootView = window.decorView
    ViewCompat.setOnApplyWindowInsetsListener(rootView) { _, windowInsets ->
      val systemBars = windowInsets.getInsets(WindowInsetsCompat.Type.systemBars())
      val cutout = windowInsets.getInsets(WindowInsetsCompat.Type.displayCutout())

      val top = maxOf(systemBars.top, cutout.top)
      val bottom = maxOf(systemBars.bottom, cutout.bottom)

      if (top > 0 || bottom > 0) {
        lastTopPx = top
        lastBottomPx = bottom
        // Retry injection: WebView may not be ready yet during initial layout
        scheduleInsetInjection(top, bottom)
      }

      windowInsets
    }

    // Attach the JS bridge so the web page can read insets synchronously.
    // This eliminates the race between evaluateJavascript and page load.
    attachJsBridge()
  }

  inner class InsetBridge {
    @JavascriptInterface
    fun getTopPx(): Int = lastTopPx

    @JavascriptInterface
    fun getBottomPx(): Int = lastBottomPx
  }

  private fun attachJsBridge() {
    if (bridgeAttached) return
    val webView = findWebView(window.decorView)
    if (webView != null) {
      webView.addJavascriptInterface(InsetBridge(), "FlowLikeInsets")
      bridgeAttached = true
      tuneWebView(webView)
      Log.d("FlowLike", "JS inset bridge attached")
    } else {
      window.decorView.postDelayed({ attachJsBridge() }, 50)
    }
  }

  private fun tuneWebView(webView: WebView) {
    webView.settings.apply {
      domStorageEnabled = true
      databaseEnabled = true
      cacheMode = android.webkit.WebSettings.LOAD_DEFAULT
      // Allow mixed content so asset:// and https:// resources work together
      mixedContentMode = android.webkit.WebSettings.MIXED_CONTENT_COMPATIBILITY_MODE
    }
    // Request high-priority rendering for this webview
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      webView.setRendererPriorityPolicy(WebView.RENDERER_PRIORITY_IMPORTANT, false)
    }
    // Ensure hardware acceleration is enabled on the webview layer
    webView.setLayerType(android.view.View.LAYER_TYPE_HARDWARE, null)
  }

  override fun onResume() {
    super.onResume()
    if (lastTopPx > 0 || lastBottomPx > 0) {
      scheduleInsetInjection(lastTopPx, lastBottomPx)
    }
  }

  override fun onWindowFocusChanged(hasFocus: Boolean) {
    super.onWindowFocusChanged(hasFocus)
    if (hasFocus && (lastTopPx > 0 || lastBottomPx > 0)) {
      scheduleInsetInjection(lastTopPx, lastBottomPx)
    }
  }

  private fun scheduleInsetInjection(top: Int, bottom: Int) {
    val delays = longArrayOf(0, 100, 300, 600, 1200, 2500)
    for (delay in delays) {
      window.decorView.postDelayed({
        injectSafeAreaInsets(top, bottom)
      }, delay)
    }
  }

  private fun injectSafeAreaInsets(topPhysical: Int, bottomPhysical: Int) {
    val webView = findWebView(window.decorView)
    if (webView == null) {
      Log.d("FlowLike", "injectSafeAreaInsets: WebView not found yet")
      return
    }
    Log.d("FlowLike", "injectSafeAreaInsets: top=${topPhysical}px bottom=${bottomPhysical}px")
    val js = """
      (function(){
        var dpr=window.devicePixelRatio||1;
        var top=Math.ceil(${topPhysical}/dpr);
        var bottom=Math.ceil(${bottomPhysical}/dpr);
        var d=document.documentElement;
        d.style.setProperty('--fl-native-safe-top',top+'px');
        d.style.setProperty('--fl-native-safe-bottom',bottom+'px');
        window.__FL_NATIVE_SAFE_TOP=top;
        window.__FL_NATIVE_SAFE_BOTTOM=bottom;
      })();
    """.trimIndent()
    webView.evaluateJavascript(js, null)
  }

  private fun findWebView(view: android.view.View): WebView? {
    if (view is WebView) return view
    if (view is android.view.ViewGroup) {
      for (i in 0 until view.childCount) {
        val found = findWebView(view.getChildAt(i))
        if (found != null) return found
      }
    }
    return null
  }
}

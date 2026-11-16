package com.flow_like.app;

import android.app.Activity;
import android.app.Notification;
import android.app.NotificationManager;
import android.content.Context;
import android.content.Intent;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.os.PowerManager;
import android.os.SystemClock;
import android.service.notification.StatusBarNotification;
import android.util.Log;
import android.view.View;
import android.view.ViewGroup;
import android.webkit.WebView;
import android.widget.TextView;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry;
import androidx.test.runner.lifecycle.Stage;
import org.json.JSONObject;
import org.json.JSONTokener;
import org.junit.Test;
import org.junit.runner.RunWith;
import java.io.IOException;
import java.util.Collection;
import java.util.concurrent.Callable;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

/**
 * Run against a complete APK or an app whose Play features have been installed.
 * Pass -e waitForActivitiesToComplete false -e disableAnalytics true to the runner.
 * Tauri exits when RuntimeActivity is destroyed; disabling Activity cleanup preserves result reporting.
 * Java keeps this test independent of Kotlin helpers optimized inside the release APK.
 */
@RunWith(AndroidJUnit4.class)
public class DeliveryStartupTest {
    @Test(timeout = 210_000)
    public void launcherOpensRuntimeAndServesVerifiedEditorAssets() throws Exception {
        long deadline = SystemClock.elapsedRealtime() + 180_000;
        RunningRuntime runtime = launchRuntime(deadline);
        WebView webView = runtime.webView;
        for (String module : new String[]{"runtime", "editors"}) {
            try {
                ((FlowLikeApplication) runtime.application).getFrontendAssets().open(module + "/install.marker").close();
            } catch (IOException error) {
                throw new AssertionError("RuntimeActivity cannot access the installed " + module + " module marker", error);
            }
        }
        String key = "__flowLikeDeliverySmoke_" + SystemClock.elapsedRealtime();
        evaluate(webView, assetProbe(JSONObject.quote(key)), deadline);
        String last = "The asset probe has not reported progress.";
        while (SystemClock.elapsedRealtime() < deadline) {
            JSONObject result = jsonResult(evaluate(webView,
                "JSON.stringify(window[" + JSONObject.quote(key) + "] || null)", deadline));
            if (result != null) {
                last = result.toString();
                if ("done".equals(result.optString("state"))) {
                    assertTrue("Editor asset probe failed: " + result, result.optBoolean("ok"));
                    assertEquals("A missing editor chunk must not fall back to HTML", 404, result.getInt("missingStatus"));
                    for (String name : new String[]{"pdf", "word"}) {
                        JSONObject chunk = result.getJSONObject(name);
                        assertTrue(name + " returned an empty chunk", chunk.getLong("bytes") > 0);
                        assertEquals(name + " returned the wrong chunk size", chunk.getLong("expectedBytes"), chunk.getLong("bytes"));
                        assertEquals(name + " returned the wrong chunk hash", chunk.getString("expectedSha256"), chunk.getString("sha256"));
                    }
                    return;
                }
            }
            Thread.sleep(250);
        }
        throw new AssertionError("Editor asset probe exceeded the 180-second startup deadline. Last result: " + last);
    }

    @Test(timeout = 210_000)
    public void backgroundHostingSurvivesHomeAndScreenOff() throws Throwable {
        long deadline = SystemClock.elapsedRealtime() + 150_000;
        RunningRuntime runtime = launchRuntime(deadline);
        JSONObject initial = invokeHosting(runtime.webView, "native_background_hosting_status", "{}", deadline);
        assertTrue("Android background hosting is unavailable: " + initial, initial.getBoolean("supported"));
        assertTrue("Grant POST_NOTIFICATIONS before this smoke test: " + initial, initial.getBoolean("notificationsEnabled"));
        assertFalse("Background hosting must begin disabled: " + initial, initial.getBoolean("enabled"));
        assertFalse("Background hosting must begin stopped: " + initial, initial.getBoolean("running"));
        Throwable failure = null;
        try {
            JSONObject started = invokeHosting(runtime.webView, "native_set_background_hosting", "{enabled:true}", deadline);
            assertTrue("Hosting did not enable: " + started, started.getBoolean("enabled"));
            assertTrue("Hosting did not reach foreground-service state: " + started, started.getBoolean("running"));
            awaitHostingNotification(runtime.application, true, deadline);
            shell("input keyevent 3", deadline);
            awaitStoppedRuntime(runtime.webView, deadline);
            shell("input keyevent 223", deadline);
            awaitInteractive(runtime.application, false, deadline);
            Thread.sleep(2_000);
            awaitHostingNotification(runtime.application, true, deadline);
            RunningRuntime resumed = wakeAndResume(deadline);
            assertSame("Returning through setup recreated the runtime WebView", runtime.webView, resumed.webView);
            JSONObject running = invokeHosting(resumed.webView, "native_background_hosting_status", "{}", deadline);
            assertTrue("Hosting was disabled by Home or screen-off: " + running, running.getBoolean("enabled"));
            assertTrue("Hosting stopped while the screen was off: " + running, running.getBoolean("running"));
        } catch (Throwable error) {
            failure = error;
        } finally {
            // Always return to a visible Activity before issuing the native stop command.
            long cleanupDeadline = SystemClock.elapsedRealtime() + 45_000;
            try {
                RunningRuntime resumed = wakeAndResume(cleanupDeadline);
                JSONObject stopped = invokeHosting(resumed.webView, "native_set_background_hosting", "{enabled:false}", cleanupDeadline);
                assertFalse("Hosting did not disable: " + stopped, stopped.getBoolean("enabled"));
                assertFalse("Hosting did not stop: " + stopped, stopped.getBoolean("running"));
                awaitHostingNotification(resumed.application, false, cleanupDeadline);
            } catch (Throwable error) {
                Log.e("DeliveryStartupTest", "Background hosting cleanup failed", error);
                if (failure == null) failure = error;
            }
        }
        if (failure != null) throw failure;
    }

    private RunningRuntime launchRuntime(long deadline) throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        onMain(deadline, () -> {
            context.startActivity(new Intent(context, MainActivity.class)
                .setAction(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
            return null;
        });
        return awaitRuntimeWebView(deadline);
    }

    private RunningRuntime wakeAndResume(long deadline) throws Exception {
        shell("input keyevent 224", deadline);
        shell("wm dismiss-keyguard", deadline);
        awaitInteractive(InstrumentationRegistry.getInstrumentation().getTargetContext(), true, deadline);
        return launchRuntime(deadline);
    }

    private JSONObject invokeHosting(WebView webView, String command, String arguments, long deadline) throws Exception {
        long commandDeadline = Math.min(deadline, SystemClock.elapsedRealtime() + 20_000);
        String key = JSONObject.quote("__flowLikeHostingSmoke_" + System.nanoTime());
        String script = "(() => {const key=" + key + ";window[key]={done:false};"
            + "Promise.resolve().then(() => window.__TAURI_INTERNALS__.invoke("
            + JSONObject.quote(command) + "," + arguments + "))"
            + ".then(value => {window[key]={done:true,value};})"
            + ".catch(error => {window[key]={done:true,error:String(error)};});return 'started';})();";
        evaluate(webView, script, commandDeadline);
        while (SystemClock.elapsedRealtime() < commandDeadline) {
            JSONObject result = jsonResult(evaluate(webView, "JSON.stringify(window[" + key + "] || null)", commandDeadline));
            if (result != null && result.optBoolean("done")) {
                assertFalse(command + " failed: " + result.optString("error"), result.has("error"));
                return result.getJSONObject("value");
            }
            Thread.sleep(100);
        }
        throw new AssertionError("Timed out waiting for Tauri command " + command);
    }

    private JSONObject jsonResult(String raw) throws Exception {
        Object decoded = new JSONTokener(raw).nextValue();
        return decoded instanceof String && !"null".equals(decoded) ? new JSONObject((String) decoded) : null;
    }

    private StatusBarNotification hostingNotification(Context context) {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        for (StatusBarNotification notification : manager.getActiveNotifications()) {
            if (notification.getId() == 42011 && context.getPackageName().equals(notification.getPackageName())) return notification;
        }
        return null;
    }

    private void awaitHostingNotification(Context context, boolean present, long deadline) throws Exception {
        long timeout = Math.min(deadline, SystemClock.elapsedRealtime() + 10_000);
        while (SystemClock.elapsedRealtime() < timeout) {
            StatusBarNotification notification = hostingNotification(context);
            if (!present && notification == null) return;
            if (present && notification != null) {
                assertTrue("Hosting notification is not a foreground-service notification",
                    (notification.getNotification().flags & Notification.FLAG_FOREGROUND_SERVICE) != 0);
                if (Build.VERSION.SDK_INT >= 26) assertEquals("background_hosting", notification.getNotification().getChannelId());
                return;
            }
            Thread.sleep(100);
        }
        throw new AssertionError("Hosting notification did not become " + (present ? "visible" : "absent"));
    }

    private void awaitInteractive(Context context, boolean interactive, long deadline) throws Exception {
        PowerManager power = context.getSystemService(PowerManager.class);
        long timeout = Math.min(deadline, SystemClock.elapsedRealtime() + 10_000);
        while (SystemClock.elapsedRealtime() < timeout) {
            if (power.isInteractive() == interactive) return;
            Thread.sleep(100);
        }
        throw new AssertionError("Screen did not become " + (interactive ? "interactive" : "off"));
    }

    private void awaitStoppedRuntime(WebView webView, long deadline) throws Exception {
        long timeout = Math.min(deadline, SystemClock.elapsedRealtime() + 10_000);
        while (SystemClock.elapsedRealtime() < timeout) {
            boolean stopped = onMain(timeout, () -> {
                for (Activity activity : ActivityLifecycleMonitorRegistry.getInstance().getActivitiesInStage(Stage.STOPPED)) {
                    if (activity instanceof RuntimeActivity && findWebView(activity.getWindow().getDecorView()) == webView) return true;
                }
                return false;
            });
            if (stopped) return;
            Thread.sleep(100);
        }
        throw new AssertionError("Home did not stop the existing RuntimeActivity");
    }

    private void shell(String command, long deadline) throws Exception {
        CountDownLatch completed = new CountDownLatch(1);
        AtomicReference<Throwable> error = new AtomicReference<>();
        new Thread(() -> {
            try {
                ParcelFileDescriptor descriptor = InstrumentationRegistry.getInstrumentation().getUiAutomation().executeShellCommand(command);
                ParcelFileDescriptor.AutoCloseInputStream input = new ParcelFileDescriptor.AutoCloseInputStream(descriptor);
                try {
                    byte[] buffer = new byte[1024];
                    while (input.read(buffer) != -1) { }
                } finally { input.close(); }
            } catch (Throwable failure) { error.set(failure); }
            finally { completed.countDown(); }
        }, "delivery-smoke-shell").start();
        awaitCallback(completed, deadline, "shell command: " + command);
        if (error.get() != null) throw new AssertionError("Shell command failed: " + command, error.get());
    }

    private static final class RunningRuntime {
        final WebView webView;
        final Context application;
        RunningRuntime(WebView webView, Context application) {
            this.webView = webView;
            this.application = application;
        }
    }

    private RunningRuntime awaitRuntimeWebView(long deadline) throws Exception {
        AtomicReference<String> last = new AtomicReference<>("No resumed Activity.");
        while (SystemClock.elapsedRealtime() < deadline) {
            RunningRuntime running = onMain(deadline, () -> {
                Collection<Activity> activities = ActivityLifecycleMonitorRegistry.getInstance().getActivitiesInStage(Stage.RESUMED);
                StringBuilder description = new StringBuilder("Resumed activities: ");
                RuntimeActivity runtime = null;
                for (Activity activity : activities) {
                    description.append(activity.getClass().getSimpleName()).append(' ');
                    if (activity instanceof MainActivity) {
                        String setup = setupText(activity.getWindow().getDecorView());
                        description.append("; setup: ").append(setup.substring(0, Math.min(1200, setup.length())));
                    }
                    if (activity instanceof RuntimeActivity) runtime = (RuntimeActivity) activity;
                }
                last.set(description.toString());
                if (runtime == null) return null;
                WebView webView = findWebView(runtime.getWindow().getDecorView());
                return webView == null ? null : new RunningRuntime(webView, runtime.getApplicationContext());
            });
            if (running != null) {
                JSONObject page = jsonResult(evaluate(running.webView,
                    "JSON.stringify({ready:document.readyState,host:location.hostname})", deadline));
                last.set(last.get() + "; WebView: " + page);
                if (page != null) {
                    String ready = page.optString("ready");
                    String host = page.optString("host");
                    if (("interactive".equals(ready) || "complete".equals(ready)) &&
                        ("tauri.localhost".equals(host) || "localhost".equals(host))) return running;
                }
            }
            Thread.sleep(250);
        }
        throw new AssertionError("MainActivity did not reach a resumed RuntimeActivity with a local WebView before the deadline. " + last.get());
    }

    private WebView findWebView(View view) {
        if (view instanceof WebView) return (WebView) view;
        if (view instanceof ViewGroup) {
            ViewGroup group = (ViewGroup) view;
            for (int index = 0; index < group.getChildCount(); index++) {
                WebView found = findWebView(group.getChildAt(index));
                if (found != null) return found;
            }
        }
        return null;
    }

    private String setupText(View view) {
        if (view.getVisibility() != View.VISIBLE) return "";
        if (view instanceof TextView) return ((TextView) view).getText().toString();
        if (view instanceof ViewGroup) {
            StringBuilder text = new StringBuilder();
            ViewGroup group = (ViewGroup) view;
            for (int index = 0; index < group.getChildCount(); index++) {
                String child = setupText(group.getChildAt(index));
                if (!child.isEmpty()) {
                    if (text.length() > 0) text.append("; ");
                    text.append(child);
                }
            }
            return text.toString();
        }
        return "";
    }

    private <T> T onMain(long deadline, Callable<T> block) throws Exception {
        CountDownLatch completed = new CountDownLatch(1);
        AtomicReference<T> value = new AtomicReference<>();
        AtomicReference<Throwable> error = new AtomicReference<>();
        new Handler(Looper.getMainLooper()).post(() -> {
            try { value.set(block.call()); } catch (Throwable failure) { error.set(failure); }
            finally { completed.countDown(); }
        });
        awaitCallback(completed, deadline, "Android main thread");
        if (error.get() != null) throw new AssertionError("Android main-thread inspection failed", error.get());
        return value.get();
    }

    private String evaluate(WebView webView, String script, long deadline) throws Exception {
        CountDownLatch completed = new CountDownLatch(1);
        AtomicReference<String> value = new AtomicReference<>();
        AtomicReference<Throwable> error = new AtomicReference<>();
        new Handler(Looper.getMainLooper()).post(() -> {
            try {
                webView.evaluateJavascript(script, result -> { value.set(result); completed.countDown(); });
            } catch (Throwable failure) { error.set(failure); completed.countDown(); }
        });
        awaitCallback(completed, deadline, "WebView JavaScript callback");
        if (error.get() != null) throw new AssertionError("WebView JavaScript evaluation failed", error.get());
        return value.get() == null ? "null" : value.get();
    }

    private void awaitCallback(CountDownLatch completed, long deadline, String operation) throws Exception {
        long remaining = Math.max(0L, Math.min(10_000L, deadline - SystemClock.elapsedRealtime()));
        assertTrue("Timed out waiting for " + operation + "; the UI thread may be blocked",
            completed.await(remaining, TimeUnit.MILLISECONDS));
    }

    private String assetProbe(String key) {
        return (
            "(() => {\n" +
            "const key = __PROBE_KEY__;\n" +
            "const state = window[key] = {state: 'running', step: 'checking local origin'};\n" +
            "(async () => {\n" +
            "if (!['tauri.localhost', 'localhost'].includes(location.hostname)) {\n" +
            "throw new Error('Refusing to fetch editor assets from a nonlocal origin');\n" +
            "}\n" +
            "if (!globalThis.crypto || !crypto.subtle) throw new Error('WebView crypto.subtle is unavailable');\n" +
            "const localFetch = async (url) => {\n" +
            "if (url.origin !== location.origin) throw new Error('Editor URL escaped the local origin');\n" +
            "return fetch(url.href, {cache: 'no-store', credentials: 'omit', redirect: 'error'});\n" +
            "};\n" +
            "const verify = async (manifestPath) => {\n" +
            "state.step = 'fetching ' + manifestPath;\n" +
            "const manifestUrl = new URL(manifestPath, location.href);\n" +
            "const response = await localFetch(manifestUrl);\n" +
            "if (response.status !== 200) throw new Error(manifestPath + ': HTTP ' + response.status);\n" +
            "const manifest = await response.json();\n" +
            "const part = manifest.wasm && manifest.wasm.parts && manifest.wasm.parts[0];\n" +
            "if (!part || !/^[a-z]+\\.wasm\\.gz\\.\\d{3}$/.test(part.file) ||\n" +
            "!Number.isSafeInteger(part.byteLength) || part.byteLength <= 0 ||\n" +
            "!/^[0-9a-f]{64}$/.test(part.sha256)) {\n" +
            "throw new Error(manifestPath + ': invalid first WASM part');\n" +
            "}\n" +
            "const partUrl = new URL(part.file, manifestUrl);\n" +
            "state.step = 'fetching ' + partUrl.pathname;\n" +
            "const chunk = await localFetch(partUrl);\n" +
            "if (chunk.status !== 200) throw new Error(partUrl.pathname + ': HTTP ' + chunk.status);\n" +
            "const bytes = await chunk.arrayBuffer();\n" +
            "if (bytes.byteLength !== part.byteLength) {\n" +
            "throw new Error(partUrl.pathname + ': expected ' + part.byteLength + ' bytes, got ' + bytes.byteLength);\n" +
            "}\n" +
            "state.step = 'hashing ' + partUrl.pathname;\n" +
            "const hash = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)))\n" +
            ".map(byte => byte.toString(16).padStart(2, '0')).join('');\n" +
            "if (hash !== part.sha256) throw new Error(partUrl.pathname + ': SHA-256 mismatch, got ' + hash);\n" +
            "return {path: partUrl.pathname, bytes: bytes.byteLength, expectedBytes: part.byteLength,\n" +
            "sha256: hash, expectedSha256: part.sha256};\n" +
            "};\n" +
            "const pdf = await verify('/craft/pdfcraft/runtime.json');\n" +
            "const word = await verify('/wordcraft/runtime.json');\n" +
            "state.step = 'checking missing editor chunk';\n" +
            "const missing = await localFetch(new URL('/craft/pdfcraft/craft.wasm.gz.999', location.href));\n" +
            "if (missing.status !== 404) throw new Error('Missing editor chunk returned HTTP ' + missing.status + ', expected 404');\n" +
            "window[key] = {state: 'done', ok: true, pdf, word, missingStatus: missing.status};\n" +
            "})().catch(error => {\n" +
            "window[key] = {state: 'done', ok: false, step: state.step, error: String(error && error.message || error)};\n" +
            "});\n" +
            "return 'started';\n" +
            "})();"
        ).replace("__PROBE_KEY__", key);
    }
}

package com.flowlike.sdk;

import org.teavm.interop.Import;

public final class Host {

    private Host() {}

    // ========================================================================
    // Raw host imports — flowlike_log
    // ========================================================================

    @Import(module = "flowlike_log", name = "trace")
    public static native void hostLogTrace(int msgPtr, int msgLen);

    @Import(module = "flowlike_log", name = "debug")
    public static native void hostLogDebug(int msgPtr, int msgLen);

    @Import(module = "flowlike_log", name = "info")
    public static native void hostLogInfo(int msgPtr, int msgLen);

    @Import(module = "flowlike_log", name = "warn")
    public static native void hostLogWarn(int msgPtr, int msgLen);

    @Import(module = "flowlike_log", name = "error")
    public static native void hostLogError(int msgPtr, int msgLen);

    @Import(module = "flowlike_log", name = "log_json")
    public static native void hostLogJson(int level, int msgPtr, int msgLen, int dataPtr, int dataLen);

    // ========================================================================
    // Raw host imports — flowlike_pins
    // ========================================================================

    @Import(module = "flowlike_pins", name = "get_input")
    public static native long hostGetInput(int namePtr, int nameLen);

    @Import(module = "flowlike_pins", name = "set_output")
    public static native void hostSetOutput(int namePtr, int nameLen, int valPtr, int valLen);

    @Import(module = "flowlike_pins", name = "activate_exec")
    public static native void hostActivateExec(int namePtr, int nameLen);

    // ========================================================================
    // Raw host imports — flowlike_vars
    // ========================================================================

    @Import(module = "flowlike_vars", name = "get")
    public static native long hostVarGet(int namePtr, int nameLen);

    @Import(module = "flowlike_vars", name = "set")
    public static native void hostVarSet(int namePtr, int nameLen, int valPtr, int valLen);

    @Import(module = "flowlike_vars", name = "delete")
    public static native void hostVarDelete(int namePtr, int nameLen);

    @Import(module = "flowlike_vars", name = "has")
    public static native int hostVarHas(int namePtr, int nameLen);

    // ========================================================================
    // Raw host imports — flowlike_cache
    // ========================================================================

    @Import(module = "flowlike_cache", name = "get")
    public static native long hostCacheGet(int keyPtr, int keyLen);

    @Import(module = "flowlike_cache", name = "set")
    public static native void hostCacheSet(int keyPtr, int keyLen, int valPtr, int valLen);

    @Import(module = "flowlike_cache", name = "delete")
    public static native void hostCacheDelete(int keyPtr, int keyLen);

    @Import(module = "flowlike_cache", name = "has")
    public static native int hostCacheHas(int keyPtr, int keyLen);

    // ========================================================================
    // Raw host imports — flowlike_meta
    // ========================================================================

    @Import(module = "flowlike_meta", name = "get_node_id")
    public static native long hostGetNodeId();

    @Import(module = "flowlike_meta", name = "get_run_id")
    public static native long hostGetRunId();

    @Import(module = "flowlike_meta", name = "get_app_id")
    public static native long hostGetAppId();

    @Import(module = "flowlike_meta", name = "get_board_id")
    public static native long hostGetBoardId();

    @Import(module = "flowlike_meta", name = "get_user_id")
    public static native long hostGetUserId();

    @Import(module = "flowlike_meta", name = "is_streaming")
    public static native int hostIsStreaming();

    @Import(module = "flowlike_meta", name = "get_log_level")
    public static native int hostGetLogLevel();

    @Import(module = "flowlike_meta", name = "time_now")
    public static native long hostTimeNow();

    @Import(module = "flowlike_meta", name = "random")
    public static native long hostRandom();

    // ========================================================================
    // Raw host imports — flowlike_storage
    // ========================================================================

    @Import(module = "flowlike_storage", name = "read_request")
    public static native long hostStorageRead(int pathPtr, int pathLen);

    @Import(module = "flowlike_storage", name = "write_request")
    public static native int hostStorageWrite(int pathPtr, int pathLen, int dataPtr, int dataLen);

    @Import(module = "flowlike_storage", name = "storage_dir")
    public static native long hostStorageDir(int nodeScoped);

    @Import(module = "flowlike_storage", name = "upload_dir")
    public static native long hostUploadDir();

    @Import(module = "flowlike_storage", name = "cache_dir")
    public static native long hostCacheDir(int nodeScoped, int userScoped);

    @Import(module = "flowlike_storage", name = "user_dir")
    public static native long hostUserDir(int nodeScoped);

    @Import(module = "flowlike_storage", name = "list_request")
    public static native long hostStorageList(int pathPtr, int pathLen);

    // ========================================================================
    // Raw host imports — flowlike_models
    // ========================================================================

    @Import(module = "flowlike_models", name = "embed_text")
    public static native long hostEmbedText(int bitPtr, int bitLen, int textsPtr, int textsLen);

    // ========================================================================
    // Raw host imports — flowlike_http
    // ========================================================================

    @Import(module = "flowlike_http", name = "request")
    public static native int hostHttpRequest(int method, int urlPtr, int urlLen,
                                              int headersPtr, int headersLen,
                                              int bodyPtr, int bodyLen);

    // ========================================================================
    // Raw host imports — flowlike_stream
    // ========================================================================

    @Import(module = "flowlike_stream", name = "emit")
    public static native void hostStreamEmit(int eventPtr, int eventLen, int dataPtr, int dataLen);

    @Import(module = "flowlike_stream", name = "text")
    public static native void hostStreamText(int textPtr, int textLen);

    // ========================================================================
    // Raw host imports — flowlike_auth
    // ========================================================================

    @Import(module = "flowlike_auth", name = "get_oauth_token")
    public static native long hostGetOAuthToken(int providerPtr, int providerLen);

    @Import(module = "flowlike_auth", name = "has_oauth_token")
    public static native int hostHasOAuthToken(int providerPtr, int providerLen);

    // ========================================================================
    // High-level wrapper methods
    // ========================================================================

    public static void logTrace(String msg) {
        int[] pl = Memory.stringToPtrLen(msg);
        hostLogTrace(pl[0], pl[1]);
    }

    public static void logDebug(String msg) {
        int[] pl = Memory.stringToPtrLen(msg);
        hostLogDebug(pl[0], pl[1]);
    }

    public static void logInfo(String msg) {
        int[] pl = Memory.stringToPtrLen(msg);
        hostLogInfo(pl[0], pl[1]);
    }

    public static void logWarn(String msg) {
        int[] pl = Memory.stringToPtrLen(msg);
        hostLogWarn(pl[0], pl[1]);
    }

    public static void logError(String msg) {
        int[] pl = Memory.stringToPtrLen(msg);
        hostLogError(pl[0], pl[1]);
    }

    public static void logJson(int level, String msg, String data) {
        int[] mp = Memory.stringToPtrLen(msg);
        int[] dp = Memory.stringToPtrLen(data);
        hostLogJson(level, mp[0], mp[1], dp[0], dp[1]);
    }

    public static String getInput(String name) {
        int[] pl = Memory.stringToPtrLen(name);
        return Memory.unpackString(hostGetInput(pl[0], pl[1]));
    }

    public static void setOutput(String name, String value) {
        int[] np = Memory.stringToPtrLen(name);
        int[] vp = Memory.stringToPtrLen(value);
        hostSetOutput(np[0], np[1], vp[0], vp[1]);
    }

    public static void activateExec(String name) {
        int[] pl = Memory.stringToPtrLen(name);
        hostActivateExec(pl[0], pl[1]);
    }

    public static String getVariable(String name) {
        int[] pl = Memory.stringToPtrLen(name);
        return Memory.unpackString(hostVarGet(pl[0], pl[1]));
    }

    public static void setVariable(String name, String value) {
        int[] np = Memory.stringToPtrLen(name);
        int[] vp = Memory.stringToPtrLen(value);
        hostVarSet(np[0], np[1], vp[0], vp[1]);
    }

    public static void deleteVariable(String name) {
        int[] pl = Memory.stringToPtrLen(name);
        hostVarDelete(pl[0], pl[1]);
    }

    public static boolean hasVariable(String name) {
        int[] pl = Memory.stringToPtrLen(name);
        return hostVarHas(pl[0], pl[1]) != 0;
    }

    public static String cacheGet(String key) {
        int[] pl = Memory.stringToPtrLen(key);
        return Memory.unpackString(hostCacheGet(pl[0], pl[1]));
    }

    public static void cacheSet(String key, String value) {
        int[] kp = Memory.stringToPtrLen(key);
        int[] vp = Memory.stringToPtrLen(value);
        hostCacheSet(kp[0], kp[1], vp[0], vp[1]);
    }

    public static void cacheDelete(String key) {
        int[] pl = Memory.stringToPtrLen(key);
        hostCacheDelete(pl[0], pl[1]);
    }

    public static boolean cacheHas(String key) {
        int[] pl = Memory.stringToPtrLen(key);
        return hostCacheHas(pl[0], pl[1]) != 0;
    }

    public static String getNodeId() { return Memory.unpackString(hostGetNodeId()); }
    public static String getRunId() { return Memory.unpackString(hostGetRunId()); }
    public static String getAppId() { return Memory.unpackString(hostGetAppId()); }
    public static String getBoardId() { return Memory.unpackString(hostGetBoardId()); }
    public static String getUserId() { return Memory.unpackString(hostGetUserId()); }

    public static boolean isStreaming() { return hostIsStreaming() != 0; }
    public static int getLogLevel() { return hostGetLogLevel(); }
    public static long timeNow() { return hostTimeNow(); }
    public static long random() { return hostRandom(); }

    public static String storageRead(String path) {
        int[] pl = Memory.stringToPtrLen(path);
        return Memory.unpackString(hostStorageRead(pl[0], pl[1]));
    }

    public static boolean storageWrite(String path, String data) {
        int[] pp = Memory.stringToPtrLen(path);
        int[] dp = Memory.stringToPtrLen(data);
        return hostStorageWrite(pp[0], pp[1], dp[0], dp[1]) != 0;
    }

    public static String storageDir(boolean nodeScoped) {
        return Memory.unpackString(hostStorageDir(nodeScoped ? 1 : 0));
    }

    public static String uploadDir() { return Memory.unpackString(hostUploadDir()); }

    public static String cacheDir(boolean nodeScoped, boolean userScoped) {
        return Memory.unpackString(hostCacheDir(nodeScoped ? 1 : 0, userScoped ? 1 : 0));
    }

    public static String userDir(boolean nodeScoped) {
        return Memory.unpackString(hostUserDir(nodeScoped ? 1 : 0));
    }

    public static String storageList(String flowPathJson) {
        int[] pl = Memory.stringToPtrLen(flowPathJson);
        return Memory.unpackString(hostStorageList(pl[0], pl[1]));
    }

    public static String embedText(String bitJson, String textsJson) {
        int[] bp = Memory.stringToPtrLen(bitJson);
        int[] tp = Memory.stringToPtrLen(textsJson);
        return Memory.unpackString(hostEmbedText(bp[0], bp[1], tp[0], tp[1]));
    }

    public static boolean httpRequest(int method, String url, String headers, String body) {
        int[] up = Memory.stringToPtrLen(url);
        int[] hp = Memory.stringToPtrLen(headers);
        int[] bp = Memory.stringToPtrLen(body);
        return hostHttpRequest(method, up[0], up[1], hp[0], hp[1], bp[0], bp[1]) != 0;
    }

    public static void streamEmit(String eventType, String data) {
        int[] ep = Memory.stringToPtrLen(eventType);
        int[] dp = Memory.stringToPtrLen(data);
        hostStreamEmit(ep[0], ep[1], dp[0], dp[1]);
    }

    public static void streamText(String text) {
        int[] pl = Memory.stringToPtrLen(text);
        hostStreamText(pl[0], pl[1]);
    }

    public static String getOAuthToken(String provider) {
        int[] pl = Memory.stringToPtrLen(provider);
        return Memory.unpackString(hostGetOAuthToken(pl[0], pl[1]));
    }

    public static boolean hasOAuthToken(String provider) {
        int[] pl = Memory.stringToPtrLen(provider);
        return hostHasOAuthToken(pl[0], pl[1]) != 0;
    }
}

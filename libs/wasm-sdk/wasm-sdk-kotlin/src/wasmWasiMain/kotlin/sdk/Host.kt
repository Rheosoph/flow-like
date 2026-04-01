package sdk

// ============================================================================
// flowlike_log
// ============================================================================

@WasmImport("flowlike_log", "debug")
private external fun hostLogDebug(ptr: Int, len: Int)

@WasmImport("flowlike_log", "info")
private external fun hostLogInfo(ptr: Int, len: Int)

@WasmImport("flowlike_log", "warn")
private external fun hostLogWarn(ptr: Int, len: Int)

@WasmImport("flowlike_log", "error")
private external fun hostLogError(ptr: Int, len: Int)

@WasmImport("flowlike_log", "trace")
private external fun hostLogTrace(ptr: Int, len: Int)

@WasmImport("flowlike_log", "log_json")
private external fun hostLogJson(level: Int, msgPtr: Int, msgLen: Int, dataPtr: Int, dataLen: Int)

// ============================================================================
// flowlike_pins
// ============================================================================

@WasmImport("flowlike_pins", "get_input")
private external fun hostGetInput(namePtr: Int, nameLen: Int): Long

@WasmImport("flowlike_pins", "set_output")
private external fun hostSetOutput(namePtr: Int, nameLen: Int, valuePtr: Int, valueLen: Int)

@WasmImport("flowlike_pins", "activate_exec")
private external fun hostActivateExec(namePtr: Int, nameLen: Int)

// ============================================================================
// flowlike_vars
// ============================================================================

@WasmImport("flowlike_vars", "get")
private external fun hostVarGet(namePtr: Int, nameLen: Int): Long

@WasmImport("flowlike_vars", "set")
private external fun hostVarSet(namePtr: Int, nameLen: Int, valuePtr: Int, valueLen: Int)

@WasmImport("flowlike_vars", "delete")
private external fun hostVarDelete(namePtr: Int, nameLen: Int)

@WasmImport("flowlike_vars", "has")
private external fun hostVarHas(namePtr: Int, nameLen: Int): Int

// ============================================================================
// flowlike_cache
// ============================================================================

@WasmImport("flowlike_cache", "get")
private external fun hostCacheGet(keyPtr: Int, keyLen: Int): Long

@WasmImport("flowlike_cache", "set")
private external fun hostCacheSet(keyPtr: Int, keyLen: Int, valPtr: Int, valLen: Int)

@WasmImport("flowlike_cache", "delete")
private external fun hostCacheDelete(keyPtr: Int, keyLen: Int)

@WasmImport("flowlike_cache", "has")
private external fun hostCacheHas(keyPtr: Int, keyLen: Int): Int

// ============================================================================
// flowlike_meta
// ============================================================================

@WasmImport("flowlike_meta", "get_node_id")
private external fun hostGetNodeId(): Long

@WasmImport("flowlike_meta", "get_run_id")
private external fun hostGetRunId(): Long

@WasmImport("flowlike_meta", "get_app_id")
private external fun hostGetAppId(): Long

@WasmImport("flowlike_meta", "get_board_id")
private external fun hostGetBoardId(): Long

@WasmImport("flowlike_meta", "get_user_id")
private external fun hostGetUserId(): Long

@WasmImport("flowlike_meta", "is_streaming")
private external fun hostIsStreaming(): Int

@WasmImport("flowlike_meta", "get_log_level")
private external fun hostGetLogLevel(): Int

@WasmImport("flowlike_meta", "time_now")
private external fun hostTimeNow(): Long

@WasmImport("flowlike_meta", "random")
private external fun hostRandom(): Long

// ============================================================================
// flowlike_stream
// ============================================================================

@WasmImport("flowlike_stream", "emit")
private external fun hostStreamEmit(eventPtr: Int, eventLen: Int, dataPtr: Int, dataLen: Int)

@WasmImport("flowlike_stream", "text")
private external fun hostStreamText(textPtr: Int, textLen: Int)

// ============================================================================
// flowlike_storage
// ============================================================================

@WasmImport("flowlike_storage", "read_request")
private external fun hostStorageRead(pathPtr: Int, pathLen: Int): Long

@WasmImport("flowlike_storage", "write_request")
private external fun hostStorageWrite(pathPtr: Int, pathLen: Int, dataPtr: Int, dataLen: Int): Int

@WasmImport("flowlike_storage", "storage_dir")
private external fun hostStorageDir(nodeScoped: Int): Long

@WasmImport("flowlike_storage", "upload_dir")
private external fun hostUploadDir(): Long

@WasmImport("flowlike_storage", "cache_dir")
private external fun hostCacheDir(nodeScoped: Int, userScoped: Int): Long

@WasmImport("flowlike_storage", "user_dir")
private external fun hostUserDir(nodeScoped: Int): Long

@WasmImport("flowlike_storage", "list_request")
private external fun hostStorageList(pathPtr: Int, pathLen: Int): Long

// ============================================================================
// flowlike_models
// ============================================================================

@WasmImport("flowlike_models", "embed_text")
private external fun hostEmbedText(bitPtr: Int, bitLen: Int, textsPtr: Int, textsLen: Int): Long

// ============================================================================
// flowlike_http
// ============================================================================

@WasmImport("flowlike_http", "request")
private external fun hostHttpRequest(
    method: Int,
    urlPtr: Int, urlLen: Int,
    headersPtr: Int, headersLen: Int,
    bodyPtr: Int, bodyLen: Int,
): Int

// ============================================================================
// flowlike_auth
// ============================================================================

@WasmImport("flowlike_auth", "get_oauth_token")
private external fun hostGetOauthToken(providerPtr: Int, providerLen: Int): Long

@WasmImport("flowlike_auth", "has_oauth_token")
private external fun hostHasOauthToken(providerPtr: Int, providerLen: Int): Int

// ============================================================================
// Wrapper functions
// ============================================================================

// -- Logging --

fun logDebug(message: String) {
    val (ptr, len) = stringToPtr(message)
    hostLogDebug(ptr, len)
}

fun logInfo(message: String) {
    val (ptr, len) = stringToPtr(message)
    hostLogInfo(ptr, len)
}

fun logWarn(message: String) {
    val (ptr, len) = stringToPtr(message)
    hostLogWarn(ptr, len)
}

fun logError(message: String) {
    val (ptr, len) = stringToPtr(message)
    hostLogError(ptr, len)
}

fun logTrace(message: String) {
    val (ptr, len) = stringToPtr(message)
    hostLogTrace(ptr, len)
}

fun logJson(level: Int, message: String, data: String) {
    val (msgPtr, msgLen) = stringToPtr(message)
    val (dataPtr, dataLen) = stringToPtr(data)
    hostLogJson(level, msgPtr, msgLen, dataPtr, dataLen)
}

// -- Pins --

fun getInput(name: String): String? {
    val (ptr, len) = stringToPtr(name)
    val result = hostGetInput(ptr, len)
    return unpackString(result)
}

fun setOutput(name: String, value: String) {
    val (namePtr, nameLen) = stringToPtr(name)
    val (valPtr, valLen) = stringToPtr(value)
    hostSetOutput(namePtr, nameLen, valPtr, valLen)
}

fun activateExec(name: String) {
    val (ptr, len) = stringToPtr(name)
    hostActivateExec(ptr, len)
}

// -- Variables --

fun getVariable(name: String): String? {
    val (ptr, len) = stringToPtr(name)
    return unpackString(hostVarGet(ptr, len))
}

fun setVariable(name: String, value: String) {
    val (namePtr, nameLen) = stringToPtr(name)
    val (valPtr, valLen) = stringToPtr(value)
    hostVarSet(namePtr, nameLen, valPtr, valLen)
}

fun deleteVariable(name: String) {
    val (ptr, len) = stringToPtr(name)
    hostVarDelete(ptr, len)
}

fun hasVariable(name: String): Boolean {
    val (ptr, len) = stringToPtr(name)
    return hostVarHas(ptr, len) != 0
}

// -- Cache --

fun cacheGet(key: String): String? {
    val (ptr, len) = stringToPtr(key)
    return unpackString(hostCacheGet(ptr, len))
}

fun cacheSet(key: String, value: String) {
    val (keyPtr, keyLen) = stringToPtr(key)
    val (valPtr, valLen) = stringToPtr(value)
    hostCacheSet(keyPtr, keyLen, valPtr, valLen)
}

fun cacheDelete(key: String) {
    val (ptr, len) = stringToPtr(key)
    hostCacheDelete(ptr, len)
}

fun cacheHas(key: String): Boolean {
    val (ptr, len) = stringToPtr(key)
    return hostCacheHas(ptr, len) != 0
}

// -- Stream --

fun stream(eventType: String, data: String) {
    val (evtPtr, evtLen) = stringToPtr(eventType)
    val (dataPtr, dataLen) = stringToPtr(data)
    hostStreamEmit(evtPtr, evtLen, dataPtr, dataLen)
}

fun streamTextRaw(text: String) {
    val (ptr, len) = stringToPtr(text)
    hostStreamText(ptr, len)
}

fun streamText(text: String) {
    stream("text", text)
}

// -- Meta --

fun getNodeId(): String? = unpackString(hostGetNodeId())
fun getRunId(): String? = unpackString(hostGetRunId())
fun getAppId(): String? = unpackString(hostGetAppId())
fun getBoardId(): String? = unpackString(hostGetBoardId())
fun getUserId(): String? = unpackString(hostGetUserId())
fun isStreaming(): Boolean = hostIsStreaming() != 0
fun getLogLevel(): Int = hostGetLogLevel()
fun timeNow(): Long = hostTimeNow()
fun random(): Long = hostRandom()

// -- Storage --

fun storageRead(path: String): String? {
    val (ptr, len) = stringToPtr(path)
    return unpackString(hostStorageRead(ptr, len))
}

fun storageWrite(path: String, data: String): Boolean {
    val (pathPtr, pathLen) = stringToPtr(path)
    val (dataPtr, dataLen) = stringToPtr(data)
    return hostStorageWrite(pathPtr, pathLen, dataPtr, dataLen) != 0
}

fun storageDir(nodeScoped: Boolean): String? =
    unpackString(hostStorageDir(if (nodeScoped) 1 else 0))

fun uploadDir(): String? = unpackString(hostUploadDir())

fun cacheDir(nodeScoped: Boolean, userScoped: Boolean): String? =
    unpackString(hostCacheDir(if (nodeScoped) 1 else 0, if (userScoped) 1 else 0))

fun userDir(nodeScoped: Boolean): String? =
    unpackString(hostUserDir(if (nodeScoped) 1 else 0))

fun storageList(path: String): String? {
    val (ptr, len) = stringToPtr(path)
    return unpackString(hostStorageList(ptr, len))
}

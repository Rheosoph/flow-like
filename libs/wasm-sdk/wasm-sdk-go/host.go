package sdk

// ============================================================================
// Host Imports — flowlike_log
// ============================================================================

//go:wasmimport flowlike_log trace
func hostLogTrace(ptr uint32, len uint32)

//go:wasmimport flowlike_log debug
func hostLogDebug(ptr uint32, len uint32)

//go:wasmimport flowlike_log info
func hostLogInfo(ptr uint32, len uint32)

//go:wasmimport flowlike_log warn
func hostLogWarn(ptr uint32, len uint32)

//go:wasmimport flowlike_log error
func hostLogError(ptr uint32, len uint32)

//go:wasmimport flowlike_log log_json
func hostLogJSON(level int32, msgPtr uint32, msgLen uint32, dataPtr uint32, dataLen uint32)

// ============================================================================
// Host Imports — flowlike_pins
// ============================================================================

//go:wasmimport flowlike_pins get_input
func hostGetInput(namePtr uint32, nameLen uint32) int64

//go:wasmimport flowlike_pins set_output
func hostSetOutput(namePtr uint32, nameLen uint32, valPtr uint32, valLen uint32)

//go:wasmimport flowlike_pins activate_exec
func hostActivateExec(namePtr uint32, nameLen uint32)

// ============================================================================
// Host Imports — flowlike_vars
// ============================================================================

//go:wasmimport flowlike_vars get
func hostVarGet(namePtr uint32, nameLen uint32) int64

//go:wasmimport flowlike_vars set
func hostVarSet(namePtr uint32, nameLen uint32, valPtr uint32, valLen uint32)

//go:wasmimport flowlike_vars delete
func hostVarDelete(namePtr uint32, nameLen uint32)

//go:wasmimport flowlike_vars has
func hostVarHas(namePtr uint32, nameLen uint32) int32

// ============================================================================
// Host Imports — flowlike_cache
// ============================================================================

//go:wasmimport flowlike_cache get
func hostCacheGet(keyPtr uint32, keyLen uint32) int64

//go:wasmimport flowlike_cache set
func hostCacheSet(keyPtr uint32, keyLen uint32, valPtr uint32, valLen uint32)

//go:wasmimport flowlike_cache delete
func hostCacheDelete(keyPtr uint32, keyLen uint32)

//go:wasmimport flowlike_cache has
func hostCacheHas(keyPtr uint32, keyLen uint32) int32

// ============================================================================
// Host Imports — flowlike_meta
// ============================================================================

//go:wasmimport flowlike_meta get_node_id
func hostGetNodeID() int64

//go:wasmimport flowlike_meta get_run_id
func hostGetRunID() int64

//go:wasmimport flowlike_meta get_app_id
func hostGetAppID() int64

//go:wasmimport flowlike_meta get_board_id
func hostGetBoardID() int64

//go:wasmimport flowlike_meta get_user_id
func hostGetUserID() int64

//go:wasmimport flowlike_meta is_streaming
func hostIsStreaming() int32

//go:wasmimport flowlike_meta get_log_level
func hostGetLogLevel() int32

//go:wasmimport flowlike_meta time_now
func hostTimeNow() int64

//go:wasmimport flowlike_meta random
func hostRandom() int64

// ============================================================================
// Host Imports — flowlike_storage
// ============================================================================

//go:wasmimport flowlike_storage read_request
func hostStorageRead(pathPtr uint32, pathLen uint32) int64

//go:wasmimport flowlike_storage write_request
func hostStorageWrite(pathPtr uint32, pathLen uint32, dataPtr uint32, dataLen uint32) int32

//go:wasmimport flowlike_storage storage_dir
func hostStorageDir(nodeScoped int32) int64

//go:wasmimport flowlike_storage upload_dir
func hostUploadDir() int64

//go:wasmimport flowlike_storage cache_dir
func hostCacheDir(nodeScoped int32, userScoped int32) int64

//go:wasmimport flowlike_storage user_dir
func hostUserDir(nodeScoped int32) int64

//go:wasmimport flowlike_storage list_request
func hostStorageList(pathPtr uint32, pathLen uint32) int64

// ============================================================================
// Host Imports — flowlike_models
// ============================================================================

//go:wasmimport flowlike_models embed_text
func hostEmbedText(bitPtr uint32, bitLen uint32, textsPtr uint32, textsLen uint32) int64

// ============================================================================
// Host Imports — flowlike_http
// ============================================================================

//go:wasmimport flowlike_http request
func hostHTTPRequest(method int32, urlPtr uint32, urlLen uint32, headersPtr uint32, headersLen uint32, bodyPtr uint32, bodyLen uint32) int32

// ============================================================================
// Host Imports — flowlike_stream
// ============================================================================

//go:wasmimport flowlike_stream emit
func hostStreamEmit(eventPtr uint32, eventLen uint32, dataPtr uint32, dataLen uint32)

//go:wasmimport flowlike_stream text
func hostStreamText(textPtr uint32, textLen uint32)

// ============================================================================
// Host Imports — flowlike_auth
// ============================================================================

//go:wasmimport flowlike_auth get_oauth_token
func hostGetOAuthToken(providerPtr uint32, providerLen uint32) int64

//go:wasmimport flowlike_auth has_oauth_token
func hostHasOAuthToken(providerPtr uint32, providerLen uint32) int32

// ============================================================================
// Go wrapper functions
// ============================================================================

func LogTrace(msg string) {
	p, l := stringToPtr(msg)
	hostLogTrace(p, l)
}

func LogDebug(msg string) {
	p, l := stringToPtr(msg)
	hostLogDebug(p, l)
}

func LogInfo(msg string) {
	p, l := stringToPtr(msg)
	hostLogInfo(p, l)
}

func LogWarn(msg string) {
	p, l := stringToPtr(msg)
	hostLogWarn(p, l)
}

func LogError(msg string) {
	p, l := stringToPtr(msg)
	hostLogError(p, l)
}

func LogJSON(level int, msg, data string) {
	mp, ml := stringToPtr(msg)
	dp, dl := stringToPtr(data)
	hostLogJSON(int32(level), mp, ml, dp, dl)
}

func GetInput(name string) string {
	p, l := stringToPtr(name)
	return unpackString(hostGetInput(p, l))
}

func SetOutput(name, value string) {
	np, nl := stringToPtr(name)
	vp, vl := stringToPtr(value)
	hostSetOutput(np, nl, vp, vl)
}

func ActivateExec(name string) {
	p, l := stringToPtr(name)
	hostActivateExec(p, l)
}

func GetVariable(name string) string {
	p, l := stringToPtr(name)
	return unpackString(hostVarGet(p, l))
}

func SetVariable(name, value string) {
	np, nl := stringToPtr(name)
	vp, vl := stringToPtr(value)
	hostVarSet(np, nl, vp, vl)
}

func DeleteVariable(name string) {
	p, l := stringToPtr(name)
	hostVarDelete(p, l)
}

func HasVariable(name string) bool {
	p, l := stringToPtr(name)
	return hostVarHas(p, l) != 0
}

func CacheGet(key string) string {
	p, l := stringToPtr(key)
	return unpackString(hostCacheGet(p, l))
}

func CacheSet(key, value string) {
	kp, kl := stringToPtr(key)
	vp, vl := stringToPtr(value)
	hostCacheSet(kp, kl, vp, vl)
}

func CacheDelete(key string) {
	p, l := stringToPtr(key)
	hostCacheDelete(p, l)
}

func CacheHas(key string) bool {
	p, l := stringToPtr(key)
	return hostCacheHas(p, l) != 0
}

func GetNodeID() string  { return unpackString(hostGetNodeID()) }
func GetRunID() string   { return unpackString(hostGetRunID()) }
func GetAppID() string   { return unpackString(hostGetAppID()) }
func GetBoardID() string { return unpackString(hostGetBoardID()) }
func GetUserID() string  { return unpackString(hostGetUserID()) }

func IsStreaming() bool    { return hostIsStreaming() != 0 }
func GetLogLevel() int     { return int(hostGetLogLevel()) }
func TimeNow() int64       { return hostTimeNow() }
func Random() int64         { return hostRandom() }

func StorageRead(path string) string {
	p, l := stringToPtr(path)
	return unpackString(hostStorageRead(p, l))
}

func StorageWrite(path string, data string) bool {
	pp, pl := stringToPtr(path)
	dp, dl := stringToPtr(data)
	return hostStorageWrite(pp, pl, dp, dl) != 0
}

func StorageDir(nodeScoped bool) string {
	v := int32(0)
	if nodeScoped {
		v = 1
	}
	return unpackString(hostStorageDir(v))
}

func UploadDir() string { return unpackString(hostUploadDir()) }

func CacheDirPath(nodeScoped, userScoped bool) string {
	n, u := int32(0), int32(0)
	if nodeScoped {
		n = 1
	}
	if userScoped {
		u = 1
	}
	return unpackString(hostCacheDir(n, u))
}

func UserDir(nodeScoped bool) string {
	v := int32(0)
	if nodeScoped {
		v = 1
	}
	return unpackString(hostUserDir(v))
}

func StorageList(flowPathJSON string) string {
	p, l := stringToPtr(flowPathJSON)
	return unpackString(hostStorageList(p, l))
}

func EmbedText(bitJSON, textsJSON string) string {
	bp, bl := stringToPtr(bitJSON)
	tp, tl := stringToPtr(textsJSON)
	return unpackString(hostEmbedText(bp, bl, tp, tl))
}

func HTTPRequest(method int, url, headers, body string) bool {
	up, ul := stringToPtr(url)
	hp, hl := stringToPtr(headers)
	bp, bl := stringToPtr(body)
	return hostHTTPRequest(int32(method), up, ul, hp, hl, bp, bl) != 0
}

func StreamEmit(eventType, data string) {
	ep, el := stringToPtr(eventType)
	dp, dl := stringToPtr(data)
	hostStreamEmit(ep, el, dp, dl)
}

func StreamText(text string) {
	p, l := stringToPtr(text)
	hostStreamText(p, l)
}

func GetOAuthToken(provider string) string {
	p, l := stringToPtr(provider)
	return unpackString(hostGetOAuthToken(p, l))
}

func HasOAuthToken(provider string) bool {
	p, l := stringToPtr(provider)
	return hostHasOAuthToken(p, l) != 0
}

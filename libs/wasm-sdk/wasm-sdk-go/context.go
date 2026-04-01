package sdk

import (
	"strconv"
	"strings"
)

type Context struct {
	input   ExecutionInput
	result  ExecutionResult
	outputs map[string]string
}

func NewContext(input ExecutionInput) *Context {
	return &Context{
		input:   input,
		result:  SuccessResult(),
		outputs: make(map[string]string),
	}
}

// --- Metadata ---

func (c *Context) NodeID() string      { return c.input.NodeID }
func (c *Context) NodeName() string    { return c.input.NodeName }
func (c *Context) RunID() string       { return c.input.RunID }
func (c *Context) AppID() string       { return c.input.AppID }
func (c *Context) BoardID() string     { return c.input.BoardID }
func (c *Context) UserID() string      { return c.input.UserID }
func (c *Context) StreamEnabled() bool { return c.input.StreamState }
func (c *Context) LogLevelValue() uint8 { return c.input.LogLevel }

// --- Input getters ---

func (c *Context) GetInput(name string) (string, bool) {
	v, ok := c.input.Inputs[name]
	return v, ok
}

func (c *Context) GetString(name, defaultValue string) string {
	v, ok := c.input.Inputs[name]
	if !ok {
		return defaultValue
	}
	if len(v) >= 2 && v[0] == '"' && v[len(v)-1] == '"' {
		return v[1 : len(v)-1]
	}
	return v
}

func (c *Context) GetI64(name string, defaultValue int64) int64 {
	v, ok := c.input.Inputs[name]
	if !ok {
		return defaultValue
	}
	n, err := strconv.ParseInt(v, 10, 64)
	if err != nil {
		return defaultValue
	}
	return n
}

func (c *Context) GetF64(name string, defaultValue float64) float64 {
	v, ok := c.input.Inputs[name]
	if !ok {
		return defaultValue
	}
	f, err := strconv.ParseFloat(v, 64)
	if err != nil {
		return defaultValue
	}
	return f
}

func (c *Context) GetBool(name string, defaultValue bool) bool {
	v, ok := c.input.Inputs[name]
	if !ok {
		return defaultValue
	}
	return v == "true"
}

// --- Output setters ---

func (c *Context) SetOutput(name, value string) {
	c.outputs[name] = value
}

func (c *Context) ActivateExec(pinName string) {
	c.result.ActivateExec = append(c.result.ActivateExec, pinName)
}

func (c *Context) SetPending(pending bool) {
	c.result.Pending = pending
}

func (c *Context) SetError(err string) {
	c.result.Error = &err
}

// --- Level-gated logging ---

func (c *Context) shouldLog(level int) bool {
	return level >= int(c.input.LogLevel)
}

func (c *Context) Debug(msg string) {
	if c.shouldLog(LogLevelDebug) {
		LogDebug(msg)
	}
}

func (c *Context) Info(msg string) {
	if c.shouldLog(LogLevelInfo) {
		LogInfo(msg)
	}
}

func (c *Context) Warn(msg string) {
	if c.shouldLog(LogLevelWarn) {
		LogWarn(msg)
	}
}

func (c *Context) Error(msg string) {
	if c.shouldLog(LogLevelError) {
		LogError(msg)
	}
}

// --- Conditional streaming ---

func (c *Context) StreamText(text string) {
	if c.StreamEnabled() {
		StreamText(text)
	}
}

func (c *Context) StreamJSON(data string) {
	if c.StreamEnabled() {
		StreamEmit("json", data)
	}
}

func (c *Context) StreamProgress(progress float32, message string) {
	if c.StreamEnabled() {
		var b strings.Builder
		b.WriteString(`{"progress":`)
		b.WriteString(strconv.FormatFloat(float64(progress), 'f', -1, 32))
		b.WriteString(`,"message":"`)
		b.WriteString(message)
		b.WriteString(`"}`)
		StreamEmit("progress", b.String())
	}
}

// --- Cache ---

func (c *Context) CacheGet(key string) string        { return CacheGet(key) }
func (c *Context) CacheSet(key, value string)        { CacheSet(key, value) }
func (c *Context) CacheDelete(key string)            { CacheDelete(key) }
func (c *Context) CacheHas(key string) bool          { return CacheHas(key) }

// --- Variables ---

func (c *Context) GetVariable(name string) string {
	return GetVariable(name)
}

func (c *Context) SetVariable(name, value string) {
	SetVariable(name, value)
}

func (c *Context) DeleteVariable(name string)        { DeleteVariable(name) }
func (c *Context) HasVariable(name string) bool      { return HasVariable(name) }

// --- Dirs ---

func (c *Context) StorageDir(nodeScoped bool) string              { return StorageDir(nodeScoped) }
func (c *Context) UploadDir() string                              { return UploadDir() }
func (c *Context) CacheDirPath(nodeScoped, userScoped bool) string { return CacheDirPath(nodeScoped, userScoped) }
func (c *Context) UserDir(nodeScoped bool) string                 { return UserDir(nodeScoped) }

// --- Storage I/O ---

func (c *Context) StorageRead(path string) string             { return StorageRead(path) }
func (c *Context) StorageWrite(path, data string) bool        { return StorageWrite(path, data) }
func (c *Context) StorageList(flowPathJSON string) string     { return StorageList(flowPathJSON) }

// --- Embeddings ---

func (c *Context) EmbedText(bitJSON, textsJSON string) string { return EmbedText(bitJSON, textsJSON) }

// --- HTTP ---

func (c *Context) HTTPRequest(method int, url, headers, body string) bool {
	return HTTPRequest(method, url, headers, body)
}

// --- Auth ---

func (c *Context) GetOAuthToken(provider string) string { return GetOAuthToken(provider) }
func (c *Context) HasOAuthToken(provider string) bool   { return HasOAuthToken(provider) }

// --- Time / Random ---

func (c *Context) TimeNow() int64 { return TimeNow() }
func (c *Context) Random() int64  { return Random() }

// --- Finalize ---

func (c *Context) Finish() ExecutionResult {
	for k, v := range c.outputs {
		c.result.Outputs[k] = v
	}
	return c.result
}

func (c *Context) Success() ExecutionResult {
	c.ActivateExec("exec_out")
	return c.Finish()
}

func (c *Context) Fail(err string) ExecutionResult {
	c.SetError(err)
	return c.Finish()
}

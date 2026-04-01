-- Flow-Like WASM SDK for Lua
-- Lua runs embedded in C (via glue.c) compiled to WASM with Emscripten.
-- The C glue populates flowlike_host with native bindings.

local M = {}

-- Host bridge table (populated by C glue before this module loads)
M._host = flowlike_host or {}

-- ============================================================================
-- Constants
-- ============================================================================

M.ABI_VERSION = 1

M.DataType = {
    Exec    = "Exec",
    String  = "String",
    I64     = "I64",
    F64     = "F64",
    Bool    = "Bool",
    Generic = "Generic",
    Bytes   = "Bytes",
    Date    = "Date",
    PathBuf = "PathBuf",
    Struct  = "Struct",
}

M.ValueType = {
    Normal  = "Normal",
    Array   = "Array",
    HashMap = "HashMap",
    HashSet = "HashSet",
}

M.LogLevel = {
    TRACE    = 0,
    DEBUG    = 1,
    INFO     = 2,
    WARN     = 3,
    ERROR    = 4,
    CRITICAL = 5,
}

-- ============================================================================
-- JSON helpers (hand-rolled, no external deps)
-- ============================================================================

local function json_escape(s)
    if s == nil then return "" end
    s = tostring(s)
    s = s:gsub("\\", "\\\\")
    s = s:gsub('"', '\\"')
    s = s:gsub("\n", "\\n")
    s = s:gsub("\r", "\\r")
    s = s:gsub("\t", "\\t")
    s = s:gsub("[\x00-\x1f]", function(c)
        return string.format("\\u%04x", string.byte(c))
    end)
    return s
end

local function json_quote(s)
    return '"' .. json_escape(s) .. '"'
end

M.jsonQuote = json_quote

function M.jsonString(s)
    return json_quote(s)
end

local function json_bool(b)
    return b and "true" or "false"
end

local function json_number(n)
    if n == nil then return "0" end
    if n == math.floor(n) and math.abs(n) < 2^53 then
        return string.format("%.0f", n)
    end
    return tostring(n)
end

local function json_array(arr, serializer)
    local parts = {}
    for i = 1, #arr do
        parts[i] = serializer and serializer(arr[i]) or tostring(arr[i])
    end
    return "[" .. table.concat(parts, ",") .. "]"
end

local function json_string_array(arr)
    return json_array(arr, json_quote)
end

-- ============================================================================
-- NodeScores
-- ============================================================================

function M.newNodeScores()
    return {
        privacy     = 0,
        security    = 0,
        performance = 0,
        governance  = 0,
        reliability = 0,
        cost        = 0,
    }
end

local function serialize_scores(s)
    return '{"privacy":' .. json_number(s.privacy)
        .. ',"security":' .. json_number(s.security)
        .. ',"performance":' .. json_number(s.performance)
        .. ',"governance":' .. json_number(s.governance)
        .. ',"reliability":' .. json_number(s.reliability)
        .. ',"cost":' .. json_number(s.cost) .. '}'
end

-- ============================================================================
-- PinDefinition
-- ============================================================================

function M.inputPin(name, friendlyName, description, dataType)
    return {
        name          = name,
        friendly_name = friendlyName,
        description   = description,
        pin_type      = "Input",
        data_type     = dataType or M.DataType.String,
        default_value = nil,
        value_type    = nil,
        schema        = nil,
    }
end

function M.outputPin(name, friendlyName, description, dataType)
    return {
        name          = name,
        friendly_name = friendlyName,
        description   = description,
        pin_type      = "Output",
        data_type     = dataType or M.DataType.String,
        default_value = nil,
        value_type    = nil,
        schema        = nil,
    }
end

function M.inputExec(name)
    return M.inputPin(name or "exec", name or "Exec", "Trigger", M.DataType.Exec)
end

function M.outputExec(name)
    return M.outputPin(name or "exec_out", name or "Exec Out", "Done", M.DataType.Exec)
end

function M.withDefault(pin, value)
    pin.default_value = value
    return pin
end

function M.withValueType(pin, vt)
    pin.value_type = vt
    return pin
end

function M.withSchema(pin, s)
    pin.schema = s
    return pin
end

function M.withStep(pin, step)
    pin.step = step
    return pin
end

function M.withSensitive(pin, sensitive)
    if sensitive == nil then sensitive = true end
    pin.sensitive = sensitive
    return pin
end

function M.withEnforceSchema(pin, enforce)
    if enforce == nil then enforce = true end
    pin.enforce_schema = enforce
    return pin
end

function M.withEnforceGenericValueType(pin, enforce)
    if enforce == nil then enforce = true end
    pin.enforce_generic_value_type = enforce
    return pin
end

local function serialize_pin(p)
    local j = '{"name":' .. json_quote(p.name)
        .. ',"friendly_name":' .. json_quote(p.friendly_name)
        .. ',"description":' .. json_quote(p.description)
        .. ',"pin_type":"' .. p.pin_type .. '"'
        .. ',"data_type":"' .. p.data_type .. '"'
    if p.default_value ~= nil then
        j = j .. ',"default_value":' .. p.default_value
    end
    if p.value_type ~= nil and p.value_type ~= "" then
        j = j .. ',"value_type":' .. json_quote(p.value_type)
    end
    if p.schema ~= nil and p.schema ~= "" then
        j = j .. ',"schema":' .. json_quote(p.schema)
    end
    if p.step ~= nil then
        j = j .. ',"step":' .. json_number(p.step)
    end
    if p.sensitive ~= nil then
        j = j .. ',"sensitive":' .. json_bool(p.sensitive)
    end
    if p.enforce_schema ~= nil then
        j = j .. ',"enforce_schema":' .. json_bool(p.enforce_schema)
    end
    if p.enforce_generic_value_type ~= nil then
        j = j .. ',"enforce_generic_value_type":' .. json_bool(p.enforce_generic_value_type)
    end
    j = j .. '}'
    return j
end

-- ============================================================================
-- NodeDefinition
-- ============================================================================

function M.newNodeDefinition()
    return {
        name          = "",
        friendly_name = "",
        description   = "",
        category      = "",
        icon          = nil,
        docs          = nil,
        long_running  = false,
        abi_version   = M.ABI_VERSION,
        pins          = {},
        scores        = nil,
        permissions   = {},
    }
end

function M.addPin(def, pin)
    def.pins[#def.pins + 1] = pin
    return def
end

function M.setScores(def, scores)
    def.scores = scores
    return def
end

function M.addPermission(def, perm)
    def.permissions[#def.permissions + 1] = perm
    return def
end

function M.serializeDefinition(def)
    local pins_parts = {}
    for i = 1, #def.pins do
        pins_parts[i] = serialize_pin(def.pins[i])
    end
    local pins_json = "[" .. table.concat(pins_parts, ",") .. "]"

    local j = '{"name":' .. json_quote(def.name)
        .. ',"friendly_name":' .. json_quote(def.friendly_name)
        .. ',"description":' .. json_quote(def.description)
        .. ',"category":' .. json_quote(def.category)
        .. ',"pins":' .. pins_json
        .. ',"long_running":' .. json_bool(def.long_running)
        .. ',"abi_version":' .. json_number(def.abi_version)

    if def.icon ~= nil and def.icon ~= "" then
        j = j .. ',"icon":' .. json_quote(def.icon)
    end
    if def.scores ~= nil then
        j = j .. ',"scores":' .. serialize_scores(def.scores)
    end
    if def.docs ~= nil and def.docs ~= "" then
        j = j .. ',"docs":' .. json_quote(def.docs)
    end
    if #def.permissions > 0 then
        j = j .. ',"permissions":' .. json_string_array(def.permissions)
    end

    j = j .. '}'
    return j
end

-- ============================================================================
-- ExecutionResult
-- ============================================================================

function M.successResult()
    return {
        outputs       = {},
        error         = nil,
        activate_exec = {},
        pending       = false,
    }
end

function M.failResult(msg)
    return {
        outputs       = {},
        error         = msg,
        activate_exec = {},
        pending       = false,
    }
end

function M.serializeResult(res)
    local out_parts = {}
    for k, v in pairs(res.outputs) do
        out_parts[#out_parts + 1] = json_quote(k) .. ":" .. v
    end
    local out_json = "{" .. table.concat(out_parts, ",") .. "}"

    local exec_json = json_string_array(res.activate_exec)

    local j = '{"outputs":' .. out_json
        .. ',"activate_exec":' .. exec_json
        .. ',"pending":' .. json_bool(res.pending)
    if res.error ~= nil and res.error ~= "" then
        j = j .. ',"error":' .. json_quote(res.error)
    end
    j = j .. '}'
    return j
end

-- ============================================================================
-- Execution input parsing
-- ============================================================================

local function is_ws(c)
    return c == " " or c == "\t" or c == "\n" or c == "\r"
end

local function extract_string(json, key)
    local needle = '"' .. key .. '"'
    local pos = json:find(needle, 1, true)
    if not pos then return "" end
    local i = pos + #needle
    while i <= #json and (is_ws(json:sub(i, i)) or json:sub(i, i) == ":") do i = i + 1 end
    if i > #json or json:sub(i, i) ~= '"' then return "" end
    i = i + 1
    local result = {}
    while i <= #json and json:sub(i, i) ~= '"' do
        if json:sub(i, i) == "\\" and i + 1 <= #json then
            i = i + 1
            local c = json:sub(i, i)
            if c == '"' then result[#result + 1] = '"'
            elseif c == "\\" then result[#result + 1] = "\\"
            elseif c == "n" then result[#result + 1] = "\n"
            elseif c == "r" then result[#result + 1] = "\r"
            elseif c == "t" then result[#result + 1] = "\t"
            else result[#result + 1] = c end
        else
            result[#result + 1] = json:sub(i, i)
        end
        i = i + 1
    end
    return table.concat(result)
end

local function extract_bool(json, key)
    local needle = '"' .. key .. '"'
    local pos = json:find(needle, 1, true)
    if not pos then return false end
    local i = pos + #needle
    while i <= #json and (is_ws(json:sub(i, i)) or json:sub(i, i) == ":") do i = i + 1 end
    return i + 3 <= #json and json:sub(i, i + 3) == "true"
end

local function extract_int(json, key)
    local needle = '"' .. key .. '"'
    local pos = json:find(needle, 1, true)
    if not pos then return 0 end
    local i = pos + #needle
    while i <= #json and (is_ws(json:sub(i, i)) or json:sub(i, i) == ":") do i = i + 1 end
    local neg = false
    if i <= #json and json:sub(i, i) == "-" then
        neg = true
        i = i + 1
    end
    local num = 0
    while i <= #json and json:sub(i, i) >= "0" and json:sub(i, i) <= "9" do
        num = num * 10 + tonumber(json:sub(i, i))
        i = i + 1
    end
    return neg and -num or num
end

local function parse_inputs_object(json)
    local result = {}
    local inputs_pos = json:find('"inputs"', 1, true)
    if not inputs_pos then return result end

    local obj_start = json:find("{", inputs_pos + 8)
    if not obj_start then return result end

    local depth = 1
    local obj_end = obj_start + 1
    while depth > 0 and obj_end <= #json do
        local c = json:sub(obj_end, obj_end)
        if c == "{" then depth = depth + 1
        elseif c == "}" then depth = depth - 1 end
        obj_end = obj_end + 1
    end

    local sub = json:sub(obj_start, obj_end - 1)
    local i = 2 -- skip opening {

    while i < #sub do
        -- skip whitespace
        while i <= #sub and is_ws(sub:sub(i, i)) do i = i + 1 end
        if i > #sub or sub:sub(i, i) == "}" then break end
        if sub:sub(i, i) == "," then i = i + 1 end
        while i <= #sub and is_ws(sub:sub(i, i)) do i = i + 1 end
        if sub:sub(i, i) ~= '"' then i = i + 1 end
        if i > #sub then break end

        if sub:sub(i, i) == '"' then
            i = i + 1
            local ks = i
            while i <= #sub and sub:sub(i, i) ~= '"' do i = i + 1 end
            local k = sub:sub(ks, i - 1)
            i = i + 1

            while i <= #sub and (is_ws(sub:sub(i, i)) or sub:sub(i, i) == ":") do i = i + 1 end

            local vs = i
            local c = sub:sub(i, i)
            if c == '"' then
                i = i + 1
                while i <= #sub do
                    if sub:sub(i, i) == '"' and sub:sub(i - 1, i - 1) ~= "\\" then break end
                    i = i + 1
                end
                i = i + 1
            elseif c == "{" then
                local d = 1
                i = i + 1
                while d > 0 and i <= #sub do
                    if sub:sub(i, i) == "{" then d = d + 1
                    elseif sub:sub(i, i) == "}" then d = d - 1 end
                    i = i + 1
                end
            elseif c == "[" then
                local d = 1
                i = i + 1
                while d > 0 and i <= #sub do
                    if sub:sub(i, i) == "[" then d = d + 1
                    elseif sub:sub(i, i) == "]" then d = d - 1 end
                    i = i + 1
                end
            else
                while i <= #sub and not is_ws(sub:sub(i, i)) and sub:sub(i, i) ~= "," and sub:sub(i, i) ~= "}" do
                    i = i + 1
                end
            end

            result[k] = sub:sub(vs, i - 1)
        else
            i = i + 1
        end
    end
    return result
end

function M.parseInput(raw)
    return {
        inputs       = parse_inputs_object(raw),
        node_id      = extract_string(raw, "node_id"),
        node_name    = extract_string(raw, "node_name"),
        run_id       = extract_string(raw, "run_id"),
        app_id       = extract_string(raw, "app_id"),
        board_id     = extract_string(raw, "board_id"),
        user_id      = extract_string(raw, "user_id"),
        stream_state = extract_bool(raw, "stream_state"),
        log_level    = extract_int(raw, "log_level"),
    }
end

-- ============================================================================
-- Host wrappers — Logging
-- ============================================================================

function M.logTrace(msg) M._host.log_trace(msg) end
function M.logDebug(msg) M._host.log_debug(msg) end
function M.logInfo(msg)  M._host.log_info(msg) end
function M.logWarn(msg)  M._host.log_warn(msg) end
function M.logError(msg) M._host.log_error(msg) end

function M.logJson(level, msg, data)
    M._host.log_json(level, msg, data)
end

-- ============================================================================
-- Host wrappers — Pins
-- ============================================================================

function M.getInput(name)    return M._host.get_input(name) end
function M.setOutput(name, v) M._host.set_output(name, v) end
function M.activateExec(name) M._host.activate_exec(name) end

-- ============================================================================
-- Host wrappers — Variables
-- ============================================================================

function M.varGet(name)       return M._host.var_get(name) end
function M.varSet(name, val)  M._host.var_set(name, val) end
function M.varDelete(name)    M._host.var_delete(name) end
function M.varHas(name)       return M._host.var_has(name) end

-- ============================================================================
-- Host wrappers — Cache
-- ============================================================================

function M.cacheGet(key)       return M._host.cache_get(key) end
function M.cacheSet(key, val)  M._host.cache_set(key, val) end
function M.cacheDelete(key)    M._host.cache_delete(key) end
function M.cacheHas(key)       return M._host.cache_has(key) end

-- ============================================================================
-- Host wrappers — Meta
-- ============================================================================

function M.metaNodeId()      return M._host.get_node_id() end
function M.metaRunId()       return M._host.get_run_id() end
function M.metaAppId()       return M._host.get_app_id() end
function M.metaBoardId()     return M._host.get_board_id() end
function M.metaUserId()      return M._host.get_user_id() end
function M.metaIsStreaming() return M._host.is_streaming() end
function M.metaLogLevel()    return M._host.get_log_level() end
function M.metaTimeNow()     return M._host.time_now() end
function M.metaRandom()      return M._host.random() end

-- ============================================================================
-- Host wrappers — Storage
-- ============================================================================

function M.storageRead(path)                return M._host.storage_read(path) end
function M.storageWrite(path, data)         return M._host.storage_write(path, data) end
function M.storageDir(nodeScoped)           return M._host.storage_dir(nodeScoped and 1 or 0) end
function M.uploadDir()                      return M._host.upload_dir() end
function M.cacheDir(nodeScoped, userScoped) return M._host.cache_dir(nodeScoped and 1 or 0, userScoped and 1 or 0) end
function M.userDir(nodeScoped)              return M._host.user_dir(nodeScoped and 1 or 0) end
function M.storageList(path)                return M._host.storage_list(path) end

-- ============================================================================
-- Host wrappers — Models
-- ============================================================================

function M.embedText(bit, texts) return M._host.embed_text(bit, texts) end

-- ============================================================================
-- Host wrappers — HTTP
-- ============================================================================

function M.httpRequest(method, url, headers, body)
    return M._host.http_request(method, url, headers, body)
end

-- ============================================================================
-- Host wrappers — Stream
-- ============================================================================

function M.streamEmit(eventType, data) M._host.stream_emit(eventType, data) end
function M.streamText(text)            M._host.stream_text(text) end

-- ============================================================================
-- Host wrappers — Auth
-- ============================================================================

function M.oauthGetToken(provider) return M._host.oauth_get_token(provider) end
function M.oauthHasToken(provider) return M._host.oauth_has_token(provider) end

-- ============================================================================
-- Context
-- ============================================================================

function M.newContext(input)
    local ctx = {
        input  = input,
        result = M.successResult(),
    }

    -- Input getters

    function ctx:getRaw(name)
        if self.input.inputs[name] then
            return self.input.inputs[name]
        end
        return ""
    end

    function ctx:getString(name, default)
        default = default or ""
        local v = self.input.inputs[name]
        if v == nil then return default end
        if #v >= 2 and v:sub(1, 1) == '"' and v:sub(-1) == '"' then
            return v:sub(2, -2)
        end
        return v
    end

    function ctx:getI64(name, default)
        default = default or 0
        local v = self.input.inputs[name]
        if v == nil then return default end
        return tonumber(v) or default
    end

    function ctx:getF64(name, default)
        default = default or 0.0
        local v = self.input.inputs[name]
        if v == nil then return default end
        return tonumber(v) or default
    end

    function ctx:getBool(name, default)
        default = default or false
        local v = self.input.inputs[name]
        if v == nil then return default end
        return v == "true"
    end

    -- Metadata shortcuts

    function ctx:nodeId()        return self.input.node_id end
    function ctx:nodeName()      return self.input.node_name end
    function ctx:runId()         return self.input.run_id end
    function ctx:appId()         return self.input.app_id end
    function ctx:boardId()       return self.input.board_id end
    function ctx:userId()        return self.input.user_id end
    function ctx:streamEnabled() return self.input.stream_state end
    function ctx:getLogLevel()   return self.input.log_level end

    -- Output setters

    function ctx:setOutput(name, jsonValue)
        self.result.outputs[name] = jsonValue
    end

    function ctx:activateExec(pin)
        self.result.activate_exec[#self.result.activate_exec + 1] = pin
    end

    function ctx:setPending(p)
        self.result.pending = p
    end

    function ctx:setError(msg)
        self.result.error = msg
    end

    -- Level-gated logging

    function ctx:debug(msg)
        if self.input.log_level <= 1 then M.logDebug(msg) end
    end

    function ctx:info(msg)
        if self.input.log_level <= 2 then M.logInfo(msg) end
    end

    function ctx:warn(msg)
        if self.input.log_level <= 3 then M.logWarn(msg) end
    end

    function ctx:error(msg)
        if self.input.log_level <= 4 then M.logError(msg) end
    end

    -- Conditional streaming

    function ctx:streamText(text)
        if self.input.stream_state then M.streamText(text) end
    end

    function ctx:streamJson(json)
        if self.input.stream_state then M.streamEmit("json", json) end
    end

    function ctx:streamProgress(pct, message)
        if self.input.stream_state then
            local data = '{"progress":' .. json_number(pct) .. ',"message":' .. json_quote(message) .. '}'
            M.streamEmit("progress", data)
        end
    end

    -- Variables

    function ctx:varGet(name)            return M.varGet(name) end
    function ctx:varSet(name, val)       M.varSet(name, val) end
    function ctx:varDelete(name)         M.varDelete(name) end
    function ctx:varHas(name)            return M.varHas(name) end

    -- Cache

    function ctx:cacheGet(key)           return M.cacheGet(key) end
    function ctx:cacheSet(key, val)      M.cacheSet(key, val) end
    function ctx:cacheDelete(key)        M.cacheDelete(key) end
    function ctx:cacheHas(key)           return M.cacheHas(key) end

    -- Dirs

    function ctx:storageDir(nodeScoped)  return M.storageDir(nodeScoped) end
    function ctx:uploadDir()             return M.uploadDir() end
    function ctx:cacheDir(nodeScoped, userScoped) return M.cacheDir(nodeScoped, userScoped) end
    function ctx:userDir(nodeScoped)     return M.userDir(nodeScoped) end

    -- Storage I/O

    function ctx:storageRead(path)       return M.storageRead(path) end
    function ctx:storageWrite(path, data) return M.storageWrite(path, data) end
    function ctx:storageList(path)       return M.storageList(path) end

    -- Embeddings

    function ctx:embedText(bit, texts)   return M.embedText(bit, texts) end

    -- HTTP

    function ctx:httpRequest(method, url, headers, body)
        return M.httpRequest(method, url, headers, body)
    end

    -- Auth

    function ctx:oauthGetToken(provider) return M.oauthGetToken(provider) end
    function ctx:oauthHasToken(provider) return M.oauthHasToken(provider) end

    -- Time / Random

    function ctx:timeNow()  return M.metaTimeNow() end
    function ctx:random()   return M.metaRandom() end

    -- Finalization

    function ctx:finish()
        return self.result
    end

    function ctx:success()
        self:activateExec("exec_out")
        return self:finish()
    end

    function ctx:fail(msg)
        self:setError(msg)
        return self:finish()
    end

    return ctx
end

return M

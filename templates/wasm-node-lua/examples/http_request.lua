-- HTTP Request Node — Demonstrates declaring HTTP permissions (Lua)
--
-- This example shows how to declare the "http" permission and use the
-- host import to make outbound HTTP requests from a Lua WASM node.
-- Copy this pattern into your node.lua when you need network access.

local sdk = require("sdk")

-- ============================================================================
-- Node definition — note addPermission("http")
-- ============================================================================

function get_node()
    local def = sdk.newNodeDefinition()
    def.name          = "http_get_request_lua"
    def.friendly_name = "HTTP GET Request (Lua)"
    def.description   = "Sends a GET request to a URL and reports the result"
    def.category      = "Network/HTTP"
    sdk.addPermission(def, "http")

    sdk.addPin(def, sdk.inputExec())
    sdk.addPin(def, sdk.withDefault(
        sdk.inputPin("url", "URL", "Target URL", sdk.DataType.String),
        '"https://httpbin.org/get"'
    ))
    sdk.addPin(def, sdk.withDefault(
        sdk.inputPin("headers_json", "Headers (JSON)", "Request headers as JSON", sdk.DataType.String),
        '"{}"'
    ))

    sdk.addPin(def, sdk.outputExec())
    sdk.addPin(def, sdk.outputPin("success", "Success", "Whether the HTTP call was accepted", sdk.DataType.Bool))

    return sdk.serializeDefinition(def)
end

function get_nodes()
    return "[" .. get_node() .. "]"
end

-- ============================================================================
-- Run handler — uses httpRequest host import directly
-- ============================================================================

function run_node(raw_json)
    local input = sdk.parseInput(raw_json)
    local ctx = sdk.newContext(input)

    local url = ctx:getString("url", "https://httpbin.org/get")
    local headers = ctx:getString("headers_json", "{}")

    sdk.logInfo("Sending GET request to " .. url)

    -- Method 0 = GET. The host checks the "http" capability.
    local resultCode = sdk.httpRequest(0, url, headers, "")
    local ok = resultCode ~= -1

    if ok then
        sdk.logInfo("HTTP capability granted — request dispatched")
    else
        sdk.logError("HTTP capability denied — is the 'http' permission declared?")
    end

    ctx:setOutput("success", ok and "true" or "false")

    local result = ctx:success()
    return sdk.serializeResult(result)
end

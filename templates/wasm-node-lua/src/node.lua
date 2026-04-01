-- Flow-Like WASM Node Template (Lua)
--
-- This file contains your custom node logic.
-- Build: mise run build
-- The compiled .wasm file will be at: build/node.wasm

local sdk = require("sdk")

-- ============================================================================
-- Node Definition
-- ============================================================================

function get_node()
    local def = sdk.newNodeDefinition()
    def.name          = "my_custom_node_lua"
    def.friendly_name = "My Custom Node (Lua)"
    def.description   = "A template WASM node built with Lua"
    def.category      = "Custom/WASM"
    sdk.addPermission(def, "streaming")

    -- Input pins
    sdk.addPin(def, sdk.inputExec())
    sdk.addPin(def, sdk.withDefault(
        sdk.inputPin("input_text", "Input Text", "Text to process", sdk.DataType.String),
        '""'
    ))
    sdk.addPin(def, sdk.withDefault(
        sdk.inputPin("multiplier", "Multiplier", "Number of times to repeat", sdk.DataType.I64),
        "1"
    ))

    -- Output pins
    sdk.addPin(def, sdk.outputExec())
    sdk.addPin(def, sdk.outputPin("output_text", "Output Text", "Processed text", sdk.DataType.String))
    sdk.addPin(def, sdk.outputPin("char_count", "Character Count", "Number of characters in output", sdk.DataType.I64))

    return sdk.serializeDefinition(def)
end

function get_nodes()
    return "[" .. get_node() .. "]"
end

-- ============================================================================
-- Node Execution
-- ============================================================================

function run_node(raw_json)
    local input = sdk.parseInput(raw_json)
    local ctx = sdk.newContext(input)

    local inputText = ctx:getString("input_text", "")
    local multiplier = ctx:getI64("multiplier", 1)
    if multiplier < 0 then multiplier = 0 end

    ctx:debug("Processing: '" .. inputText .. "' x " .. tostring(multiplier))

    local parts = {}
    for i = 1, multiplier do
        parts[i] = inputText
    end
    local outputText = table.concat(parts)
    local charCount = #outputText

    ctx:streamText("Generated " .. tostring(charCount) .. " characters")

    ctx:setOutput("output_text", sdk.jsonString(outputText))
    ctx:setOutput("char_count", tostring(charCount))

    local result = ctx:success()
    return sdk.serializeResult(result)
end

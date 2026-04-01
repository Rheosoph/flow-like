// Flow-Like WASM Node Template (Swift) — Component Model
//
// Build:
//   mise run setup      # install wasm-tools, wit-bindgen, WASI adapter
//   mise run generate   # generate C bindings from WIT
//   mise run build      # compile + create WASM component
//
// The compiled component will be at: node.wasm

import WitBindings

// MARK: - Constants

let ABI_VERSION: UInt32 = 1

// MARK: - WIT String Helpers

func makeWitString(_ s: String) -> flow_like_node_string_t {
    var ws = flow_like_node_string_t()
    let utf8 = Array(s.utf8)
    guard !utf8.isEmpty else {
        ws.ptr = nil
        ws.len = 0
        return ws
    }
    let ptr = UnsafeMutablePointer<UInt8>.allocate(capacity: utf8.count)
    for i in 0..<utf8.count { ptr[i] = utf8[i] }
    ws.ptr = ptr
    ws.len = utf8.count
    return ws
}

func readWitString(_ ws: flow_like_node_string_t) -> String {
    guard let ptr = ws.ptr, ws.len > 0 else { return "" }
    let buf = UnsafeBufferPointer(start: UnsafePointer(ptr), count: ws.len)
    return String(decoding: buf, as: UTF8.self)
}

func setWitResult(_ src: String, _ ret: UnsafeMutablePointer<flow_like_node_string_t>) {
    let utf8 = Array(src.utf8)
    guard !utf8.isEmpty else {
        ret.pointee.ptr = nil
        ret.pointee.len = 0
        return
    }
    guard let buf = cabi_realloc(nil, 0, 1, utf8.count) else {
        ret.pointee.ptr = nil
        ret.pointee.len = 0
        return
    }
    let dest = buf.assumingMemoryBound(to: UInt8.self)
    for i in 0..<utf8.count { dest[i] = utf8[i] }
    ret.pointee.ptr = dest
    ret.pointee.len = utf8.count
}

// MARK: - JSON Helpers

func jsonEscape(_ s: String) -> String {
    var result: [UInt8] = []
    result.reserveCapacity(s.utf8.count)
    for c in s.utf8 {
        switch c {
        case 0x22: result.append(0x5C); result.append(0x22)
        case 0x5C: result.append(0x5C); result.append(0x5C)
        case 0x0A: result.append(0x5C); result.append(0x6E)
        case 0x0D: result.append(0x5C); result.append(0x72)
        case 0x09: result.append(0x5C); result.append(0x74)
        default: result.append(c)
        }
    }
    return String(decoding: result, as: UTF8.self)
}

func jsonQuote(_ s: String) -> String {
    "\"" + jsonEscape(s) + "\""
}

// MARK: - Type Definitions

struct PinDefinition {
    let name: String
    let friendlyName: String
    let description: String
    let pinType: String
    let dataType: String
    var defaultValue: String = ""

    static func input(_ name: String, _ friendlyName: String, _ desc: String, _ dataType: String) -> PinDefinition {
        PinDefinition(name: name, friendlyName: friendlyName, description: desc, pinType: "Input", dataType: dataType)
    }

    static func output(_ name: String, _ friendlyName: String, _ desc: String, _ dataType: String) -> PinDefinition {
        PinDefinition(name: name, friendlyName: friendlyName, description: desc, pinType: "Output", dataType: dataType)
    }

    func withDefault(_ v: String) -> PinDefinition {
        var copy = self
        copy.defaultValue = v
        return copy
    }

    func toJSON() -> String {
        var j = "{\"name\":" + jsonQuote(name)
        j += ",\"friendly_name\":" + jsonQuote(friendlyName)
        j += ",\"description\":" + jsonQuote(description)
        j += ",\"pin_type\":\"" + pinType + "\""
        j += ",\"data_type\":\"" + dataType + "\""
        if !defaultValue.isEmpty { j += ",\"default_value\":" + defaultValue }
        j += "}"
        return j
    }
}

struct NodeDefinition {
    var name = ""
    var friendlyName = ""
    var description = ""
    var category = ""
    var longRunning = false
    var abiVersion: UInt32 = 1
    var pins: [PinDefinition] = []
    var permissions: [String] = []

    mutating func addPin(_ pin: PinDefinition) { pins.append(pin) }
    mutating func addPermission(_ p: String) { permissions.append(p) }

    func toJSON() -> String {
        var pinsJSON = "["
        for (i, pin) in pins.enumerated() {
            if i > 0 { pinsJSON += "," }
            pinsJSON += pin.toJSON()
        }
        pinsJSON += "]"

        var j = "{\"name\":" + jsonQuote(name)
        j += ",\"friendly_name\":" + jsonQuote(friendlyName)
        j += ",\"description\":" + jsonQuote(description)
        j += ",\"category\":" + jsonQuote(category)
        j += ",\"pins\":" + pinsJSON
        j += ",\"long_running\":" + (longRunning ? "true" : "false")
        j += ",\"abi_version\":\(abiVersion)"
        if !permissions.isEmpty {
            j += ",\"permissions\":["
            for (i, p) in permissions.enumerated() {
                if i > 0 { j += "," }
                j += jsonQuote(p)
            }
            j += "]"
        }
        j += "}"
        return j
    }
}

struct ExecutionResult {
    var outputs: [(String, String)] = []
    var error = ""
    var activateExec: [String] = []
    var pending = false

    func toJSON() -> String {
        var outJSON = "{"
        for (i, (k, v)) in outputs.enumerated() {
            if i > 0 { outJSON += "," }
            outJSON += jsonQuote(k) + ":" + v
        }
        outJSON += "}"

        var execJSON = "["
        for (i, pin) in activateExec.enumerated() {
            if i > 0 { execJSON += "," }
            execJSON += jsonQuote(pin)
        }
        execJSON += "]"

        var j = "{\"outputs\":" + outJSON
        j += ",\"activate_exec\":" + execJSON
        j += ",\"pending\":" + (pending ? "true" : "false")
        if !error.isEmpty { j += ",\"error\":" + jsonQuote(error) }
        j += "}"
        return j
    }
}

// MARK: - Context (wraps WIT import functions)

struct Context {
    var result = ExecutionResult()

    func getInputRaw(_ name: String) -> String {
        var wname = makeWitString(name)
        var ret = flow_like_node_string_t()
        if flow_like_node_pins_get_input(&wname, &ret) {
            return readWitString(ret)
        }
        return ""
    }

    func getString(_ name: String, _ defaultValue: String = "") -> String {
        let v = getInputRaw(name)
        if v.isEmpty { return defaultValue }
        let utf8 = Array(v.utf8)
        if utf8.count >= 2 && utf8[0] == 0x22 && utf8[utf8.count - 1] == 0x22 {
            return String(decoding: utf8[1..<(utf8.count - 1)], as: UTF8.self)
        }
        return v
    }

    func getI64(_ name: String, _ defaultValue: Int64 = 0) -> Int64 {
        let v = getInputRaw(name)
        if v.isEmpty { return defaultValue }
        return Int64(v) ?? defaultValue
    }

    func getF64(_ name: String, _ defaultValue: Double = 0.0) -> Double {
        let v = getInputRaw(name)
        if v.isEmpty { return defaultValue }
        return Double(v) ?? defaultValue
    }

    func getBool(_ name: String, _ defaultValue: Bool = false) -> Bool {
        let v = getInputRaw(name)
        if v.isEmpty { return defaultValue }
        return v == "true"
    }

    mutating func setOutput(_ name: String, _ jsonValue: String) {
        var wname = makeWitString(name)
        var wval = makeWitString(jsonValue)
        flow_like_node_pins_set_output(&wname, &wval)
        result.outputs.append((name, jsonValue))
    }

    mutating func activateExec(_ pin: String) {
        var wpin = makeWitString(pin)
        flow_like_node_pins_activate_exec(&wpin)
        result.activateExec.append(pin)
    }

    func debug(_ msg: String) {
        var wmsg = makeWitString(msg)
        flow_like_node_logging_log(0, &wmsg)
    }

    func info(_ msg: String) {
        var wmsg = makeWitString(msg)
        flow_like_node_logging_log(1, &wmsg)
    }

    func warn(_ msg: String) {
        var wmsg = makeWitString(msg)
        flow_like_node_logging_log(2, &wmsg)
    }

    func logError(_ msg: String) {
        var wmsg = makeWitString(msg)
        flow_like_node_logging_log(3, &wmsg)
    }

    func streamText(_ text: String) {
        var wtext = makeWitString(text)
        flow_like_node_streaming_text(&wtext)
    }

    func streamEvent(_ eventType: String, _ data: String) {
        var wev = makeWitString(eventType)
        var wdata = makeWitString(data)
        flow_like_node_streaming_emit(&wev, &wdata)
    }

    mutating func success() -> ExecutionResult {
        activateExec("exec_out")
        return result
    }

    mutating func fail(_ err: String) -> ExecutionResult {
        result.error = err
        return result
    }
}

// MARK: - Node Definition

func buildDefinition() -> NodeDefinition {
    var def = NodeDefinition()
    def.name = "my_custom_node_swift"
    def.friendlyName = "My Custom Node (Swift)"
    def.description = "A template WASM node built with Swift (Component Model)"
    def.category = "Custom/WASM"
    def.abiVersion = ABI_VERSION
    def.addPermission("streaming")

    def.addPin(.input("exec", "Execute", "Trigger execution", "Exec"))
    def.addPin(.input("input_text", "Input Text", "Text to process", "String").withDefault("\"\""))
    def.addPin(.input("multiplier", "Multiplier", "Number of times to repeat", "I64").withDefault("1"))

    def.addPin(.output("exec_out", "Done", "Execution complete", "Exec"))
    def.addPin(.output("output_text", "Output Text", "Processed text", "String"))
    def.addPin(.output("char_count", "Character Count", "Number of characters in output", "I64"))

    return def
}

// MARK: - Node Execution

func handleRun(_ ctx: inout Context) -> ExecutionResult {
    let inputText = ctx.getString("input_text")
    let multiplier = ctx.getI64("multiplier", 1)

    ctx.debug("Processing: '\(inputText)' x \(multiplier)")

    var output = ""
    for _ in 0..<multiplier {
        output += inputText
    }
    let charCount = output.count

    ctx.streamText("Generated \(charCount) characters")

    ctx.setOutput("output_text", jsonQuote(output))
    ctx.setOutput("char_count", "\(charCount)")

    return ctx.success()
}

// MARK: - WIT Component Exports (wit-bindgen-c naming convention)

@_cdecl("exports_flow_like_node_get_node")
func _exports_get_node(_ ret: UnsafeMutablePointer<flow_like_node_string_t>) {
    let json = buildDefinition().toJSON()
    setWitResult(json, ret)
}

@_cdecl("exports_flow_like_node_get_nodes")
func _exports_get_nodes(_ ret: UnsafeMutablePointer<flow_like_node_string_t>) {
    let json = "[" + buildDefinition().toJSON() + "]"
    setWitResult(json, ret)
}

@_cdecl("exports_flow_like_node_run")
func _exports_run(_ input: UnsafeMutablePointer<flow_like_node_string_t>, _ ret: UnsafeMutablePointer<flow_like_node_string_t>) {
    var ctx = Context()
    let result = handleRun(&ctx)
    setWitResult(result.toJSON(), ret)
}

@_cdecl("exports_flow_like_node_get_abi_version")
func _exports_get_abi_version() -> UInt32 {
    ABI_VERSION
}

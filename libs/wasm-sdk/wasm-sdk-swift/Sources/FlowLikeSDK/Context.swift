// Context.swift — High-level context wrapper for node execution.

public struct Context: ~Copyable {
    public let input: ExecutionInput
    var result: ExecutionResult
    var outputs: [String: String]

    public init(input: ExecutionInput) {
        self.input = input
        self.result = .success()
        self.outputs = [:]
    }

    // MARK: - Metadata

    public var nodeId: String { input.nodeId }
    public var nodeName: String { input.nodeName }
    public var runId: String { input.runId }
    public var appId: String { input.appId }
    public var boardId: String { input.boardId }
    public var userId: String { input.userId }
    public var streamEnabled: Bool { input.streamState }
    public var logLevelValue: UInt8 { input.logLevel }

    // MARK: - Input getters

    public func getInput(_ name: String) -> String? {
        input.inputs[name]
    }

    public func getString(_ name: String, _ defaultValue: String = "") -> String {
        guard let v = input.inputs[name] else { return defaultValue }
        let utf8 = Array(v.utf8)
        if utf8.count >= 2 && utf8[0] == 0x22 && utf8[utf8.count - 1] == 0x22 {
            return String(_bytes: Array(utf8[1..<(utf8.count - 1)]))
        }
        return v
    }

    public func getI64(_ name: String, _ defaultValue: Int64 = 0) -> Int64 {
        guard let v = input.inputs[name] else { return defaultValue }
        return parseInt64(v) ?? defaultValue
    }

    public func getF64(_ name: String, _ defaultValue: Double = 0.0) -> Double {
        guard let v = input.inputs[name] else { return defaultValue }
        return parseDouble(v) ?? defaultValue
    }

    public func getBool(_ name: String, _ defaultValue: Bool = false) -> Bool {
        guard let v = input.inputs[name] else { return defaultValue }
        return v == "true"
    }

    // MARK: - Output setters

    public mutating func setOutput(_ name: String, _ value: String) {
        outputs[name] = value
    }

    public mutating func activateExec(_ pinName: String) {
        result.activateExec.append(pinName)
    }

    public mutating func setPending(_ pending: Bool) {
        result.pending = pending
    }

    public mutating func setError(_ err: String) {
        result.error = err
    }

    // MARK: - Level-gated logging

    func shouldLog(_ level: UInt8) -> Bool {
        level >= input.logLevel
    }

    public func debug(_ msg: String) {
        if shouldLog(LogLevel.debug) { logDebug(msg) }
    }

    public func info(_ msg: String) {
        if shouldLog(LogLevel.info) { logInfo(msg) }
    }

    public func warn(_ msg: String) {
        if shouldLog(LogLevel.warn) { logWarn(msg) }
    }

    public func error(_ msg: String) {
        if shouldLog(LogLevel.error) { logError(msg) }
    }

    // MARK: - Conditional streaming

    public func streamText(_ text: String) {
        if streamEnabled { FlowLikeSDK.streamText(text) }
    }

    public func streamJSON(_ data: String) {
        if streamEnabled { streamEmit(eventType: "json", data: data) }
    }

    public func streamProgress(_ progress: Double, _ message: String) {
        if streamEnabled {
            var b = JSONBuilder()
            b.beginObject()
            b.addKey("progress"); b.addRaw(formatDouble(progress))
            b.addKey("message"); b.addString(message)
            b.endObject()
            streamEmit(eventType: "progress", data: b.build())
        }
    }

    // MARK: - Variables

    public func getVariable(_ name: String) -> String {
        FlowLikeSDK.getVariable(name)
    }

    public func setVariable(_ name: String, _ value: String) {
        FlowLikeSDK.setVariable(name, value)
    }

    public func deleteVariable(_ name: String) { FlowLikeSDK.deleteVariable(name) }
    public func hasVariable(_ name: String) -> Bool { FlowLikeSDK.hasVariable(name) }

    // MARK: - Cache

    public func cacheGet(_ key: String) -> String       { FlowLikeSDK.cacheGet(key) }
    public func cacheSet(_ key: String, _ value: String) { FlowLikeSDK.cacheSet(key, value) }
    public func cacheDelete(_ key: String)               { FlowLikeSDK.cacheDelete(key) }
    public func cacheHas(_ key: String) -> Bool          { FlowLikeSDK.cacheHas(key) }

    // MARK: - Dirs

    public func storageDir(nodeScoped: Bool = false) -> String             { FlowLikeSDK.storageDir(nodeScoped: nodeScoped) }
    public func uploadDir() -> String                                       { FlowLikeSDK.uploadDir() }
    public func cacheDirPath(nodeScoped: Bool = false, userScoped: Bool = false) -> String {
        FlowLikeSDK.cacheDirPath(nodeScoped: nodeScoped, userScoped: userScoped)
    }
    public func userDir(nodeScoped: Bool = false) -> String                { FlowLikeSDK.userDir(nodeScoped: nodeScoped) }

    // MARK: - Storage I/O

    public func storageRead(_ path: String) -> String             { FlowLikeSDK.storageRead(path) }
    public func storageWrite(_ path: String, _ data: String) -> Bool { FlowLikeSDK.storageWrite(path, data) }
    public func storageList(_ flowPathJSON: String) -> String     { FlowLikeSDK.storageList(flowPathJSON) }

    // MARK: - Embeddings

    public func embedText(bitJSON: String, textsJSON: String) -> String {
        FlowLikeSDK.embedText(bitJSON: bitJSON, textsJSON: textsJSON)
    }

    // MARK: - HTTP

    public func httpRequest(method: Int, url: String, headers: String, body: String) -> Bool {
        FlowLikeSDK.httpRequest(method: method, url: url, headers: headers, body: body)
    }

    // MARK: - Auth

    public func getOAuthToken(_ provider: String) -> String { FlowLikeSDK.getOAuthToken(provider) }
    public func hasOAuthToken(_ provider: String) -> Bool   { FlowLikeSDK.hasOAuthToken(provider) }

    // MARK: - Time / Random

    public func timeNow() -> Int64 { FlowLikeSDK.timeNow() }
    public func random() -> Int64  { FlowLikeSDK.random() }

    // MARK: - Finalize

    public mutating func finish() -> ExecutionResult {
        for (k, v) in outputs {
            result.outputs[k] = v
        }
        return result
    }

    public mutating func success() -> ExecutionResult {
        activateExec("exec_out")
        return finish()
    }

    public mutating func fail(_ err: String) -> ExecutionResult {
        setError(err)
        return finish()
    }
}

// MARK: - Internal number parsing (no Foundation)

func parseInt64(_ s: String) -> Int64? {
    let bytes = Array(s.utf8)
    if bytes.isEmpty { return nil }
    var idx = 0
    var negative = false
    if bytes[0] == 0x2D { negative = true; idx = 1 } // '-'
    if idx >= bytes.count { return nil }
    var value: Int64 = 0
    while idx < bytes.count {
        let d = bytes[idx]
        guard d >= 0x30 && d <= 0x39 else { return nil }
        value = value * 10 + Int64(d - 0x30)
        idx += 1
    }
    return negative ? -value : value
}

func parseDouble(_ s: String) -> Double? {
    let bytes = Array(s.utf8)
    if bytes.isEmpty { return nil }
    var idx = 0
    var negative = false
    if bytes[0] == 0x2D { negative = true; idx = 1 }
    if idx >= bytes.count { return nil }

    var intPart: Double = 0
    while idx < bytes.count && bytes[idx] >= 0x30 && bytes[idx] <= 0x39 {
        intPart = intPart * 10 + Double(bytes[idx] - 0x30)
        idx += 1
    }

    var fracPart: Double = 0
    if idx < bytes.count && bytes[idx] == 0x2E { // '.'
        idx += 1
        var divisor: Double = 10
        while idx < bytes.count && bytes[idx] >= 0x30 && bytes[idx] <= 0x39 {
            fracPart += Double(bytes[idx] - 0x30) / divisor
            divisor *= 10
            idx += 1
        }
    }

    var result = intPart + fracPart
    if negative { result = -result }

    // Handle scientific notation (e.g. 1.5e10)
    if idx < bytes.count && (bytes[idx] == 0x65 || bytes[idx] == 0x45) { // 'e' or 'E'
        idx += 1
        var expNeg = false
        if idx < bytes.count && bytes[idx] == 0x2D { expNeg = true; idx += 1 }
        else if idx < bytes.count && bytes[idx] == 0x2B { idx += 1 }
        var exp: Int = 0
        while idx < bytes.count && bytes[idx] >= 0x30 && bytes[idx] <= 0x39 {
            exp = exp * 10 + Int(bytes[idx] - 0x30)
            idx += 1
        }
        var multiplier: Double = 1
        for _ in 0..<exp { multiplier *= 10 }
        if expNeg { result /= multiplier } else { result *= multiplier }
    }

    return result
}

func formatDouble(_ value: Double) -> String {
    if value == 0 { return "0" }
    let isNeg = value < 0
    let v = isNeg ? -value : value
    let intPart = Int(v)
    let fracPart = v - Double(intPart)

    var result: [UInt8] = []
    if isNeg { result.append(0x2D) }

    // Integer part
    if intPart == 0 {
        result.append(0x30)
    } else {
        var digits: [UInt8] = []
        var n = intPart
        while n > 0 { digits.append(UInt8(n % 10) + 0x30); n /= 10 }
        for d in digits.reversed() { result.append(d) }
    }

    // Fractional part (up to 6 digits)
    if fracPart > 0.000001 {
        result.append(0x2E) // '.'
        var frac = fracPart
        for _ in 0..<6 {
            frac *= 10
            let digit = Int(frac)
            result.append(UInt8(digit) + 0x30)
            frac -= Double(digit)
            if frac < 0.000001 { break }
        }
    }

    return String(_bytes: result)
}

// JSON.swift — Hand-rolled JSON utilities (no Foundation).

// MARK: - JSON Escape

public func jsonEscape(_ s: String) -> String {
    var result: [UInt8] = []
    result.reserveCapacity(s.utf8.count)
    for c in s.utf8 {
        switch c {
        case 0x22: // "
            result.append(0x5C); result.append(0x22)
        case 0x5C: // backslash
            result.append(0x5C); result.append(0x5C)
        case 0x0A: // \n
            result.append(0x5C); result.append(0x6E)
        case 0x0D: // \r
            result.append(0x5C); result.append(0x72)
        case 0x09: // \t
            result.append(0x5C); result.append(0x74)
        default:
            result.append(c)
        }
    }
    return String(_bytes: result)
}

public func jsonQuote(_ s: String) -> String {
    "\"" + jsonEscape(s) + "\""
}

// MARK: - String from UTF-8 bytes (no Foundation)

extension String {
    init(_bytes: [UInt8]) {
        self = _bytes.withUnsafeBufferPointer { buf in
            String(decoding: buf, as: UTF8.self)
        }
    }
}

// MARK: - JSONBuilder

public struct JSONBuilder: ~Copyable, Sendable {
    var buffer: [UInt8] = []
    var needsComma: Bool = false

    public init() {}

    public mutating func beginObject() {
        if needsComma { buffer.append(0x2C) } // ,
        buffer.append(0x7B) // {
        needsComma = false
    }

    public mutating func endObject() {
        buffer.append(0x7D) // }
        needsComma = true
    }

    public mutating func beginArray() {
        if needsComma { buffer.append(0x2C) } // ,
        buffer.append(0x5B) // [
        needsComma = false
    }

    public mutating func endArray() {
        buffer.append(0x5D) // ]
        needsComma = true
    }

    public mutating func addComma() {
        buffer.append(0x2C) // ,
    }

    public mutating func addKey(_ key: String) {
        if needsComma { buffer.append(0x2C) }
        appendQuotedString(key)
        buffer.append(0x3A) // :
        needsComma = false
    }

    public mutating func addKeyRaw(_ key: String) {
        appendQuotedString(key)
        buffer.append(0x3A) // :
        needsComma = false
    }

    public mutating func addString(_ value: String) {
        appendQuotedString(value)
        needsComma = true
    }

    public mutating func addInt(_ value: Int) {
        appendIntLiteral(value)
        needsComma = true
    }

    public mutating func addBool(_ value: Bool) {
        if value {
            for c in "true".utf8 { buffer.append(c) }
        } else {
            for c in "false".utf8 { buffer.append(c) }
        }
        needsComma = true
    }

    public mutating func addRaw(_ raw: String) {
        for c in raw.utf8 { buffer.append(c) }
        needsComma = true
    }

    public consuming func build() -> String {
        String(_bytes: buffer)
    }

    // MARK: - Internal helpers

    mutating func appendQuotedString(_ s: String) {
        buffer.append(0x22) // "
        for c in s.utf8 {
            switch c {
            case 0x22:
                buffer.append(0x5C); buffer.append(0x22)
            case 0x5C:
                buffer.append(0x5C); buffer.append(0x5C)
            case 0x0A:
                buffer.append(0x5C); buffer.append(0x6E)
            case 0x0D:
                buffer.append(0x5C); buffer.append(0x72)
            case 0x09:
                buffer.append(0x5C); buffer.append(0x74)
            default:
                buffer.append(c)
            }
        }
        buffer.append(0x22) // "
    }

    mutating func appendIntLiteral(_ value: Int) {
        if value == 0 {
            buffer.append(0x30) // '0'
            return
        }
        var v = value
        var negative = false
        if v < 0 {
            negative = true
            v = -v
        }
        var digits: [UInt8] = []
        while v > 0 {
            digits.append(UInt8(v % 10) + 0x30)
            v /= 10
        }
        if negative { buffer.append(0x2D) } // '-'
        for d in digits.reversed() { buffer.append(d) }
    }
}

// MARK: - Minimal JSON parser for ExecutionInput

public func parseExecutionInput(_ s: String) -> ExecutionInput {
    var input = ExecutionInput()
    let bytes = Array(s.utf8)
    var idx = 0

    func skipWhitespace() {
        while idx < bytes.count {
            let c = bytes[idx]
            if c == 0x20 || c == 0x09 || c == 0x0A || c == 0x0D {
                idx += 1
            } else {
                break
            }
        }
    }

    func readString() -> String {
        guard idx < bytes.count, bytes[idx] == 0x22 else { return "" }
        idx += 1 // skip opening "
        var result: [UInt8] = []
        while idx < bytes.count && bytes[idx] != 0x22 {
            if bytes[idx] == 0x5C { // backslash
                idx += 1
                if idx < bytes.count {
                    switch bytes[idx] {
                    case 0x6E: result.append(0x0A)
                    case 0x72: result.append(0x0D)
                    case 0x74: result.append(0x09)
                    case 0x22: result.append(0x22)
                    case 0x5C: result.append(0x5C)
                    default: result.append(bytes[idx])
                    }
                    idx += 1
                }
            } else {
                result.append(bytes[idx])
                idx += 1
            }
        }
        if idx < bytes.count { idx += 1 } // skip closing "
        return String(_bytes: result)
    }

    func readValue() -> String {
        skipWhitespace()
        guard idx < bytes.count else { return "" }
        let c = bytes[idx]
        if c == 0x22 { // "
            let v = readString()
            return "\"" + jsonEscape(v) + "\""
        } else if c == 0x7B { // {
            var depth = 0
            let start = idx
            while idx < bytes.count {
                if bytes[idx] == 0x7B { depth += 1 }
                else if bytes[idx] == 0x7D {
                    depth -= 1
                    if depth == 0 { idx += 1; return String(_bytes: Array(bytes[start..<idx])) }
                } else if bytes[idx] == 0x22 {
                    idx += 1
                    while idx < bytes.count && bytes[idx] != 0x22 {
                        if bytes[idx] == 0x5C { idx += 1 }
                        idx += 1
                    }
                }
                idx += 1
            }
            return String(_bytes: Array(bytes[start..<idx]))
        } else if c == 0x5B { // [
            var depth = 0
            let start = idx
            while idx < bytes.count {
                if bytes[idx] == 0x5B { depth += 1 }
                else if bytes[idx] == 0x5D {
                    depth -= 1
                    if depth == 0 { idx += 1; return String(_bytes: Array(bytes[start..<idx])) }
                } else if bytes[idx] == 0x22 {
                    idx += 1
                    while idx < bytes.count && bytes[idx] != 0x22 {
                        if bytes[idx] == 0x5C { idx += 1 }
                        idx += 1
                    }
                }
                idx += 1
            }
            return String(_bytes: Array(bytes[start..<idx]))
        } else {
            let start = idx
            while idx < bytes.count {
                let b = bytes[idx]
                if b == 0x2C || b == 0x7D || b == 0x5D ||
                   b == 0x20 || b == 0x09 || b == 0x0A || b == 0x0D {
                    break
                }
                idx += 1
            }
            return String(_bytes: Array(bytes[start..<idx]))
        }
    }

    skipWhitespace()
    guard idx < bytes.count, bytes[idx] == 0x7B else { return input }
    idx += 1 // skip {

    while idx < bytes.count {
        skipWhitespace()
        if idx >= bytes.count || bytes[idx] == 0x7D { break }
        if bytes[idx] == 0x2C { idx += 1; continue }
        let key = readString()
        skipWhitespace()
        if idx < bytes.count && bytes[idx] == 0x3A { idx += 1 }
        skipWhitespace()

        switch key {
        case "node_id": input.nodeId = readString()
        case "node_name": input.nodeName = readString()
        case "run_id": input.runId = readString()
        case "app_id": input.appId = readString()
        case "board_id": input.boardId = readString()
        case "user_id": input.userId = readString()
        case "stream_state":
            let v = readValue()
            input.streamState = (v == "true")
        case "log_level":
            let v = readValue()
            if let digit = v.utf8.first, digit >= 0x30 && digit <= 0x39 {
                input.logLevel = digit - 0x30
            }
        case "inputs":
            skipWhitespace()
            if idx < bytes.count && bytes[idx] == 0x7B {
                idx += 1
                while idx < bytes.count {
                    skipWhitespace()
                    if idx >= bytes.count || bytes[idx] == 0x7D { idx += 1; break }
                    if bytes[idx] == 0x2C { idx += 1; continue }
                    let iKey = readString()
                    skipWhitespace()
                    if idx < bytes.count && bytes[idx] == 0x3A { idx += 1 }
                    let iVal = readValue()
                    input.inputs[iKey] = iVal
                }
            } else {
                _ = readValue()
            }
        default:
            _ = readValue()
        }
    }

    return input
}

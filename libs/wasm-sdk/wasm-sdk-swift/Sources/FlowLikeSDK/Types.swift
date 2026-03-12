// Types.swift — Core type definitions for the Flow-Like WASM SDK.

public let ABI_VERSION: UInt32 = 1

// MARK: - DataType

public enum DataType: Sendable {
    case exec
    case string
    case i64
    case f64
    case bool
    case generic
    case bytes
    case date
    case pathBuf
    case `struct`

    public var rawValue: Swift.String {
        switch self {
        case .exec: return "Exec"
        case .string: return "String"
        case .i64: return "I64"
        case .f64: return "F64"
        case .bool: return "Bool"
        case .generic: return "Generic"
        case .bytes: return "Bytes"
        case .date: return "Date"
        case .pathBuf: return "PathBuf"
        case .struct: return "Struct"
        }
    }
}

// MARK: - ValueType

public enum ValueTypeKind: Sendable {
    case normal
    case array
    case hashMap
    case hashSet

    public var rawValue: Swift.String {
        switch self {
        case .normal: return "Normal"
        case .array: return "Array"
        case .hashMap: return "HashMap"
        case .hashSet: return "HashSet"
        }
    }
}

// MARK: - PinType

public enum PinType: Sendable {
    case input
    case output

    public var rawValue: Swift.String {
        switch self {
        case .input: return "Input"
        case .output: return "Output"
        }
    }
}

// MARK: - LogLevel

public struct LogLevel {
    public static let debug: UInt8 = 0
    public static let info: UInt8 = 1
    public static let warn: UInt8 = 2
    public static let error: UInt8 = 3
    public static let critical: UInt8 = 4
}

// MARK: - NodeScores

public struct NodeScores: Sendable {
    public var privacy: Int
    public var security: Int
    public var performance: Int
    public var governance: Int
    public var reliability: Int
    public var cost: Int

    public init(
        privacy: Int = 50,
        security: Int = 50,
        performance: Int = 50,
        governance: Int = 50,
        reliability: Int = 50,
        cost: Int = 50
    ) {
        self.privacy = privacy
        self.security = security
        self.performance = performance
        self.governance = governance
        self.reliability = reliability
        self.cost = cost
    }

    public func toJSON() -> Swift.String {
        var b = JSONBuilder()
        b.beginObject()
        b.addKey("privacy"); b.addInt(privacy)
        b.addKey("security"); b.addInt(security)
        b.addKey("performance"); b.addInt(performance)
        b.addKey("governance"); b.addInt(governance)
        b.addKey("reliability"); b.addInt(reliability)
        b.addKey("cost"); b.addInt(cost)
        b.endObject()
        return b.build()
    }
}

// MARK: - PinDefinition

public struct PinDefinition: Sendable {
    public var name: Swift.String
    public var friendlyName: Swift.String
    public var description: Swift.String
    public var pinType: PinType
    public var dataType: DataType
    public var defaultValue: Swift.String?
    public var valueType: Swift.String?
    public var schema: Swift.String?
    public var step: Double?
    public var sensitive: Bool?
    public var enforceSchema: Bool?
    public var enforceGenericValueType: Bool?

    public init(
        name: Swift.String,
        friendlyName: Swift.String,
        description: Swift.String,
        pinType: PinType,
        dataType: DataType
    ) {
        self.name = name
        self.friendlyName = friendlyName
        self.description = description
        self.pinType = pinType
        self.dataType = dataType
    }

    public func withDefault(_ value: Swift.String) -> PinDefinition {
        var copy = self
        copy.defaultValue = value
        return copy
    }

    public func withValueType(_ vt: Swift.String) -> PinDefinition {
        var copy = self
        copy.valueType = vt
        return copy
    }

    public func withValueType(_ vt: ValueTypeKind) -> PinDefinition {
        var copy = self
        copy.valueType = vt.rawValue
        return copy
    }

    public func withSchema(_ s: Swift.String) -> PinDefinition {
        var copy = self
        copy.schema = s
        return copy
    }

    public func withStep(_ step: Double) -> PinDefinition {
        var copy = self
        copy.step = step
        return copy
    }

    public func withSensitive(_ sensitive: Bool = true) -> PinDefinition {
        var copy = self
        copy.sensitive = sensitive
        return copy
    }

    public func withEnforceSchema(_ enforce: Bool = true) -> PinDefinition {
        var copy = self
        copy.enforceSchema = enforce
        return copy
    }

    public func withEnforceGenericValueType(_ enforce: Bool = true) -> PinDefinition {
        var copy = self
        copy.enforceGenericValueType = enforce
        return copy
    }

    public func toJSON() -> Swift.String {
        var b = JSONBuilder()
        b.beginObject()
        b.addKey("name"); b.addString(name)
        b.addKey("friendly_name"); b.addString(friendlyName)
        b.addKey("description"); b.addString(description)
        b.addKey("pin_type"); b.addString(pinType.rawValue)
        b.addKey("data_type"); b.addString(dataType.rawValue)
        if let dv = defaultValue {
            b.addKey("default_value"); b.addRaw(dv)
        }
        if let vt = valueType {
            b.addKey("value_type"); b.addString(vt)
        }
        if let sc = schema {
            b.addKey("schema"); b.addString(sc)
        }
        if let st = step {
            b.addKey("step"); b.addRaw(Swift.String(st))
        }
        if let sn = sensitive {
            b.addKey("sensitive"); b.addBool(sn)
        }
        if let es = enforceSchema {
            b.addKey("enforce_schema"); b.addBool(es)
        }
        if let eg = enforceGenericValueType {
            b.addKey("enforce_generic_value_type"); b.addBool(eg)
        }
        b.endObject()
        return b.build()
    }
}

// MARK: - Helper constructors

public func inputPin(
    _ name: Swift.String,
    _ friendlyName: Swift.String,
    _ description: Swift.String,
    _ dataType: DataType
) -> PinDefinition {
    PinDefinition(
        name: name,
        friendlyName: friendlyName,
        description: description,
        pinType: .input,
        dataType: dataType
    )
}

public func outputPin(
    _ name: Swift.String,
    _ friendlyName: Swift.String,
    _ description: Swift.String,
    _ dataType: DataType
) -> PinDefinition {
    PinDefinition(
        name: name,
        friendlyName: friendlyName,
        description: description,
        pinType: .output,
        dataType: dataType
    )
}

public func inputExec(_ name: Swift.String = "exec") -> PinDefinition {
    inputPin(name, name, "", .exec)
}

public func outputExec(_ name: Swift.String = "exec_out") -> PinDefinition {
    outputPin(name, name, "", .exec)
}

// MARK: - NodeDefinition

public struct NodeDefinition: Sendable {
    public var name: Swift.String
    public var friendlyName: Swift.String
    public var description: Swift.String
    public var category: Swift.String
    public var icon: Swift.String?
    public var pins: [PinDefinition]
    public var scores: NodeScores?
    public var longRunning: Bool
    public var docs: Swift.String?
    public var abiVersion: UInt32

    public init() {
        name = ""
        friendlyName = ""
        description = ""
        category = ""
        pins = []
        longRunning = false
        abiVersion = ABI_VERSION
    }

    public mutating func addPin(_ pin: PinDefinition) {
        pins.append(pin)
    }

    public mutating func setScores(_ s: NodeScores) {
        scores = s
    }

    public func toJSON() -> Swift.String {
        var b = JSONBuilder()
        b.beginObject()
        b.addKey("name"); b.addString(name)
        b.addKey("friendly_name"); b.addString(friendlyName)
        b.addKey("description"); b.addString(description)
        b.addKey("category"); b.addString(category)
        b.addKey("pins")
        b.beginArray()
        for (i, pin) in pins.enumerated() {
            if i > 0 { b.addComma() }
            b.addRaw(pin.toJSON())
        }
        b.endArray()
        b.addKey("long_running"); b.addBool(longRunning)
        b.addKey("abi_version"); b.addInt(Int(abiVersion))
        if let ic = icon {
            b.addKey("icon"); b.addString(ic)
        }
        if let sc = scores {
            b.addKey("scores"); b.addRaw(sc.toJSON())
        }
        if let dc = docs {
            b.addKey("docs"); b.addString(dc)
        }
        b.endObject()
        return b.build()
    }
}

// MARK: - ExecutionInput

public struct ExecutionInput: Sendable {
    public var inputs: [Swift.String: Swift.String]
    public var nodeId: Swift.String
    public var nodeName: Swift.String
    public var runId: Swift.String
    public var appId: Swift.String
    public var boardId: Swift.String
    public var userId: Swift.String
    public var streamState: Bool
    public var logLevel: UInt8

    public init() {
        inputs = [:]
        nodeId = ""
        nodeName = ""
        runId = ""
        appId = ""
        boardId = ""
        userId = ""
        streamState = false
        logLevel = LogLevel.info
    }
}

// MARK: - ExecutionResult

public struct ExecutionResult: Sendable {
    public var outputs: [Swift.String: Swift.String]
    public var error: Swift.String?
    public var activateExec: [Swift.String]
    public var pending: Bool

    public init() {
        outputs = [:]
        activateExec = []
        pending = false
    }

    public static func success() -> ExecutionResult {
        ExecutionResult()
    }

    public static func fail(_ message: Swift.String) -> ExecutionResult {
        var r = ExecutionResult()
        r.error = message
        return r
    }

    public mutating func setOutput(_ name: Swift.String, _ value: Swift.String) {
        outputs[name] = value
    }

    public mutating func activateExecPin(_ pinName: Swift.String) {
        activateExec.append(pinName)
    }

    public mutating func setPending(_ p: Bool) {
        pending = p
    }

    public func toJSON() -> Swift.String {
        var b = JSONBuilder()
        b.beginObject()
        b.addKey("outputs")
        b.beginObject()
        var first = true
        for (k, v) in outputs {
            if !first { b.addComma() }
            first = false
            b.addKeyRaw(k); b.addRaw(v)
        }
        b.endObject()
        b.addKey("activate_exec")
        b.beginArray()
        for (i, e) in activateExec.enumerated() {
            if i > 0 { b.addComma() }
            b.addString(e)
        }
        b.endArray()
        b.addKey("pending"); b.addBool(pending)
        if let err = error {
            b.addKey("error"); b.addString(err)
        }
        b.endObject()
        return b.build()
    }
}

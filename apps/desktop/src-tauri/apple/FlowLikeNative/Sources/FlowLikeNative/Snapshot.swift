import Foundation

public struct NativeQueryParameter: Codable, Sendable, Equatable {
    public var name: String
    public var value: String

    public init(name: String, value: String) {
        self.name = name
        self.value = value
    }
}

public enum NativeAppRoute {
    private static func containsControl(_ value: String) -> Bool {
        value.unicodeScalars.contains { $0.properties.generalCategory == .control }
    }

    private static func decodePercentEscapes(_ value: String, formValue: Bool = false) -> String {
        let source = Array(value.utf8)
        var bytes: [UInt8] = []
        var index = 0
        func hex(_ byte: UInt8) -> UInt8? {
            switch byte {
            case 48...57: return byte - 48
            case 65...70: return byte - 55
            case 97...102: return byte - 87
            default: return nil
            }
        }
        while index < source.count {
            if source[index] == 37, index + 2 < source.count,
               let high = hex(source[index + 1]), let low = hex(source[index + 2]) {
                bytes.append(high * 16 + low)
                index += 3
            } else {
                bytes.append(formValue && source[index] == 43 ? 32 : source[index])
                index += 1
            }
        }
        return String(decoding: bytes, as: UTF8.self)
    }

    static func queryParameters(encodedQuery: String) -> [NativeQueryParameter] {
        encodedQuery.split(separator: "&").map {
            let pair = $0.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            return NativeQueryParameter(name: decodePercentEscapes(String(pair[0]), formValue: true),
                                        value: decodePercentEscapes(pair.count > 1 ? String(pair[1]) : "", formValue: true))
        }
    }

    public static func queryParameters(names: [String]?, values: [String]?) throws -> [NativeQueryParameter] {
        let names = names ?? []
        let values = values ?? []
        guard names.count == values.count else { throw NativeIntegrationError.invalidRoute }
        let parameters = zip(names, values).map { NativeQueryParameter(name: $0.0, value: $0.1) }
        try validate(path: nil, queryParams: parameters)
        return parameters
    }

    public static func validate(path: String?, queryParams: [NativeQueryParameter]?) throws {
        let path = path ?? ""
        let parameters = queryParams ?? []
        guard path.utf8.count <= 4096, parameters.count <= 32,
              !containsControl(path),
              !path.contains("#") else { throw NativeIntegrationError.invalidRoute }
        let parts = path.split(separator: "?", maxSplits: 1, omittingEmptySubsequences: false)
        let route = String(parts[0])
            .trimmingCharacters(in: .whitespaces)
        for candidate in [route, decodePercentEscapes(route)] {
            guard !candidate.hasPrefix("//"), !candidate.contains("\\"),
                  !containsControl(candidate),
                  candidate.range(of: "^[A-Za-z][A-Za-z0-9+.-]*:", options: .regularExpression) == nil,
                  !candidate.split(separator: "/", omittingEmptySubsequences: false).contains(where: { $0 == "." || $0 == ".." }) else {
                throw NativeIntegrationError.invalidRoute
            }
        }
        let rawParameters = parts.count > 1 ? queryParameters(encodedQuery: String(parts[1])) : []
        guard rawParameters.count + parameters.count <= 32 else { throw NativeIntegrationError.invalidRoute }
        for parameter in rawParameters + parameters {
            guard !parameter.name.isEmpty, parameter.name.utf8.count <= 256,
                  parameter.value.utf8.count <= 4096,
                  !containsControl(parameter.name), !containsControl(parameter.value) else {
                throw NativeIntegrationError.invalidRoute
            }
        }
        let size = parameters.reduce(path.utf8.count) { $0 + $1.name.utf8.count + $1.value.utf8.count }
        guard size <= 8192 else { throw NativeIntegrationError.invalidRoute }
    }
}

public struct NativeAction: Codable, Sendable, Equatable {
    public var kind: String
    public var appId: String?
    public var eventId: String?
    public var runId: String?
    public var prompt: String?
    public var voice: Bool?
    public var operation: String?
    public var text: String?
    public var files: [String]?
    public var path: String?
    public var queryParams: [NativeQueryParameter]?

    public init(kind: String, appId: String? = nil, eventId: String? = nil,
                runId: String? = nil, prompt: String? = nil, voice: Bool? = nil,
                operation: String? = nil, text: String? = nil, files: [String]? = nil,
                path: String? = nil, queryParams: [NativeQueryParameter]? = nil) {
        self.kind = kind
        self.appId = appId
        self.eventId = eventId
        self.runId = runId
        self.prompt = prompt
        self.voice = voice
        self.operation = operation
        self.text = text
        self.files = files
        self.path = path
        self.queryParams = queryParams
    }
}

public struct NativeActionRequest: Codable, Sendable {
    public var id: String
    public var scope: String
    public var action: NativeAction
    public var createdAt: String?
    public var responseMode: String?
    public var responseDeadline: String?

    public var isCurrent: Bool {
        guard let createdAt, let date = NativeSnapshot.date(createdAt) else { return false }
        guard date <= Date().addingTimeInterval(60) && date > Date().addingTimeInterval(-300) else { return false }
        if let responseMode {
            guard ["text", "result"].contains(responseMode), let responseDeadline,
                  let deadline = NativeSnapshot.date(responseDeadline), deadline > Date() else { return false }
        }
        return true
    }
}

public struct NativeItem: Codable, Sendable, Identifiable {
    public var id: String
    public var title: String
    public var subtitle: String?
    public var value: String?
    public var action: NativeAction
    public var status: String?
    public var progress: Double?
    public var startedAt: String?
    public var icon: NativeNotificationIcon? = nil
}

public struct NativeSection: Codable, Sendable {
    public var kind: String
    public var title: String
    public var state: String
    public var items: [NativeItem]
}

public struct NativeEvent: Codable, Sendable, Identifiable {
    public var id: String
    public var appId: String
    public var eventId: String
    public var title: String
    public var subtitle: String?
    public var eventType: String
    public var route: String?
    public var pageId: String?
    public var action: NativeAction
    public var surfaces: [String]
}

public struct NativeApp: Codable, Sendable, Identifiable {
    public var id: String
    public var title: String
    public var spotlightEligible: Bool?
}

public struct NativePage: Codable, Sendable {
    public var title: String
    public var url: String
}

public struct NativeSnapshot: Codable, Sendable {
    public var version: Int
    public var scope: String
    public var generatedAt: String
    public var expiresAt: String
    public var sections: [NativeSection]
    public var events: [NativeEvent]
    public var apps: [NativeApp]
    public var activePage: NativePage?
    public var webOrigin: String?
    public var customWidgets: [NativeCustomWidget]? = nil

    public var isCurrent: Bool {
        guard version == 1, !scope.isEmpty, let expiry = Self.date(expiresAt) else { return false }
        return expiry > Date()
    }

    public static func date(_ value: String) -> Date? {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.date(from: value) ?? ISO8601DateFormatter().date(from: value)
    }
}

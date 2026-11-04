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
    public var botId: String?
    public var conversationId: String?
    public var taskId: String?
    public var turnId: String?
    public var approvalId: String?
    public var decision: String?

    public init(kind: String, appId: String? = nil, eventId: String? = nil,
                runId: String? = nil, prompt: String? = nil, voice: Bool? = nil,
                operation: String? = nil, text: String? = nil, files: [String]? = nil,
                path: String? = nil, queryParams: [NativeQueryParameter]? = nil,
                botId: String? = nil, conversationId: String? = nil, taskId: String? = nil,
                turnId: String? = nil, approvalId: String? = nil, decision: String? = nil) {
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
        self.botId = botId
        self.conversationId = conversationId
        self.taskId = taskId
        self.turnId = turnId
        self.approvalId = approvalId
        self.decision = decision
    }
}

public struct NativeActionRequest: Codable, Sendable {
    public var id: String
    public var scope: String
    public var action: NativeAction
    public var createdAt: String?
    public var responseMode: String?
    public var responseDeadline: String?
    public var executionMode: String? = nil
    public var backgroundDelivery: Bool? = nil
    public var backgroundError: String? = nil

    public var isBackground: Bool { executionMode == "background" }

    public var allowsBackgroundDelivery: Bool {
        backgroundDelivery == true && responseMode == nil && !isBackground &&
        ["ask_bot", "respond_bot_approval"].contains(action.kind) && action.voice != true &&
        action.files?.isEmpty != false
    }

    public var isCurrent: Bool {
        guard executionMode == nil || executionMode == "foreground" || isBackground else { return false }
        guard let createdAt, let date = NativeSnapshot.date(createdAt) else { return false }
        let lifetime: TimeInterval = allowsBackgroundDelivery ? 86_400 : 300
        guard date <= Date().addingTimeInterval(60) && date > Date().addingTimeInterval(-lifetime) else { return false }
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
    public var updatedAt: String? = nil
    public var icon: NativeNotificationIcon? = nil
    /// A run the app is executing right now, not an entry read from run history.
    public var live: Bool? = nil
}

public struct NativeSection: Codable, Sendable {
    public var kind: String
    public var title: String
    public var state: String
    public var items: [NativeItem]
    // Set only on sections carried over from an earlier snapshot after a failed read.
    public var updatedAt: String?
    public var staleAt: String?
    public var expiresAt: String?
}

extension NativeSection {
    /// Keeps the items the app just sent and fills in last good items after them. The app failed to
    /// confirm the carried items, so the section counts as stale from now on.
    func carrying(_ history: [NativeItem], from last: NativeSection, freshness: NativeFreshness, now: Date) -> NativeSection {
        var carried = last
        let fresh = Set(items.map(\.id))
        carried.items = Array((items + history.filter { !fresh.contains($0.id) }).prefix(12))
        carried.updatedAt = NativeSnapshot.string(freshness.updatedAt)
        carried.staleAt = NativeSnapshot.string(min(freshness.staleAt, now))
        carried.expiresAt = NativeSnapshot.string(freshness.expiresAt)
        return carried
    }
}

public struct NativeFreshness: Sendable, Equatable {
    public let updatedAt: Date
    public let staleAt: Date
    public let expiresAt: Date

    public func isStale(at date: Date) -> Bool { staleAt <= date }
    public func isExpired(at date: Date) -> Bool { expiresAt <= date }
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

public struct NativeBot: Codable, Sendable, Identifiable {
    public var id: String
    public var title: String
    public var subtitle: String?
    public var local: Bool?
    public var encrypted: Bool?
    public var enabled: Bool?
    public var surfaces: [String]?
    public var action: NativeAction?

    public func exposes(_ surface: String) -> Bool {
        enabled != false && surfaces?.contains(surface) == true
    }
}

public struct NativeBotConversation: Codable, Sendable, Identifiable {
    public var id: String
    public var botId: String?
    public var title: String
    public var action: NativeAction?
}

public struct NativeBotTask: Codable, Sendable, Identifiable {
    public var id: String
    public var botId: String
    public var title: String
    public var enabled: Bool?
    public var status: String?
    public var action: NativeAction?
}

public struct NativeBotApproval: Codable, Sendable, Identifiable {
    public var id: String
    public var botId: String
    public var conversationId: String?
    public var turnId: String
    public var title: String
    public var status: String
    public var expiresAt: String?
    public var action: NativeAction?

    public func isPending(at date: Date = Date()) -> Bool {
        guard ["pending", "waiting"].contains(status) else { return false }
        guard let expiresAt else { return true }
        return NativeSnapshot.date(expiresAt).map { $0 > date } ?? false
    }
}

public struct NativeSnapshot: Codable, Sendable {
    public var version: Int
    public var scope: String
    public var generatedAt: String
    /// Content turns stale here; stale content still renders with its age.
    public var staleAt: String?
    /// Privacy window: nothing from this snapshot renders after it.
    public var expiresAt: String
    public var sections: [NativeSection]
    public var events: [NativeEvent]
    public var apps: [NativeApp]
    public var activePage: NativePage?
    public var webOrigin: String?
    public var customWidgets: [NativeCustomWidget]? = nil
    public var userId: String? = nil
    public var bots: [NativeBot]? = nil
    public var conversations: [NativeBotConversation]? = nil
    public var botTasks: [NativeBotTask]? = nil
    public var botApprovals: [NativeBotApproval]? = nil
    // Failed reads the app reports so publishing keeps the previous values.
    public var appsUnavailable: Bool? = nil
    public var eventsUnavailable: [String]? = nil

    public var isCurrent: Bool { isCurrent(at: Date()) }

    public func isCurrent(at date: Date) -> Bool {
        guard version == 1, !scope.isEmpty, let expiry = Self.date(expiresAt) else { return false }
        return expiry > date
    }

    // Snapshots written before staleAt existed expired after one hour, so they turn stale at expiry.
    public func freshness(of section: NativeSection) -> NativeFreshness? {
        guard let generated = Self.date(generatedAt), let expiry = Self.date(expiresAt) else { return nil }
        let sectionExpiry = min(section.expiresAt.flatMap(Self.date) ?? expiry, expiry)
        let stale = section.staleAt.flatMap(Self.date) ?? staleAt.flatMap(Self.date) ?? expiry
        return NativeFreshness(updatedAt: section.updatedAt.flatMap(Self.date) ?? generated,
                               staleAt: min(stale, sectionExpiry), expiresAt: sectionExpiry)
    }

    /// Replaces data whose read failed with the last good same-account data that is still inside its privacy window.
    public func retainingLastGood(from previous: NativeSnapshot?, now: Date = Date()) -> NativeSnapshot {
        var result = self
        result.appsUnavailable = nil
        result.eventsUnavailable = nil
        guard let previous, previous.scope == scope, previous.isCurrent(at: now) else { return result }
        let failedEventApps = appsUnavailable == true ? [] : Set(eventsUnavailable ?? [])
        if appsUnavailable == true {
            result.apps = previous.apps
            result.events = previous.events
        } else if !failedEventApps.isEmpty {
            result.restoreEvents(of: failedEventApps, from: previous)
        }
        let appIds = Set(result.apps.map(\.id))
        let exposed = Set(result.events.filter { $0.surfaces.contains("widget") }.map(\.id))
        for index in result.sections.indices {
            let incoming = result.sections[index]
            guard let last = previous.sections.first(where: { $0.kind == incoming.kind && $0.state == "ready" }),
                  let freshness = previous.freshness(of: last), !freshness.isExpired(at: now) else { continue }
            let history = last.items.filter { $0.live != true && ($0.action.appId.map(appIds.contains) ?? true) }
            if incoming.state == "unavailable" {
                result.sections[index] = incoming.carrying(history, from: last, freshness: freshness, now: now)
            } else if incoming.kind == "event_favorites", incoming.state == "ready", !failedEventApps.isEmpty {
                let kept = history.filter { $0.action.appId.map(failedEventApps.contains) == true && exposed.contains($0.id) }
                if !kept.isEmpty { result.sections[index] = incoming.carrying(kept, from: last, freshness: freshness, now: now) }
            }
        }
        if let widgets = result.customWidgets {
            result.customWidgets = widgets.map { $0.retainingContent(from: previous.customWidgets, now: now) }
        }
        return result
    }

    private mutating func restoreEvents(of failed: Set<String>, from previous: NativeSnapshot) {
        let present = Set(events.map(\.id))
        events += previous.events.filter { failed.contains($0.appId) && !present.contains($0.id) }
        let eligible = Dictionary(previous.apps.map { ($0.id, $0.spotlightEligible) }, uniquingKeysWith: { first, _ in first })
        for index in apps.indices where failed.contains(apps[index].id) {
            if let value = eligible[apps[index].id] { apps[index].spotlightEligible = value }
        }
    }

    public static func date(_ value: String) -> Date? {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.date(from: value) ?? ISO8601DateFormatter().date(from: value)
    }

    static func string(_ date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.string(from: date)
    }
}

extension NativeSnapshot {
    func validateBotAction(_ action: NativeAction, now: Date = Date()) throws {
        let kinds = ["open_bot", "ask_bot", "open_conversation", "run_bot_task", "respond_bot_approval",
                     "capture_bot_voice", "capture_bot_document", "local_bot_summary"]
        guard kinds.contains(action.kind) || (action.kind == "share" && action.botId != nil) else { return }
        func validId(_ id: String?) -> Bool {
            guard let id else { return false }
            return !id.isEmpty && id.utf8.count <= 512 && !id.unicodeScalars.contains { $0.properties.generalCategory == .control }
        }
        guard validId(action.botId), let bot = bots?.first(where: { $0.id == action.botId }), bot.enabled != false else {
            throw NativeIntegrationError.invalidAction
        }
        if let conversationId = action.conversationId {
            guard validId(conversationId) else { throw NativeIntegrationError.invalidAction }
            // The host authorizes newly received conversations; cached conversations must agree on the bot.
            if let conversation = conversations?.first(where: { $0.id == conversationId }), let owner = conversation.botId, owner != bot.id {
                throw NativeIntegrationError.invalidAction
            }
        }
        if action.kind == "open_conversation", !validId(action.conversationId) { throw NativeIntegrationError.invalidAction }
        if action.kind == "ask_bot" {
            guard let prompt = action.prompt, !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  prompt.utf8.count <= 65_536 else { throw NativeIntegrationError.invalidAction }
        }
        if action.kind == "share", !bot.exposes("share") { throw NativeIntegrationError.invalidAction }
        if action.kind == "run_bot_task" {
            guard validId(action.taskId), botTasks?.contains(where: {
                $0.id == action.taskId && $0.botId == bot.id && $0.enabled != false
            }) == true else { throw NativeIntegrationError.invalidAction }
        }
        if action.kind == "respond_bot_approval" {
            guard validId(action.turnId), validId(action.approvalId), ["once", "deny"].contains(action.decision ?? "") else {
                throw NativeIntegrationError.invalidAction
            }
            // The authenticated host resolves the request again before recording a decision.
            if let approval = botApprovals?.first(where: { $0.id == action.approvalId }) {
                guard approval.botId == bot.id, approval.turnId == action.turnId, approval.isPending(at: now) else {
                    throw NativeIntegrationError.invalidAction
                }
            }
        } else if action.decision != nil {
            throw NativeIntegrationError.invalidAction
        }
    }
}

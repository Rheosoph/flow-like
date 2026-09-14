#if !os(iOS) || FLOW_LIKE_INTENTS_HOST
import AppIntents
import CoreTransferable
import Foundation
#if os(iOS)
import FlowLikeNative
#endif

public struct FlowLikeNativePackage: AppIntentsPackage {}

public struct FlowAppEntity: AppEntity {
    public static let typeDisplayRepresentation: TypeDisplayRepresentation = "App"
    public static let defaultQuery = FlowAppQuery()
    public var id: String
    public var sourceId: String
    public var scope: String
    @Property(title: "Name") public var title: String

    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(title)", image: NativeStore.shared.appIconData(scope: scope, appId: sourceId)
            .map { .init(data: $0, isTemplate: false) } ?? .init(systemName: "app.fill"))
    }

    public init(_ app: NativeApp, scope: String) {
        id = "\(scope.utf8.count):\(scope)\(app.id)"
        sourceId = app.id
        self.scope = scope
        title = app.title
    }
}

public struct FlowAppQuery: EntityStringQuery {
    public init() {}
    private var entities: [FlowAppEntity] {
        guard let snapshot = NativeStore.shared.catalog() else { return [] }
        return snapshot.apps.filter { $0.spotlightEligible == true }.map {
            FlowAppEntity($0, scope: snapshot.scope)
        }
    }
    public func entities(for identifiers: [String]) async throws -> [FlowAppEntity] {
        entities.filter { identifiers.contains($0.id) }
    }
    public func entities(matching string: String) async throws -> [FlowAppEntity] {
        entities.filter { $0.title.localizedStandardContains(string) }
    }
    public func suggestedEntities() async throws -> [FlowAppEntity] { entities }
}

public struct OpenFlowAppIntent: AppIntent {
    public static let title: LocalizedStringResource = "Open App"
    public static let description = IntentDescription("Open a Flow Like app at an internal path. Add query names and matching values without URL encoding them.")
    public static let openAppWhenRun = true
    public static let authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "App") public var app: FlowAppEntity
    @Parameter(title: "Path") public var path: String?
    @Parameter(title: "Query names") public var queryNames: [String]?
    @Parameter(title: "Query values") public var queryValues: [String]?
    public static var parameterSummary: some ParameterSummary {
        Summary("Open \(\.$app) at \(\.$path)") {
            \.$queryNames
            \.$queryValues
        }
    }
    public init() {}
    public init(app: FlowAppEntity, path: String? = nil, queryNames: [String]? = nil,
                queryValues: [String]? = nil) {
        self.app = app
        self.path = path
        self.queryNames = queryNames
        self.queryValues = queryValues
    }
    public func perform() async throws -> some IntentResult {
        guard let snapshot = NativeStore.shared.catalog(), snapshot.scope == app.scope,
              snapshot.apps.contains(where: { $0.id == app.sourceId && $0.spotlightEligible == true }) else {
            throw NativeIntegrationError.invalidAction
        }
        let query = try NativeAppRoute.queryParameters(names: queryNames, values: queryValues)
        let action = NativeAction(kind: "open_app", appId: app.sourceId, path: path, queryParams: query)
        _ = try NativeStore.shared.enqueue(action, expectedScope: app.scope)
        return .result()
    }
}

public struct FlowEventEntity: AppEntity {
    public static let typeDisplayRepresentation: TypeDisplayRepresentation = "Event"
    public static let defaultQuery = FlowEventQuery()
    public var id: String
    public var sourceId: String
    public var scope: String
    @Property(title: "Name") public var title: String
    @Property(title: "App") public var appName: String

    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(title)", subtitle: "\(appName)", image: NativeStore.shared.appIconData(scope: scope, eventId: sourceId)
            .map { .init(data: $0, isTemplate: false) } ?? .init(systemName: "bolt.fill"))
    }

    public init(_ event: NativeEvent, scope: String) {
        id = "\(scope.utf8.count):\(scope)\(event.id)"
        sourceId = event.id
        self.scope = scope
        title = event.title
        appName = event.subtitle ?? "Flow Like"
    }
}

public struct FlowEventQuery: EntityStringQuery {
    public init() {}
    private var entities: [FlowEventEntity] {
        guard let snapshot = NativeStore.shared.catalog() else { return [] }
        return snapshot.events.filter {
            $0.surfaces.contains("siri") || $0.surfaces.contains("shortcuts")
        }.map { FlowEventEntity($0, scope: snapshot.scope) }
    }
    public func entities(for identifiers: [String]) async throws -> [FlowEventEntity] {
        entities.filter { identifiers.contains($0.id) }
    }
    public func entities(matching string: String) async throws -> [FlowEventEntity] {
        entities.filter { $0.title.localizedStandardContains(string) || $0.appName.localizedStandardContains(string) }
    }
    public func suggestedEntities() async throws -> [FlowEventEntity] { entities }

}

public struct RunFlowEventIntent: AppIntent {
    public static let title: LocalizedStringResource = "Run Event"
    public static let description = IntentDescription("Run an exposed Event in Flow Like and return its Text and JSON output. The app opens while it runs.")
    public static let openAppWhenRun = true
    public static let authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "Event") public var event: FlowEventEntity
    @Parameter(title: "Input") public var input: String?
    public static var parameterSummary: some ParameterSummary { Summary("Run \(\.$event)") { \.$input } }
    public init() {}

    public func perform() async throws -> some IntentResult & ReturnsValue<FlowEventResultEntity> & ProvidesDialog {
        let request = try enqueue(in: .shared)
        let result = try await NativeStore.shared.waitForActionResult(request)
        let value = FlowEventResultEntity(result)
        return .result(value: value, dialog: "\(value.dialogText)")
    }

    func enqueue(in store: NativeStore) throws -> NativeActionRequest {
        guard let snapshot = store.catalog(), snapshot.scope == event.scope,
              let item = snapshot.events.first(where: {
            $0.id == event.sourceId && ($0.surfaces.contains("siri") || $0.surfaces.contains("shortcuts"))
        }) else { throw NativeIntegrationError.invalidAction }
        var action = item.action
        action.text = input
        return try store.enqueue(action, expectedScope: event.scope, responseMode: "result")
    }
}

public struct AskFlowPilotIntent: AppIntent {
    public static let title: LocalizedStringResource = "Ask FlowPilot"
    public static let description = IntentDescription("Ask FlowPilot and return its reply as text. The app opens while it prepares the answer.")
    public static let openAppWhenRun = true
    public static let authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "Question", requestValueDialog: "What would you like to ask FlowPilot?") public var question: String?
    @Parameter(title: "Start voice", default: false) public var voice: Bool
    public static var parameterSummary: some ParameterSummary {
        Summary("Ask FlowPilot \(\.$question)") { \.$voice }
    }
    public init() {}

    public func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
        let request = try enqueue(in: .shared)
        let result = try await NativeStore.shared.waitForActionResult(request)
        let text = result.text ?? ""
        return .result(value: text, dialog: "\(String(text.prefix(4000)))")
    }

    func enqueue(in store: NativeStore) throws -> NativeActionRequest {
        let prompt = question?.trimmingCharacters(in: .whitespacesAndNewlines)
        guard prompt?.isEmpty == false else {
            throw $question.needsValueError("What would you like to ask FlowPilot?")
        }
        return try store.enqueue(NativeAction(kind: "flowpilot", prompt: prompt, voice: voice), responseMode: "text")
    }
}

public struct FlowChatEventEntity: AppEntity {
    public static let typeDisplayRepresentation: TypeDisplayRepresentation = "App chat"
    public static let defaultQuery = FlowChatEventQuery()
    public var id: String
    public var sourceId: String
    public var scope: String
    @Property(title: "Name") public var title: String
    @Property(title: "App") public var appName: String
    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(title)", subtitle: "\(appName)", image: NativeStore.shared.appIconData(scope: scope, eventId: sourceId)
            .map { .init(data: $0, isTemplate: false) } ?? .init(systemName: "bubble.left.and.bubble.right.fill"))
    }
    public init(_ event: NativeEvent, scope: String) {
        let entity = FlowEventEntity(event, scope: scope)
        id = entity.id
        sourceId = entity.sourceId
        self.scope = scope
        title = event.title
        appName = event.subtitle ?? "Flow Like"
    }
}

public struct FlowChatEventQuery: EntityStringQuery {
    public init() {}
    private var entities: [FlowChatEventEntity] {
        guard let snapshot = NativeStore.shared.catalog() else { return [] }
        return snapshot.events.filter {
            $0.eventType == "simple_chat" && ($0.surfaces.contains("siri") || $0.surfaces.contains("shortcuts"))
        }.map { FlowChatEventEntity($0, scope: snapshot.scope) }
    }
    public func entities(for identifiers: [String]) async throws -> [FlowChatEventEntity] { entities.filter { identifiers.contains($0.id) } }
    public func entities(matching string: String) async throws -> [FlowChatEventEntity] {
        entities.filter { $0.title.localizedStandardContains(string) || $0.appName.localizedStandardContains(string) }
    }
    public func suggestedEntities() async throws -> [FlowChatEventEntity] { entities }
}

public struct AskAppChatIntent: AppIntent {
    public static let title: LocalizedStringResource = "Ask App Chat"
    public static let description = IntentDescription("Ask an app's exposed Chat Event and return its reply as text. The app opens while the chat runs.")
    public static let openAppWhenRun = true
    public static let authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "Chat") public var chat: FlowChatEventEntity
    @Parameter(title: "Question", requestValueDialog: "What would you like to ask this app?") public var question: String
    public static var parameterSummary: some ParameterSummary { Summary("Ask \(\.$chat) \(\.$question)") }
    public init() {}
    public func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
        let request = try enqueue(in: .shared)
        let result = try await NativeStore.shared.waitForActionResult(request)
        let text = result.text ?? ""
        return .result(value: text, dialog: "\(String(text.prefix(4000)))")
    }
    func enqueue(in store: NativeStore) throws -> NativeActionRequest {
        let prompt = question.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !prompt.isEmpty else { throw $question.needsValueError("What would you like to ask this app?") }
        guard let snapshot = store.catalog(), snapshot.scope == chat.scope,
              let event = snapshot.events.first(where: {
                  $0.id == chat.sourceId && $0.eventType == "simple_chat" &&
                  ($0.surfaces.contains("siri") || $0.surfaces.contains("shortcuts"))
              }) else { throw NativeIntegrationError.invalidAction }
        var action = event.action
        action.text = prompt
        return try store.enqueue(action, expectedScope: chat.scope, responseMode: "text")
    }
}

public struct FlowEventResultEntity: TransientAppEntity, Transferable {
    public static let typeDisplayRepresentation: TypeDisplayRepresentation = "Event result"
    @Property(title: "Text") public var text: String
    @Property(title: "JSON") public var json: String
    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "Event result", subtitle: "\(String(text.prefix(200)))")
    }
    public var transferableText: String { text.isEmpty ? json : text }
    public var dialogText: String {
        if !text.isEmpty { return String(text.prefix(4000)) }
        return json.isEmpty ? "The Event finished without returning an output." : "The Event returned JSON output."
    }
    public static var transferRepresentation: some TransferRepresentation { ProxyRepresentation(exporting: \.transferableText) }
    public init() {
        text = ""
        json = ""
    }
    public init(_ result: NativeActionResult) {
        text = result.text ?? ""
        json = result.json ?? ""
    }
}

public struct OpenFlowPilotIntent: AppIntent {
    public static let title: LocalizedStringResource = "Open FlowPilot"
    public static let isDiscoverable = false
    public static let openAppWhenRun = true
    public static let authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "Voice input", default: false) public var voice: Bool
    public init() {}
    public init(voice: Bool) { self.voice = voice }
    public func perform() async throws -> some IntentResult {
        _ = try NativeStore.shared.enqueue(NativeAction(kind: "flowpilot", voice: voice))
        return .result()
    }
}

public struct OpenFlowInboxIntent: AppIntent {
    public static let title: LocalizedStringResource = "Open Notifications"
    public static let openAppWhenRun = true
    public static let authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    public init() {}
    public func perform() async throws -> some IntentResult {
        _ = try NativeStore.shared.enqueue(NativeAction(kind: "open_inbox"))
        return .result()
    }
}

public struct OpenFlowHomeIntent: AppIntent {
    public static let title: LocalizedStringResource = "Open Workspace"
    public static let openAppWhenRun = true
    public init() {}
    public func perform() async throws -> some IntentResult {
        _ = try NativeStore.shared.enqueue(NativeAction(kind: "open_home"))
        return .result()
    }
}

// Widget buttons keep the original item ID and scope, never an arbitrary executable payload.
public struct OpenNativeItemIntent: AppIntent {
    public static let title: LocalizedStringResource = "Open in Flow Like"
    public static let isDiscoverable = false
    public static let openAppWhenRun = true
    public static let authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "Item") public var itemId: String
    @Parameter(title: "Section") public var section: String
    @Parameter(title: "Workspace") public var scope: String
    public init() {}
    public init(itemId: String, section: String, scope: String) {
        self.itemId = itemId
        self.section = section
        self.scope = scope
    }
    public func perform() async throws -> some IntentResult {
        guard let snapshot = NativeStore.shared.snapshot(), snapshot.scope == scope,
              let item = snapshot.sections.first(where: { $0.kind == section && $0.state == "ready" })?
                .items.first(where: { $0.id == itemId }) else { throw NativeIntegrationError.expired }
        if let eventId = item.action.eventId {
            guard snapshot.events.contains(where: {
                $0.eventId == eventId && $0.appId == item.action.appId && $0.surfaces.contains("widget")
            }) else { throw NativeIntegrationError.invalidAction }
        }
        _ = try NativeStore.shared.enqueue(item.action, expectedScope: scope)
        return .result()
    }
}

#if !os(iOS) || FLOW_LIKE_SHORTCUTS_HOST
public struct FlowLikeShortcuts: AppShortcutsProvider {
    public static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: OpenFlowAppIntent(), phrases: [
            "Open \(\.$app) in \(.applicationName)",
            "Open an app in \(.applicationName)"
        ], shortTitle: "Open App", systemImageName: "app.fill")
        AppShortcut(intent: RunFlowEventIntent(), phrases: [
            "Run \(\.$event) in \(.applicationName)",
            "Run an Event in \(.applicationName)"
        ], shortTitle: "Run Event", systemImageName: "bolt.fill")
        AppShortcut(intent: AskFlowPilotIntent(), phrases: [
            "Ask FlowPilot in \(.applicationName)"
        ], shortTitle: "Ask FlowPilot", systemImageName: "sparkles")
        AppShortcut(intent: AskAppChatIntent(), phrases: [
            "Ask \(\.$chat) in \(.applicationName)",
            "Ask an app in \(.applicationName)"
        ], shortTitle: "Ask App Chat", systemImageName: "bubble.left.and.bubble.right")
        AppShortcut(intent: OpenFlowInboxIntent(), phrases: [
            "Open notifications in \(.applicationName)"
        ], shortTitle: "Notifications", systemImageName: "bell")
        AppShortcut(intent: OpenFlowHomeIntent(), phrases: [
            "Open my workspace in \(.applicationName)"
        ], shortTitle: "Workspace", systemImageName: "square.grid.2x2")
    }
}
#endif

public struct WidgetFlowEventEntity: AppEntity {
    public static let typeDisplayRepresentation: TypeDisplayRepresentation = "Event"
    public static let defaultQuery = WidgetFlowEventQuery()
    public var id: String
    public var sourceId: String
    public var scope: String
    public var title: String
    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(title)", image: NativeStore.shared.appIconData(scope: scope, eventId: sourceId)
            .map { .init(data: $0, isTemplate: false) } ?? .init(systemName: "bolt.fill"))
    }
    public init(_ event: NativeEvent, scope: String) {
        id = "\(scope.utf8.count):\(scope)\(event.id)"
        sourceId = event.id
        self.scope = scope
        title = event.title
    }
}

public struct WidgetFlowEventQuery: EntityStringQuery {
    public init() {}
    private var entities: [WidgetFlowEventEntity] {
        guard let snapshot = NativeStore.shared.snapshot() else { return [] }
        return snapshot.events.filter { $0.surfaces.contains("widget") }.map {
            WidgetFlowEventEntity($0, scope: snapshot.scope)
        }
    }
    public func entities(for identifiers: [String]) async throws -> [WidgetFlowEventEntity] {
        entities.filter { identifiers.contains($0.id) }
    }
    public func entities(matching string: String) async throws -> [WidgetFlowEventEntity] {
        entities.filter { $0.title.localizedStandardContains(string) }
    }
    public func suggestedEntities() async throws -> [WidgetFlowEventEntity] { entities }

}

@available(iOS 18.0, macOS 26.0, *)
public struct FlowEventControlConfiguration: ControlConfigurationIntent {
    public static let title: LocalizedStringResource = "Event"
    @Parameter(title: "Event") public var event: WidgetFlowEventEntity?
    public init() {}
}

public struct RunWidgetFlowEventIntent: AppIntent {
    public static let title: LocalizedStringResource = "Open Event"
    public static let isDiscoverable = false
    public static let openAppWhenRun = true
    public static let authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "Event") public var event: WidgetFlowEventEntity?
    public init() {}
    public init(event: WidgetFlowEventEntity?) { self.event = event }
    public func perform() async throws -> some IntentResult {
        guard let event, let snapshot = NativeStore.shared.snapshot(), snapshot.scope == event.scope,
              let item = snapshot.events.first(where: {
            $0.id == event.sourceId && $0.surfaces.contains("widget")
        }) else { throw NativeIntegrationError.invalidAction }
        _ = try NativeStore.shared.enqueue(item.action, expectedScope: event.scope)
        return .result()
    }
}
#endif

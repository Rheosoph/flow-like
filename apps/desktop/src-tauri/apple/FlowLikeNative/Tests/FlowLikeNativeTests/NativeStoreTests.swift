import Foundation
import Testing
@testable import FlowLikeNative

@Test func rejectsExpiredSnapshot() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    #expect(throws: (any Error).self) { try store.publish(snapshot(scope: "alice", expires: .distantPast)) }
    #expect(store.snapshot() == nil)
}

@Test func accountChangeRemovesPendingActions() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    _ = try store.publish(snapshot(scope: "alice"))
    _ = try store.enqueue(NativeAction(kind: "flowpilot", prompt: "Private question"))
    _ = try store.publish(snapshot(scope: "bob"))
    #expect(try store.takeActions().isEmpty)
}

@Test func actionsAreConsumedOnceAndSurviveSameScopeRefresh() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    _ = try store.publish(snapshot(scope: "alice"))
    let request = try store.enqueue(NativeAction(kind: "flowpilot"))
    _ = try store.publish(snapshot(scope: "alice"))
    let actions = try store.takeActions()
    #expect(actions.count == 1)
    #expect(actions.first?.id == request.id)
    #expect(try store.takeActions().isEmpty)
}

@Test func onlyExposedEventsCanBeQueued() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    _ = try store.publish(snapshot(scope: "alice"))
    #expect(throws: (any Error).self) {
        try store.enqueue(NativeAction(kind: "run_event", appId: "app", eventId: "not-published"))
    }
}

@Test func pendingActionsSurviveReadsAndRestartUntilScopedAcknowledgement() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    try store.publish(snapshot(scope: "alice"))
    let first = try store.enqueue(NativeAction(kind: "flowpilot", prompt: "Question"))
    let second = try store.enqueue(NativeAction(kind: "open_home"))
    #expect(try store.pendingActions().map(\.id) == [first.id, second.id])
    let reopened = NativeStore(directory: directory)
    #expect(try reopened.pendingActions().map(\.id) == [first.id, second.id])
    try reopened.acknowledgeAction(id: first.id, scope: "bob")
    #expect(try reopened.pendingActions().count == 2)
    try reopened.acknowledgeAction(id: first.id, scope: "alice")
    try reopened.acknowledgeAction(id: first.id, scope: "alice")
    #expect(try reopened.pendingActions().map(\.id) == [second.id])
    #expect(try reopened.takeActions().map(\.id) == [second.id])
    #expect(try reopened.pendingActions().isEmpty)
}

@Test func pendingActionsPruneExpiredRequestsWithoutRevivingThemOnRefresh() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    try store.publish(snapshot(scope: "alice"))
    let old = NativeActionRequest(id: UUID().uuidString, scope: "alice", action: NativeAction(kind: "flowpilot"),
                                  createdAt: ISO8601DateFormatter().string(from: Date().addingTimeInterval(-600)))
    try JSONEncoder().encode([old]).write(to: directory.appendingPathComponent("actions.json"))
    #expect(try store.pendingActions().isEmpty)
    #expect(try JSONDecoder().decode([NativeActionRequest].self, from: Data(contentsOf: directory.appendingPathComponent("actions.json"))).isEmpty)
    try store.publish(snapshot(scope: "alice"))
    #expect(try store.pendingActions().isEmpty)
}

@Test func refreshingSnapshotDoesNotReviveExpiredActions() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    _ = try store.publish(snapshot(scope: "alice"))
    let old = NativeActionRequest(id: UUID().uuidString, scope: "alice", action: NativeAction(kind: "flowpilot"),
                                  createdAt: ISO8601DateFormatter().string(from: Date().addingTimeInterval(-600)))
    try JSONEncoder().encode([old]).write(to: directory.appendingPathComponent("actions.json"))
    _ = try store.publish(snapshot(scope: "alice"))
    #expect(try store.takeActions().isEmpty)
}

@Test func expiredWidgetCacheStillAcceptsScopedNavigationAndShares() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    _ = try store.publish(snapshot(scope: "alice"))
    try snapshot(scope: "alice", expires: .distantPast).write(to: directory.appendingPathComponent("snapshot.json"))
    _ = try store.enqueue(NativeAction(kind: "flowpilot"))
    _ = try store.enqueue(NativeAction(kind: "share", text: "Incoming text"))
    #expect(throws: (any Error).self) {
        try store.enqueue(NativeAction(kind: "run_event", appId: "app", eventId: "event"))
    }
    let actions = try store.takeActions()
    #expect(actions.map(\.action.kind) == ["flowpilot", "share"])
    #expect(actions.allSatisfy { $0.scope == "alice" })
}

@Test func sharedFilesExpireWithoutDeletingLiveQueuedAttachments() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    _ = try store.publish(snapshot(scope: "alice"))
    let source = directory.appendingPathComponent("input.txt")
    try Data("hello".utf8).write(to: source)
    let expired = try store.importSharedFile(source)
    let queued = try store.importSharedFile(source)
    for file in [expired, queued] {
        try FileManager.default.setAttributes([.modificationDate: Date().addingTimeInterval(-90_000)], ofItemAtPath: file.path)
    }
    _ = try store.enqueue(NativeAction(kind: "share", files: [queued.path]))
    _ = try store.importSharedFile(source)
    #expect(!FileManager.default.fileExists(atPath: expired.path))
    #expect(FileManager.default.fileExists(atPath: queued.path))
}

@Test func oversizedActionQueueRejectsNewShareAndPreservesEarlierContent() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    _ = try store.publish(snapshot(scope: "alice"))
    let text = String(repeating: "x", count: 900_000)
    let first = try store.enqueue(NativeAction(kind: "share", text: text))
    let second = try store.enqueue(NativeAction(kind: "share", text: text))
    #expect(throws: (any Error).self) { try store.enqueue(NativeAction(kind: "share", text: text)) }
    #expect(try store.takeActions().map(\.id) == [first.id, second.id])
}

@Test func sharedFilesRejectCapacityOverflowAndPruneOldUnqueuedFiles() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    let folder = try store.sharedFilesDirectory()
    let source = directory.appendingPathComponent("input.txt")
    try Data([1]).write(to: source)
    let files = try (0..<64).map { _ in try store.importSharedFile(source) }
    #expect(throws: (any Error).self) { try store.importSharedFile(source) }
    try FileManager.default.setAttributes([.modificationDate: Date().addingTimeInterval(-600)], ofItemAtPath: files[0].path)
    _ = try store.importSharedFile(source)
    #expect(!FileManager.default.fileExists(atPath: files[0].path))
    #expect(try FileManager.default.contentsOfDirectory(atPath: folder.path).count == 64)
}

@Test func sharedFilesPruneToByteBudget() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    let folder = try store.sharedFilesDirectory()
    let old = folder.appendingPathComponent(UUID().uuidString + "-old.bin")
    #expect(FileManager.default.createFile(atPath: old.path, contents: nil))
    let file = try FileHandle(forWritingTo: old)
    try file.truncate(atOffset: 134_217_728)
    try file.close()
    try FileManager.default.setAttributes([.modificationDate: Date().addingTimeInterval(-600)], ofItemAtPath: old.path)
    let source = directory.appendingPathComponent("input.txt")
    try Data([1]).write(to: source)
    _ = try store.importSharedFile(source)
    #expect(!FileManager.default.fileExists(atPath: old.path))
}

@Test func appNavigationPreservesRawRouteAndOrderedQueryValues() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    _ = try store.publish(snapshot(scope: "alice", apps: [["id": "app", "title": "Reports", "spotlightEligible": true]]))
    let query = try NativeAppRoute.queryParameters(
        names: ["filter", "filter", "id", "empty"],
        values: ["München + Zürich", "A&B#50%", "another-app", ""])
    let action = NativeAction(kind: "open_app", appId: "app", path: "/reports/日本語?tag=one&tag=two%2Bthree", queryParams: query)
    let request = try store.enqueue(action, expectedScope: "alice")
    let queued = try #require(store.takeActions().first)
    #expect(queued.id == request.id)
    #expect(queued.scope == "alice")
    #expect(queued.action == action)
    #expect(queued.action.queryParams?.map(\.name) == ["filter", "filter", "id", "empty"])
    #expect(queued.action.queryParams?.map(\.value) == ["München + Zürich", "A&B#50%", "another-app", ""])
}

@Test func appNavigationRequiresCurrentAppMembershipAndOriginalScope() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    _ = try store.publish(snapshot(scope: "alice", apps: [["id": "app", "title": "Reports"]]))
    #expect(throws: NativeIntegrationError.invalidAction) {
        try store.enqueue(NativeAction(kind: "open_app", appId: "missing"))
    }
    #expect(throws: NativeIntegrationError.invalidAction) {
        try store.enqueue(NativeAction(kind: "open_app", appId: "app"), expectedScope: "bob")
    }
    #expect(throws: NativeIntegrationError.invalidAction) {
        try store.enqueue(NativeAction(kind: "flowpilot", path: "/reports"))
    }
    _ = try store.enqueue(NativeAction(kind: "open_app", appId: "app"), expectedScope: "alice")
    _ = try store.publish(snapshot(scope: "bob", apps: [["id": "app", "title": "Different app"]]))
    #expect(throws: NativeIntegrationError.invalidAction) {
        try store.enqueue(NativeAction(kind: "open_app", appId: "app"), expectedScope: "alice")
    }
    #expect(try store.takeActions().isEmpty)
}

@Test func appRouteConfigurationRejectsExternalTraversalAndMismatchedQueries() throws {
    for path in ["https://example.com", "flow-like://native/action", "//example.com", "/\\example.com", "/../settings", "reports/./today", "/%2e%2e/settings", "/%2e%2e/settings%", "/%2Fexample.com", "/reports#secret", "/reports\n", "/reports%0A", "/reports?name=%00", "/reports?=value"] {
        #expect(throws: NativeIntegrationError.invalidRoute) {
            try NativeAppRoute.validate(path: path, queryParams: nil)
        }
    }
    for path in ["", "/", "reports/today", "/settings", "/reports/12:30", "/reports/日本語", "/reports?url=https://example.com&filter=A%26B"] {
        try NativeAppRoute.validate(path: path, queryParams: nil)
    }
    #expect(throws: NativeIntegrationError.invalidRoute) {
        try NativeAppRoute.queryParameters(names: ["a", "b"], values: ["one"])
    }
    #expect(throws: NativeIntegrationError.invalidRoute) {
        try NativeAppRoute.queryParameters(names: [""], values: ["one"])
    }
    #expect(throws: NativeIntegrationError.invalidRoute) {
        try NativeAppRoute.validate(path: String(repeating: "é", count: 2049), queryParams: nil)
    }
    #expect(throws: NativeIntegrationError.invalidRoute) {
        try NativeAppRoute.queryParameters(names: Array(repeating: "a", count: 33), values: Array(repeating: "b", count: 33))
    }
    #expect(throws: NativeIntegrationError.invalidRoute) {
        try NativeAppRoute.validate(path: "/reports?" + Array(repeating: "a=b", count: 32).joined(separator: "&"), queryParams: [NativeQueryParameter(name: "last", value: "value")])
    }
    try NativeAppRoute.validate(path: "/reports", queryParams: [NativeQueryParameter(name: "emoji", value: "👩‍💻")])
    #expect(throws: NativeIntegrationError.invalidRoute) {
        try NativeAppRoute.validate(path: String(repeating: "a", count: 4096), queryParams: [NativeQueryParameter(name: "name", value: String(repeating: "b", count: 4096))])
    }
}

@Test func appEntityIdentifiersAreScopedAndUnambiguous() {
    let app = NativeApp(id: "reports", title: "Reports", spotlightEligible: true)
    let first = FlowAppEntity(app, scope: "é:alice")
    let second = FlowAppEntity(app, scope: "bob")
    #expect(first.id != second.id)
    #expect(first.id == "8:é:alicereports")
    #expect(first.sourceId == "reports")
    #expect(first.scope == "é:alice")
}

private func snapshot(scope: String, expires: Date = Date().addingTimeInterval(3600), apps: [[String: Any]] = []) throws -> Data {
    try JSONSerialization.data(withJSONObject: [
        "version": 1, "scope": scope, "generatedAt": ISO8601DateFormatter().string(from: Date()),
        "expiresAt": ISO8601DateFormatter().string(from: expires), "sections": [], "events": [], "apps": apps
    ])
}

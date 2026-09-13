import AppIntents
import Foundation
import Testing
@testable import FlowLikeNative

@Test func askFlowPilotRequiresAQuestionBeforeEnqueuing() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = try intentStore(directory)
    let intent = AskFlowPilotIntent()
    for question in [nil, "", " \n "] as [String?] {
        intent.question = question
        #expect(throws: (any Error).self) { try intent.enqueue(in: store) }
        #expect(try store.pendingActions().isEmpty)
    }
    intent.question = "  Summarize my recent activity  "
    let queued = try intent.enqueue(in: store)
    #expect(queued.action.prompt == "Summarize my recent activity")
    #expect(queued.scope == "alice")
    #expect(try store.pendingActions().map(\.id) == [queued.id])
}

@Test func voiceLaunchIsSeparateFromAnAskThatMustReturnAnAnswer() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = try intentStore(directory)
    let intent = AskFlowPilotIntent()
    intent.voice = true
    #expect(throws: (any Error).self) { try intent.enqueue(in: store) }
    #expect(try store.pendingActions().isEmpty)
    #expect(OpenFlowPilotIntent.isDiscoverable == false)
    #expect(OpenFlowPilotIntent(voice: true).voice)
    #expect(OpenFlowPilotIntent().voice == false)
}

@Test func actionResultSurvivesDeliveryAcknowledgementAndReturnsTypedProperties() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = try intentStore(directory)
    let request = try store.enqueue(NativeAction(kind: "flowpilot", prompt: "Hello"), responseMode: "result")
    try store.acknowledgeAction(id: request.id, scope: request.scope)
    #expect(try store.pendingActions().isEmpty)
    let result = NativeActionResult(id: request.id, scope: request.scope, status: "success", text: "日本語 + & %", json: "{\"count\":2}")
    try store.completeAction(result)
    try store.completeAction(result)
    let resumed = NativeStore(directory: directory)
    let returned = try await resumed.waitForActionResult(request)
    let entity = FlowEventResultEntity(returned)
    #expect(entity.text == "日本語 + & %")
    #expect(entity.json == "{\"count\":2}")
    #expect(entity.transferableText == entity.text)
    let jsonOnly = FlowEventResultEntity(NativeActionResult(id: "json-only", scope: request.scope, status: "success", json: "{\"count\":2}"))
    #expect(jsonOnly.text.isEmpty)
    #expect(jsonOnly.transferableText == "{\"count\":2}")
    #expect(jsonOnly.dialogText == "The Event returned JSON output.")
    #expect(FlowEventResultEntity().dialogText == "The Event finished without returning an output.")
    #expect(throws: NativeIntegrationError.expired) { try resumed.completeAction(result) }
}

@Test func actionResultRejectsForeignUnknownMalformedAndOversizedCompletions() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = try intentStore(directory)
    let request = try store.enqueue(NativeAction(kind: "flowpilot", prompt: "Hello"), responseMode: "text")
    for result in [
        NativeActionResult(id: request.id, scope: "bob", status: "success", text: "Other account"),
        NativeActionResult(id: "unregistered", scope: request.scope, status: "success", text: "Unknown request"),
        NativeActionResult(id: request.id, scope: request.scope, status: "success", json: "{\"only\":\"json\"}"),
        NativeActionResult(id: request.id, scope: request.scope, status: "success", text: "OK", json: "not json"),
        NativeActionResult(id: request.id, scope: request.scope, status: "success", text: String(repeating: "x", count: 196_609)),
    ] {
        #expect(throws: (any Error).self) { try store.completeAction(result) }
    }
    #expect(try store.pendingActions().map(\.id) == [request.id])
    let result = NativeActionResult(id: request.id, scope: request.scope, status: "success", text: "Accepted")
    try store.completeAction(result)
    #expect(throws: NativeIntegrationError.invalidAction) {
        try store.completeAction(NativeActionResult(id: request.id, scope: request.scope, status: "success", text: "Overwrite"))
    }
}

@Test func timedOutOrCancelledResultRequestsCannotExecuteLate() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = try intentStore(directory)
    let request = try store.enqueue(NativeAction(kind: "flowpilot", prompt: "Hello"), responseMode: "text", responseTimeout: 0.02)
    await #expect(throws: NativeActionFailure.self) { try await store.waitForActionResult(request) }
    #expect(try store.pendingActions().isEmpty)
    #expect(throws: NativeIntegrationError.expired) {
        try store.completeAction(NativeActionResult(id: request.id, scope: request.scope, status: "success", text: "Too late"))
    }
    let cancelled = try store.enqueue(NativeAction(kind: "flowpilot", prompt: "Cancel"), responseMode: "text")
    let waiting = Task { try await store.waitForActionResult(cancelled) }
    waiting.cancel()
    await #expect(throws: CancellationError.self) { try await waiting.value }
    #expect(try store.pendingActions().isEmpty)
}

@Test func interactionRequiredAndAccountChangesCannotReturnFalseSuccess() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = try intentStore(directory)
    let request = try store.enqueue(NativeAction(kind: "flowpilot", prompt: "Confirm"), responseMode: "text")
    try store.completeAction(NativeActionResult(id: request.id, scope: request.scope, status: "interaction_required", error: "Approve the tool in Flow Like."))
    await #expect(throws: NativeActionFailure.self) { try await store.waitForActionResult(request) }
    let changed = try store.enqueue(NativeAction(kind: "flowpilot", prompt: "Old account"), responseMode: "text")
    var next = try #require(store.catalog())
    next.scope = "bob"
    try store.publish(JSONEncoder().encode(next))
    await #expect(throws: NativeIntegrationError.invalidAction) { try await store.waitForActionResult(changed) }
    #expect(try store.pendingActions().isEmpty)
}

@Test func savedEventIntentSurvivesWidgetExpiryButRejectsChangedAccountOrExposure() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = try intentStore(directory)
    let event = try #require(store.catalog()?.events.first)
    var expired = try #require(store.catalog())
    expired.expiresAt = "2000-01-01T00:00:00Z"
    try JSONEncoder().encode(expired).write(to: directory.appendingPathComponent("snapshot.json"))
    #expect(store.snapshot() == nil)
    let intent = RunFlowEventIntent()
    intent.event = FlowEventEntity(event, scope: "alice")
    intent.input = "{\"question\":\"日本語 + & %\"}"
    _ = try intent.enqueue(in: store)
    let request = try #require(store.pendingActions().first)
    #expect(request.action.eventId == "event")
    #expect(request.action.text == intent.input)
    #expect(request.scope == "alice")

    expired.expiresAt = ISO8601DateFormatter().string(from: Date().addingTimeInterval(3600))
    expired.events[0].surfaces = ["widget"]
    try store.publish(JSONEncoder().encode(expired))
    #expect(throws: NativeIntegrationError.invalidAction) { try intent.enqueue(in: store) }
    expired.scope = "bob"
    try store.publish(JSONEncoder().encode(expired))
    #expect(throws: NativeIntegrationError.invalidAction) { try intent.enqueue(in: store) }
    #expect(try store.pendingActions().isEmpty)
}

@Test func appChatIntentReturnsTextModeOnlyForAnExposedChatEvent() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = try intentStore(directory)
    var snapshot = try #require(store.catalog())
    snapshot.events[0].eventType = "simple_chat"
    snapshot.events[0].action.kind = "open_event"
    try store.publish(JSONEncoder().encode(snapshot))
    let intent = AskAppChatIntent()
    intent.chat = FlowChatEventEntity(snapshot.events[0], scope: snapshot.scope)
    intent.question = "  How many orders?  "
    let request = try intent.enqueue(in: store)
    #expect(request.responseMode == "text")
    #expect(request.responseDeadline != nil)
    #expect(request.action.text == "How many orders?")
    #expect(request.action.eventId == "event")
    snapshot.events[0].eventType = "page"
    try store.publish(JSONEncoder().encode(snapshot))
    #expect(throws: NativeIntegrationError.invalidAction) { try intent.enqueue(in: store) }
}

private func intentStore(_ directory: URL) throws -> NativeStore {
    let store = NativeStore(directory: directory)
    try store.publish(JSONSerialization.data(withJSONObject: [
        "version": 1, "scope": "alice", "generatedAt": ISO8601DateFormatter().string(from: Date()),
        "expiresAt": ISO8601DateFormatter().string(from: Date().addingTimeInterval(3600)),
        "sections": [], "apps": [["id": "app", "title": "Reports", "spotlightEligible": true]],
        "events": [["id": "app:event", "appId": "app", "eventId": "event", "title": "Summarize",
                    "eventType": "quick_action", "surfaces": ["siri", "shortcuts"],
                    "action": ["kind": "run_event", "appId": "app", "eventId": "event"]]],
    ]))
    return store
}

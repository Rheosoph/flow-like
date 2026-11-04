import AppIntents
import Foundation
import Testing
@testable import FlowLikeNative

private func botSnapshot() throws -> NativeSnapshot {
    try JSONDecoder().decode(NativeSnapshot.self, from: JSONSerialization.data(withJSONObject: [
        "version": 1, "scope": "hub:alice", "userId": "alice",
        "generatedAt": NativeSnapshot.string(Date()), "expiresAt": NativeSnapshot.string(Date().addingTimeInterval(3600)),
        "apps": [], "events": [], "sections": [],
        "bots": [["id": "research", "title": "Research", "surfaces": ["siri", "shortcuts", "share", "widget", "spotlight"], "enabled": true]],
        "conversations": [["id": "chat", "botId": "research", "title": "Research chat"], ["id": "group", "title": "Group"]],
        "botTasks": [["id": "daily", "botId": "research", "title": "Daily summary", "enabled": true]],
        "botApprovals": [["id": "approval", "botId": "research", "turnId": "turn", "title": "Review", "status": "waiting"]],
    ]))
}

@Test func botActionsRejectForeignScopesRemovedBotsAndPausedTasks() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    var snapshot = try botSnapshot()
    try store.publish(JSONEncoder().encode(snapshot))
    let action = NativeAction(kind: "run_bot_task", botId: "research", taskId: "daily")
    #expect(throws: NativeIntegrationError.invalidAction) { try store.enqueue(action, expectedScope: "other:alice") }
    let request = try store.enqueue(action, expectedScope: snapshot.scope)
    #expect(request.action.taskId == "daily")
    snapshot.botTasks?[0].enabled = false
    try store.publish(JSONEncoder().encode(snapshot))
    #expect(throws: NativeIntegrationError.invalidAction) { try store.enqueue(action, expectedScope: snapshot.scope) }
    snapshot.bots = []
    try store.publish(JSONEncoder().encode(snapshot))
    #expect(throws: NativeIntegrationError.invalidAction) { try store.enqueue(NativeAction(kind: "open_bot", botId: "research")) }
}

@Test func botIntentQueuesSelectedBotAndRechecksExposure() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    var snapshot = try botSnapshot()
    try store.publish(JSONEncoder().encode(snapshot))
    let intent = AskFlowBotIntent()
    intent.bot = FlowBotEntity(try #require(snapshot.bots?.first), scope: snapshot.scope)
    intent.question = "  Summarize this week  "
    let request = try intent.enqueue(in: store)
    #expect(request.action.botId == "research")
    #expect(request.action.prompt == "Summarize this week")
    #expect(request.responseMode == "text")
    snapshot.bots?[0].surfaces = ["widget"]
    try store.publish(JSONEncoder().encode(snapshot))
    #expect(throws: NativeIntegrationError.invalidAction) { try intent.enqueue(in: store) }
    #expect(throws: NativeIntegrationError.invalidAction) {
        try store.enqueue(NativeAction(kind: "share", text: "private", botId: "research"))
    }
}

@Test func notificationDecisionsRequireMatchingAccountHubAndUnexpiredRequest() throws {
    let snapshot = try botSnapshot()
    var payload: [AnyHashable: Any] = ["user_id": "alice", "native_scope": "hub:alice", "bot_id": "research", "turn_id": "turn", "approval_id": "approval"]
    let accepted = NativeBotNotifications.action(identifier: NativeBotNotifications.approveOnceAction, userInfo: payload, reply: nil, snapshot: snapshot)
    #expect(accepted?.decision == "once")
    #expect(accepted?.turnId == "turn")
    payload["native_scope"] = "another-hub:alice"
    #expect(NativeBotNotifications.action(identifier: NativeBotNotifications.denyAction, userInfo: payload, reply: nil, snapshot: snapshot) == nil)
    payload["native_scope"] = "hub:alice"
    payload["user_id"] = "bob"
    #expect(NativeBotNotifications.action(identifier: NativeBotNotifications.denyAction, userInfo: payload, reply: nil, snapshot: snapshot) == nil)
    payload["user_id"] = "alice"
    payload["expires_at"] = "2000-01-01T00:00:00Z"
    #expect(NativeBotNotifications.action(identifier: NativeBotNotifications.denyAction, userInfo: payload, reply: nil, snapshot: snapshot) == nil)
}

@Test func pushCanQueueNewConversationAndApprovalWhileRejectingKnownResolvedApproval() throws {
    var snapshot = try botSnapshot()
    let payload: [AnyHashable: Any] = ["user_id": "alice", "native_scope": snapshot.scope, "bot_id": "research", "conversation_id": "new-chat", "turn_id": "new-turn", "approval_id": "new-approval"]
    let reply = NativeBotNotifications.action(identifier: NativeBotNotifications.replyAction, userInfo: payload, reply: "  Continue  ", snapshot: snapshot)
    #expect(reply?.conversationId == "new-chat")
    #expect(reply?.prompt == "Continue")
    #expect(NativeBotNotifications.action(identifier: NativeBotNotifications.denyAction, userInfo: payload, reply: nil, snapshot: snapshot)?.decision == "deny")
    snapshot.botApprovals?[0].status = "answered"
    let action = NativeAction(kind: "respond_bot_approval", botId: "research", turnId: "turn", approvalId: "approval", decision: "once")
    #expect(throws: NativeIntegrationError.invalidAction) { try snapshot.validateBotAction(action) }
    #expect(throws: NativeIntegrationError.invalidAction) {
        try snapshot.validateBotAction(NativeAction(kind: "respond_bot_approval", botId: "research", turnId: "new-turn", approvalId: "new-approval", decision: "always"))
    }
}

@Test func deviceRequestsRejectCrossWorkspaceAndOversizedModelInputBeforeProcessing() throws {
    let snapshot = try botSnapshot()
    for values: [String: Any] in [
        ["operation": "voice", "scope": "other:alice"],
        ["operation": "extract", "scope": snapshot.scope, "text": String(repeating: "x", count: 8193)],
        ["operation": "summarize", "scope": snapshot.scope, "text": " "],
        ["operation": "voice", "scope": snapshot.scope, "botId": "removed"],
    ] {
        let options = try JSONDecoder().decode(NativeBotDeviceOptions.self, from: JSONSerialization.data(withJSONObject: values))
        #expect(throws: NativeBotDeviceFailure.self) { try options.validate(snapshot: snapshot) }
    }
    let options = try JSONDecoder().decode(NativeBotDeviceOptions.self, from: Data("{\"operation\":\"capabilities\"}".utf8))
    try options.validate(snapshot: nil)
}

@Test func botHandoffAndWidgetURLsPreserveScopeAndConversation() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    var snapshot = try botSnapshot()
    snapshot.webOrigin = "https://app.flow-like.com"
    snapshot.activePage = NativePage(title: "Research", url: "https://app.flow-like.com/bots?conversation=chat")
    try store.publish(JSONEncoder().encode(snapshot))
    let activity = try #require(NativeSystemIntegration.handoffActivity(snapshot))
    #expect(NativeActivityBridge.handle(activity, store: store))
    #expect(try store.pendingActions().first?.action.conversationId == "chat")
    let url = NativeWidgetLaunchURL.bot(botId: "research+notes", scope: "hub:alice+work", conversationId: "chat & more")
    let query = try #require(URLComponents(url: url, resolvingAgainstBaseURL: false)).queryItems ?? []
    #expect(query.first(where: { $0.name == "scope" })?.value == "hub:alice+work")
    #expect(query.first(where: { $0.name == "conversationId" })?.value == "chat & more")
    #expect(url.absoluteString.contains("%2B"))
}

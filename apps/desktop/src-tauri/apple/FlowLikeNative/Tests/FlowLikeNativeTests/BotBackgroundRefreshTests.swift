import Foundation
import Testing
import UserNotifications
@testable import FlowLikeNative

private func refreshStore() throws -> (NativeStore, URL, NativeSnapshot) {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let snapshot = try JSONDecoder().decode(NativeSnapshot.self, from: JSONSerialization.data(withJSONObject: [
        "version": 1, "scope": "hub:refresh-\(UUID().uuidString)", "userId": "alice",
        "generatedAt": NativeSnapshot.string(Date().addingTimeInterval(-120)),
        "staleAt": NativeSnapshot.string(Date().addingTimeInterval(180)),
        "expiresAt": NativeSnapshot.string(Date().addingTimeInterval(3600)),
        "apps": [], "events": [],
        "sections": [
            ["kind": "inbox", "title": "Inbox", "state": "ready", "items": []],
            ["kind": "bot_tasks", "title": "Bot tasks", "state": "ready", "items": [
                ["id": "scheduled", "title": "Daily research", "action": ["kind": "run_bot_task", "botId": "research", "taskId": "daily"]],
            ]],
            ["kind": "bot_approvals", "title": "Approvals", "state": "ready", "items": [
                ["id": "approval", "title": "Review action", "action": ["kind": "open_conversation", "botId": "research", "conversationId": "chat", "approvalId": "approval"]],
            ]],
        ],
        "bots": [["id": "research", "title": "Research", "local": false, "encrypted": false, "enabled": true]],
        "botTasks": [["id": "daily", "botId": "research", "title": "Daily research", "enabled": true]],
        "botApprovals": [["id": "approval", "botId": "research", "turnId": "turn", "title": "Review action", "status": "waiting"]],
    ]))
    let store = NativeStore(directory: directory)
    try store.publish(JSONEncoder().encode(snapshot))
    return (store, directory, snapshot)
}

@Test func onlyExplicitSimpleBotDeliveriesReceiveTheLongerRetryWindow() throws {
    let (store, directory, _) = try refreshStore()
    defer { try? FileManager.default.removeItem(at: directory) }
    let action = NativeAction(kind: "ask_bot", prompt: "Continue", botId: "research", conversationId: "chat")
    var foreground = try store.enqueue(action)
    var retry = try store.enqueue(action, backgroundDelivery: true)
    foreground.createdAt = NativeSnapshot.string(Date().addingTimeInterval(-600))
    retry.createdAt = foreground.createdAt
    #expect(!foreground.isCurrent)
    #expect(retry.isCurrent)
    retry.createdAt = NativeSnapshot.string(Date().addingTimeInterval(-86_401))
    #expect(!retry.isCurrent)
    #expect(throws: NativeIntegrationError.invalidAction) {
        try store.enqueue(action, responseMode: "text", backgroundDelivery: true)
    }
    #expect(throws: NativeIntegrationError.invalidAction) {
        try store.enqueue(NativeAction(kind: "capture_bot_voice", botId: "research"), backgroundDelivery: true)
    }
}

@Test func refreshLeaseStopsAtExpiryScopeChangeOrCompletion() {
    let lease = NativeBotRefreshLease()
    let now = Date()
    #expect(lease.begin(id: "refresh", scope: "hub:alice", actionIDs: ["reply"], now: now))
    #expect(!lease.begin(id: "overlap", scope: "hub:alice", actionIDs: [], now: now))
    #expect(lease.active(id: "refresh", scope: "hub:alice", persistedScope: "hub:alice", now: now.addingTimeInterval(24)))
    #expect(!lease.active(id: "refresh", scope: "hub:alice", persistedScope: "hub:bob", now: now))
    #expect(!lease.active(id: "refresh", scope: "hub:alice", persistedScope: "hub:alice", now: now.addingTimeInterval(25)))
    #expect(lease.ownedIDs(scope: "hub:alice", now: now) == ["reply"])
    #expect(lease.ownedIDs(scope: "hub:bob", now: now).isEmpty)
    #expect(lease.end(id: "unrelated") == nil)
    #expect(lease.end(id: "refresh")?.actionIDs == ["reply"])
    #expect(lease.end(id: "refresh") == nil)
    #expect(!lease.active(id: "refresh", scope: "hub:alice", persistedScope: "hub:alice", now: now))
}

@Test func backgroundOwnedActionsAreHiddenFromForegroundUntilTheLeaseEnds() throws {
    let (store, directory, snapshot) = try refreshStore()
    defer { try? FileManager.default.removeItem(at: directory) }
    let request = try store.enqueue(NativeAction(kind: "ask_bot", prompt: "Continue", botId: "research", conversationId: "chat"), backgroundDelivery: true)
    let id = UUID().uuidString
    #expect(NativeBotRefreshBridge.lease.begin(id: id, scope: snapshot.scope, actionIDs: [request.id]))
    defer { NativeBotRefreshBridge.lease.end(id: id) }
    #expect(try store.pendingActions().isEmpty)
    #expect(try store.takeActions().isEmpty)
    NativeBotRefreshBridge.lease.end(id: id)
    #expect(try store.pendingActions().map(\.id) == [request.id])
}

@Test func partialRefreshAcknowledgesOnlyOwnedDeliveriesAndRetainsVisibleTerminalErrors() throws {
    let (store, directory, snapshot) = try refreshStore()
    defer { try? FileManager.default.removeItem(at: directory) }
    let action = NativeAction(kind: "ask_bot", prompt: "Continue", botId: "research", conversationId: "chat")
    let sent = try store.enqueue(action, backgroundDelivery: true)
    let failed = try store.enqueue(action, backgroundDelivery: true)
    let foreground = try store.enqueue(NativeAction(kind: "open_home"))
    let result = NativeBotRefreshResult(success: false, deliveredIds: [sent.id, foreground.id],
        failedActions: [.init(id: failed.id, error: "This approval expired.")])
    try store.applyBotRefresh(result, scope: snapshot.scope, allowedIDs: [sent.id, failed.id])
    let pending = try store.pendingActions()
    #expect(!pending.contains(where: { $0.id == sent.id }))
    #expect(pending.contains(where: { $0.id == foreground.id }))
    #expect(pending.first(where: { $0.id == failed.id })?.backgroundError == "This approval expired.")
    #expect(try store.botRefreshPayload(includeActions: true).actions.isEmpty)
    #expect(store.catalog()?.generatedAt == snapshot.generatedAt)
    #expect(throws: NativeIntegrationError.invalidAction) {
        try store.applyBotRefresh(result, scope: "other:alice", allowedIDs: [sent.id])
    }
}

@Test func statusRefreshUpdatesOnlyTurnRowsWithoutRefreshingOtherSectionsOrTaskMetadata() throws {
    let (store, directory, snapshot) = try refreshStore()
    defer { try? FileManager.default.removeItem(at: directory) }
    let now = Date()
    let updated = NativeSnapshot.string(now.addingTimeInterval(-10))
    let result = NativeBotRefreshResult(success: true, turns: [
        .init(id: "active", bot_id: "research", conversation_id: "group", status: "waiting", updated_at: updated),
        .init(id: "ended", bot_id: "research", conversation_id: "chat", status: "done"),
        .init(id: "foreign", bot_id: "another-bot", conversation_id: "foreign", status: "running"),
    ])
    try store.applyBotRefresh(result, scope: snapshot.scope, allowedIDs: [], now: now)
    let refreshed = try #require(store.catalog())
    #expect(refreshed.generatedAt == snapshot.generatedAt)
    #expect(refreshed.staleAt == snapshot.staleAt)
    #expect(refreshed.expiresAt == snapshot.expiresAt)
    let inbox = try #require(refreshed.sections.first(where: { $0.kind == "inbox" }))
    #expect(refreshed.freshness(of: inbox) == snapshot.freshness(of: inbox))
    let tasks = try #require(refreshed.sections.first(where: { $0.kind == "bot_tasks" }))
    #expect(tasks.items.map(\.id) == ["active"])
    #expect(tasks.items.first?.action.conversationId == "group")
    #expect(tasks.items.first?.updatedAt == updated)
    #expect(tasks.updatedAt == NativeSnapshot.string(now))
    #expect(refreshed.botTasks?.first?.title == "Daily research")
}

@Test func deliveredDecisionsRemoveResolvedCardsWithoutRefreshingTheApprovalList() throws {
    let (store, directory, snapshot) = try refreshStore()
    defer { try? FileManager.default.removeItem(at: directory) }
    let action = NativeAction(kind: "respond_bot_approval", botId: "research", conversationId: "chat",
        turnId: "turn", approvalId: "approval", decision: "once")
    let request = try store.enqueue(action, backgroundDelivery: true)
    try store.applyBotRefresh(.init(success: false, deliveredIds: [request.id]), scope: snapshot.scope, allowedIDs: [request.id])
    let refreshed = try #require(store.catalog())
    #expect(refreshed.botApprovals?.isEmpty == true)
    let approvals = try #require(refreshed.sections.first(where: { $0.kind == "bot_approvals" }))
    #expect(approvals.items.isEmpty)
    #expect(refreshed.freshness(of: approvals) == snapshot.freshness(of: approvals))
    #expect(refreshed.generatedAt == snapshot.generatedAt)
}

@Test func rejectedBotNotificationsNeverReachGenericTapStorageAndPlainRepliesCanRetry() throws {
    let (store, directory, snapshot) = try refreshStore()
    defer { try? FileManager.default.removeItem(at: directory) }
    var payload: [AnyHashable: Any] = ["user_id": "alice", "native_scope": snapshot.scope,
        "bot_id": "research", "conversation_id": "chat", "conversation_encryption": "plain"]
    #expect(NativeBotNotifications.handle(category: NativeBotNotifications.messageCategory,
        identifier: NativeBotNotifications.replyAction, userInfo: payload, reply: "Continue", store: store))
    let reply = try #require(store.pendingActions().first)
    #expect(reply.backgroundDelivery == true)
    try store.acknowledgeAction(id: reply.id, scope: snapshot.scope)
    payload["native_scope"] = "another-hub:alice"
    #expect(NativeBotNotifications.handle(category: NativeBotNotifications.messageCategory,
        identifier: NativeBotNotifications.replyAction, userInfo: payload, reply: "Private", store: store))
    #expect(try store.pendingActions().isEmpty)
    #expect(NativeBotNotifications.handle(category: NativeBotNotifications.reviewCategory,
        identifier: UNNotificationDefaultActionIdentifier, userInfo: payload, reply: nil, store: store))
    #expect(try store.pendingActions().first?.action.kind == "open_home")
    #expect(!NativeBotNotifications.handle(category: "unrelated", identifier: "tap", userInfo: payload, reply: nil, store: store))
}

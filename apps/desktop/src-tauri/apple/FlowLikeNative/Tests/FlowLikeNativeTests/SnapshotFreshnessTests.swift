import Foundation
import Testing
@testable import FlowLikeNative

@Test func freshSectionsTurnStaleBeforeTheyExpire() throws {
    let now = Date()
    let snapshot = try decode(snapshotJSON(generated: now))
    let freshness = try #require(snapshot.freshness(of: snapshot.sections[0]))
    #expect(!freshness.isStale(at: now))
    #expect(freshness.isStale(at: now.addingTimeInterval(2 * 3600)))
    #expect(!freshness.isExpired(at: now.addingTimeInterval(6 * 86_400)))
    #expect(freshness.isExpired(at: now.addingTimeInterval(8 * 86_400)))
}

@Test func snapshotsWithoutStaleAtStayFreshUntilTheyExpire() throws {
    let now = Date()
    var object = snapshotJSON(generated: now, retention: 3600)
    object.removeValue(forKey: "staleAt")
    let snapshot = try decode(object)
    let freshness = try #require(snapshot.freshness(of: snapshot.sections[0]))
    #expect(freshness.staleAt == freshness.expiresAt)
}

@Test func failedSectionReadKeepsLastGoodItemsWithTheirAge() throws {
    let directory = temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    let earlier = Date().addingTimeInterval(-2 * 3600)
    try store.publish(data(snapshotJSON(generated: earlier)))
    let stored = try store.publish(data(snapshotJSON(generated: Date(), inbox: "unavailable")))
    let inbox = try #require(stored.sections.first { $0.kind == "inbox" })
    #expect(inbox.state == "ready")
    #expect(inbox.items.map(\.id) == ["n1"])
    let freshness = try #require(stored.freshness(of: inbox))
    #expect(abs(freshness.updatedAt.timeIntervalSince(earlier)) < 1)
    #expect(freshness.isStale(at: Date()))
    #expect(try #require(store.snapshot()).sections.first { $0.kind == "inbox" }?.items.count == 1)
}

@Test func carriedSectionsKeepTheirOriginalPrivacyWindow() throws {
    let start = Date()
    let first = try decode(snapshotJSON(generated: start))
    let second = try decode(snapshotJSON(generated: start.addingTimeInterval(3 * 86_400), inbox: "unavailable"))
        .retainingLastGood(from: first, now: start.addingTimeInterval(3 * 86_400))
    let carried = try #require(second.freshness(of: second.sections[0]))
    #expect(abs(carried.updatedAt.timeIntervalSince(start)) < 1)
    #expect(abs(carried.expiresAt.timeIntervalSince(start.addingTimeInterval(7 * 86_400))) < 1)

    let later = start.addingTimeInterval(8 * 86_400)
    let third = try decode(snapshotJSON(generated: later, inbox: "unavailable")).retainingLastGood(from: second, now: later)
    #expect(third.sections[0].state == "unavailable")
    #expect(third.sections[0].items.isEmpty)
}

@Test func lastGoodDataNeverCrossesAccountsOrSignOut() throws {
    let now = Date()
    let alice = try decode(snapshotJSON(generated: now))
    let bob = try decode(snapshotJSON(generated: now, scope: "bob", inbox: "unavailable")).retainingLastGood(from: alice, now: now)
    #expect(bob.sections[0].state == "unavailable")
    let signedOut = try decode(snapshotJSON(generated: now, inbox: "signed_out")).retainingLastGood(from: alice, now: now)
    #expect(signedOut.sections[0].state == "signed_out")
    #expect(signedOut.sections[0].items.isEmpty)
}

@Test func carriedItemsDropAppsTheAccountNoLongerHas() throws {
    let now = Date()
    var object = snapshotJSON(generated: now)
    object["sections"] = [[
        "kind": "recent_runs", "title": "Recent runs", "state": "ready",
        "items": [
            ["id": "kept", "title": "Kept", "action": ["kind": "open_run", "appId": "app", "runId": "kept"]],
            ["id": "gone", "title": "Gone", "action": ["kind": "open_run", "appId": "removed", "runId": "gone"]],
        ],
    ]]
    var failed = snapshotJSON(generated: now)
    failed["sections"] = [["kind": "recent_runs", "title": "Recent runs", "state": "unavailable", "items": []]]
    let result = try decode(failed).retainingLastGood(from: decode(object), now: now)
    #expect(result.sections[0].items.map(\.id) == ["kept"])
}

@Test func failedAppReadKeepsAppsEventsAndCustomWidgets() throws {
    let directory = temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    let now = Date()
    try store.publish(data(snapshotJSON(generated: now, customWidgets: [chartWidget(now: now)])))
    var failed = snapshotJSON(generated: now, apps: [], events: [], customWidgets: [chartWidget(now: now)])
    failed["appsUnavailable"] = true
    let stored = try store.publish(data(failed))
    #expect(stored.apps.map(\.id) == ["app"])
    #expect(stored.events.map(\.id) == ["app:event"])
    #expect(stored.customWidgets?.first?.state == "ready")
    #expect(stored.appsUnavailable == nil)
}

@Test func failedEventReadKeepsThatAppsEventsAndEligibility() throws {
    let now = Date()
    let previous = try decode(snapshotJSON(generated: now))
    var object = snapshotJSON(generated: now, apps: [["id": "app", "title": "App", "spotlightEligible": false]], events: [])
    object["eventsUnavailable"] = ["app"]
    let result = try decode(object).retainingLastGood(from: previous, now: now)
    #expect(result.events.map(\.id) == ["app:event"])
    #expect(result.apps.first?.spotlightEligible == true)
    #expect(result.eventsUnavailable == nil)
}

@Test func pendingCustomWidgetKeepsStoredContentForTheSameTarget() throws {
    let directory = temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    let now = Date()
    try store.publish(data(snapshotJSON(generated: now, customWidgets: [chartWidget(now: now)])))

    var renamed = pendingShell(now: now)
    renamed["title"] = "Renamed"
    let kept = try #require(try store.publish(data(snapshotJSON(generated: now, customWidgets: [renamed]))).customWidgets?.first)
    #expect(kept.state == "ready")
    #expect(kept.chart != nil)
    #expect(kept.title == "Renamed")
    #expect(kept.pending == nil)

    var moved = pendingShell(now: now)
    moved["action"] = ["kind": "open_app", "appId": "app", "path": "/other"]
    let shell = try #require(try store.publish(data(snapshotJSON(generated: now, customWidgets: [moved]))).customWidgets?.first)
    #expect(shell.state == "unavailable")
    #expect(shell.chart == nil)
}

@Test func pendingCustomWidgetDropsContentWhenItsQueryOrContainerChanged() throws {
    let directory = temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    let now = Date()
    try store.publish(data(snapshotJSON(generated: now, customWidgets: [chartWidget(now: now)])))
    var retargeted = pendingShell(now: now)
    retargeted["target"] = "fedcba9876543210"
    let shell = try #require(try store.publish(data(snapshotJSON(generated: now, customWidgets: [retargeted]))).customWidgets?.first)
    #expect(shell.state == "unavailable")
    #expect(shell.chart == nil)
}

@Test func liveRunsAreNeverCarriedAsHistory() throws {
    let now = Date()
    var previous = snapshotJSON(generated: now)
    previous["sections"] = [runsSection("ready", [run("finished-live", live: true), run("history")])]
    var incoming = snapshotJSON(generated: now)
    incoming["sections"] = [runsSection("unavailable", [run("current", live: true)])]
    let result = try decode(incoming).retainingLastGood(from: decode(previous), now: now)
    #expect(result.sections[0].state == "ready")
    #expect(result.sections[0].items.map(\.id) == ["current", "history"])
    #expect(try #require(result.freshness(of: result.sections[0])).isStale(at: now.addingTimeInterval(1)))
}

@Test func liveRunsShowWithoutHistoryToCarry() throws {
    let now = Date()
    var incoming = snapshotJSON(generated: now)
    incoming["sections"] = [runsSection("unavailable", [run("current", live: true)])]
    let result = try decode(incoming).retainingLastGood(from: nil, now: now)
    #expect(result.sections[0].state == "unavailable")
    #expect(result.sections[0].items.map(\.id) == ["current"])
}

@Test func failedEventAppKeepsOnlyItsOwnStillExposedFavorites() throws {
    let now = Date()
    let apps: [[String: Any]] = [["id": "a", "title": "A", "spotlightEligible": true], ["id": "b", "title": "B", "spotlightEligible": true]]
    var previous = snapshotJSON(generated: now, apps: apps, events: [event("a", "deleted"), event("b", "kept"), event("b", "hidden")])
    previous["sections"] = [favorites(["a:deleted", "b:kept", "b:hidden"])]
    var incoming = snapshotJSON(generated: now, apps: apps, events: [event("a", "new")])
    incoming["sections"] = [favorites(["a:new"])]
    incoming["eventsUnavailable"] = ["b"]
    var previousSnapshot = try decode(previous)
    previousSnapshot.events[2].surfaces = []
    let result = try decode(incoming).retainingLastGood(from: previousSnapshot, now: now)
    #expect(result.sections[0].items.map(\.id) == ["a:new", "b:kept"])
}

@Test func partlyFailedSectionKeepsFreshItemsAndFillsTheRest() throws {
    let now = Date()
    var previous = snapshotJSON(generated: now.addingTimeInterval(-3600))
    previous["sections"] = [workspace("ready", [metric("apps", "4"), metric("activity", "120")])]
    var incoming = snapshotJSON(generated: now)
    incoming["sections"] = [workspace("unavailable", [metric("apps", "5")])]
    let result = try decode(incoming).retainingLastGood(from: decode(previous), now: now)
    #expect(result.sections[0].items.map(\.value) == ["5", "120"])
}

@Test func ageLabelsChangeHourlyThenDailyUntilExpiry() throws {
    let updated = Date(timeIntervalSince1970: 1_790_000_000)
    let snapshot = try decode(snapshotJSON(generated: updated))
    let freshness = try #require(snapshot.freshness(of: snapshot.sections[0]))
    let dates = NativeWidgetFreshness.labelDates(freshness, after: updated)
    #expect(dates.first == updated.addingTimeInterval(3600))
    #expect(dates.count == 29)
    #expect(dates.last == updated.addingTimeInterval(6 * 86_400))
    #expect(dates.allSatisfy { $0 < freshness.expiresAt })
    #expect(NativeWidgetFreshness.labelDates(freshness, after: updated.addingTimeInterval(5 * 3600 + 1)).first == updated.addingTimeInterval(6 * 3600))
}

@Test func ageLabelsRoundDownToTheLargestUnit() {
    let now = Date()
    #expect(NativeWidgetFreshness.age(since: now.addingTimeInterval(-3 * 3600 - 1800), at: now)
        == NativeWidgetFreshness.age(since: now.addingTimeInterval(-3 * 3600), at: now))
    #expect(NativeWidgetFreshness.age(since: now.addingTimeInterval(-3 * 3600), at: now)
        != NativeWidgetFreshness.age(since: now.addingTimeInterval(-4 * 3600), at: now))
}

private let iso: ISO8601DateFormatter = {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return formatter
}()

private func snapshotJSON(generated: Date, scope: String = "alice", inbox: String = "ready",
                          retention: TimeInterval = 7 * 86_400,
                          apps: [[String: Any]] = [["id": "app", "title": "App", "spotlightEligible": true]],
                          events: [[String: Any]]? = nil,
                          customWidgets: [[String: Any]]? = nil) -> [String: Any] {
    let action: [String: Any] = ["kind": "run_event", "appId": "app", "eventId": "event"]
    var object: [String: Any] = [
        "version": 1, "scope": scope, "generatedAt": iso.string(from: generated),
        "staleAt": iso.string(from: generated.addingTimeInterval(3600)),
        "expiresAt": iso.string(from: generated.addingTimeInterval(retention)),
        "sections": [[
            "kind": "inbox", "title": "Inbox", "state": inbox,
            "items": inbox == "ready" ? [["id": "n1", "title": "Build finished", "action": ["kind": "open_inbox"]]] : [],
        ]],
        "events": events ?? [[
            "id": "app:event", "appId": "app", "eventId": "event", "title": "Sync", "eventType": "simple",
            "action": action, "surfaces": ["widget"],
        ]],
        "apps": apps,
    ]
    if let customWidgets { object["customWidgets"] = customWidgets }
    return object
}

private func chartWidget(now: Date) -> [String: Any] {
    [
        "id": "chart", "title": "Revenue", "kind": "chart", "appId": "app", "state": "ready", "target": "0123456789abcdef",
        "updatedAt": iso.string(from: now), "staleAt": iso.string(from: now.addingTimeInterval(1800)),
        "expiresAt": iso.string(from: now.addingTimeInterval(7 * 86_400)),
        "action": ["kind": "open_app", "appId": "app", "path": "/reports"],
        "chart": ["type": "line", "format": ["style": "number", "currency": "USD", "decimals": 0],
                  "points": [["id": "one", "label": "Monday", "series": "Sales", "value": 12, "formattedValue": "12"]]],
    ]
}

private func pendingShell(now: Date) -> [String: Any] {
    [
        "id": "chart", "title": "Revenue", "kind": "chart", "appId": "app", "state": "unavailable", "pending": true,
        "target": "0123456789abcdef",
        "message": "Open Flow Like to refresh this widget.",
        "updatedAt": iso.string(from: now), "staleAt": iso.string(from: now.addingTimeInterval(1800)),
        "expiresAt": iso.string(from: now.addingTimeInterval(7 * 86_400)),
        "action": ["kind": "open_app", "appId": "app", "path": "/reports"],
    ]
}

private func run(_ id: String, live: Bool = false) -> [String: Any] {
    var item: [String: Any] = ["id": id, "title": id, "status": "running", "action": ["kind": "open_run", "appId": "app", "runId": id]]
    if live { item["live"] = true }
    return item
}

private func runsSection(_ state: String, _ items: [[String: Any]]) -> [String: Any] {
    ["kind": "recent_runs", "title": "Recent runs", "state": state, "items": items]
}

private func event(_ appId: String, _ eventId: String) -> [String: Any] {
    ["id": "\(appId):\(eventId)", "appId": appId, "eventId": eventId, "title": eventId, "eventType": "simple",
     "action": ["kind": "run_event", "appId": appId, "eventId": eventId], "surfaces": ["widget"]]
}

private func favorites(_ ids: [String]) -> [String: Any] {
    ["kind": "event_favorites", "title": "Favorites", "state": "ready", "items": ids.map { id -> [String: Any] in
        let parts = id.split(separator: ":").map(String.init)
        return ["id": id, "title": parts[1], "action": ["kind": "run_event", "appId": parts[0], "eventId": parts[1]]]
    }]
}

private func metric(_ id: String, _ value: String) -> [String: Any] {
    ["id": id, "title": id, "value": value, "action": ["kind": "open_home"]]
}

private func workspace(_ state: String, _ items: [[String: Any]]) -> [String: Any] {
    ["kind": "workspace", "title": "Workspace", "state": state, "items": items]
}

private func decode(_ object: [String: Any]) throws -> NativeSnapshot {
    try JSONDecoder().decode(NativeSnapshot.self, from: data(object))
}

private func data(_ object: [String: Any]) throws -> Data {
    try JSONSerialization.data(withJSONObject: object)
}

private func temporaryDirectory() -> URL {
    FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
}

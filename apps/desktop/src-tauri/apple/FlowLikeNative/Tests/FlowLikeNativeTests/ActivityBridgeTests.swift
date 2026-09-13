import CoreSpotlight
import Foundation
import ObjectiveC
import Testing
@testable import FlowLikeNative

@Test func spotlightActivityQueuesDestinationBeforeFrontendStarts() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    try store.publish(JSONSerialization.data(withJSONObject: [
        "version": 1, "scope": "alice", "generatedAt": ISO8601DateFormatter().string(from: Date()),
        "expiresAt": ISO8601DateFormatter().string(from: Date().addingTimeInterval(3600)),
        "sections": [], "events": [], "apps": [],
    ]))
    let activity = NSUserActivity(activityType: CSSearchableItemActionType)
    activity.userInfo = [CSSearchableItemActivityIdentifier: "flow-like:flowpilot"]
    #expect(NativeActivityBridge.handle(activity, store: store))
    let pending = try store.takeActions()
    #expect(pending.count == 1)
    #expect(pending.first?.action.kind == "flowpilot")
    #expect(pending.first?.scope == "alice")

    activity.userInfo = [CSSearchableItemActivityIdentifier: "flow-like:event:removed"]
    #expect(!NativeActivityBridge.handle(activity, store: store))
    #expect(try store.takeActions().isEmpty)
}

private final class ActivityDelegateFixture: NSObject {
    var forwarded = 0
    @objc func application(_ application: NSObject, continueUserActivity activity: NSUserActivity,
                           restorationHandler: NSObject) -> Bool {
        forwarded += 1
        return false
    }
}

@Test @MainActor func activityHookKeepsExistingDelegateBehaviorAndInstallsOnce() {
    var handled = 0
    NativeActivityBridge.install(delegate: ActivityDelegateFixture.self) { activity in
        guard activity.activityType == CSSearchableItemActionType else { return false }
        handled += 1
        return true
    }
    NativeActivityBridge.install(delegate: ActivityDelegateFixture.self) { _ in
        Issue.record("Installing twice must keep the first hook")
        return false
    }
    let delegate = ActivityDelegateFixture()
    let selector = NSSelectorFromString("application:continueUserActivity:restorationHandler:")
    typealias Handler = @convention(c) (AnyObject, Selector, AnyObject, NSUserActivity, AnyObject) -> Bool
    let implementation = class_getMethodImplementation(ActivityDelegateFixture.self, selector)!
    let invoke = unsafeBitCast(implementation, to: Handler.self)
    #expect(invoke(delegate, selector, NSObject(), NSUserActivity(activityType: CSSearchableItemActionType), NSObject()))
    #expect(handled == 1)
    #expect(delegate.forwarded == 0)
    #expect(!invoke(delegate, selector, NSObject(), NSUserActivity(activityType: "unrelated"), NSObject()))
    #expect(delegate.forwarded == 1)
}

@Test func handoffPreservesAppRouteAndNestedOrderedQueryValues() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = try handoffStore(directory)
    let activity = NSUserActivity(activityType: NativeSystemIntegration.activityType)
    activity.userInfo = ["scope": "alice"]
    activity.webpageURL = URL(string: "https://hub.example/use?id=app&route=%2Freports%2F%E6%97%A5%E6%9C%AC%E8%AA%9E+today&appQuery=filter%3DA%252BB%2526%2523%2525%26filter%3DZ%25C3%25BCrich%2Bname%26id%3Dother")!
    #expect(NativeActivityBridge.handle(activity, store: store))
    let action = try #require(store.takeActions().first?.action)
    #expect(action.kind == "open_app")
    #expect(action.appId == "app")
    #expect(action.path == "/reports/日本語 today")
    #expect(action.queryParams == [
        NativeQueryParameter(name: "filter", value: "A+B&#%"),
        NativeQueryParameter(name: "filter", value: "Zürich name"),
        NativeQueryParameter(name: "id", value: "other"),
    ])
}

@Test func handoffRejectsForeignScopeOriginsAndUnsafeRoutes() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = try handoffStore(directory)
    for (url, scope) in [
        ("https://other.example/use?id=app&route=%2Freports", "alice"),
        ("https://hub.example/use?id=app&route=%2Freports", "bob"),
        ("https://hub.example/settings?id=app", "alice"),
        ("https://hub.example/use?id=missing", "alice"),
        ("https://hub.example/use?id=app&id=another", "alice"),
        ("https://hub.example/use?id=app&route=https%3A%2F%2Fother.example", "alice"),
        ("https://hub.example/use?id=app&route=%2F..%2Fsettings", "alice"),
        ("https://hub.example/use?id=app&eventId=run-event", "alice"),
    ] {
        let activity = NSUserActivity(activityType: NativeSystemIntegration.activityType)
        activity.userInfo = ["scope": scope]
        activity.webpageURL = URL(string: url)!
        #expect(!NativeActivityBridge.handle(activity, store: store))
        #expect(try store.takeActions().isEmpty)
    }
}

@Test func handoffRestoresAnExpiredReceiverCatalogWithoutExposingStaleWidgets() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = try handoffStore(directory)
    var cached = try #require(store.catalog())
    cached.expiresAt = "2000-01-01T00:00:00Z"
    try JSONEncoder().encode(cached).write(to: directory.appendingPathComponent("snapshot.json"))
    #expect(store.snapshot() == nil)
    let activity = NSUserActivity(activityType: NativeSystemIntegration.activityType)
    activity.userInfo = ["scope": "alice"]
    activity.webpageURL = URL(string: "https://hub.example/use?id=app&route=%2Freports")
    #expect(NativeActivityBridge.handle(activity, store: store))
    #expect(try store.pendingActions().first?.action.path == "/reports")
    try store.clear()
    #expect(!NativeActivityBridge.handle(activity, store: store))
}

@Test func publishedHandoffRequiresItsScopeToSurviveRestoration() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = try handoffStore(directory)
    var snapshot = try #require(store.catalog())
    snapshot.activePage = NativePage(title: "Reports", url: "https://hub.example/use?id=app&route=%2Freports")
    let activity = try #require(NativeSystemIntegration.handoffActivity(snapshot))
    #expect(activity.requiredUserInfoKeys == ["scope"])
    #expect(activity.userInfo?["scope"] as? String == "alice")
    #expect(activity.webpageURL?.absoluteString == snapshot.activePage?.url)
    #expect(activity.isEligibleForHandoff)
    #expect(!activity.isEligibleForSearch)
    snapshot.activePage?.url = "https://user@hub.example/use?id=app"
    #expect(NativeSystemIntegration.handoffActivity(snapshot) == nil)
}

private func handoffStore(_ directory: URL) throws -> NativeStore {
    let store = NativeStore(directory: directory)
    _ = try store.publish(JSONSerialization.data(withJSONObject: [
        "version": 1, "scope": "alice", "generatedAt": ISO8601DateFormatter().string(from: Date()),
        "expiresAt": ISO8601DateFormatter().string(from: Date().addingTimeInterval(3600)),
        "sections": [], "events": [], "apps": [["id": "app", "title": "Reports", "spotlightEligible": true]],
        "webOrigin": "https://hub.example",
    ]))
    return store
}

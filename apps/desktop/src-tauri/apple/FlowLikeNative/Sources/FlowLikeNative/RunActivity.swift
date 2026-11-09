import Foundation
#if os(iOS)
import ActivityKit

public struct FlowRunAttributes: ActivityAttributes {
    public struct ContentState: Codable, Hashable {
        public var title: String
        public var status: String
        public var progress: Double?
        public var updatedAt: Date
    }
    public var runId: String
    public var appId: String
    public var scope: String
}

@MainActor enum FlowRunActivities {
    private static let reconciler = NativeActivityReconciler(FlowRunActivityDriver())

    static func refresh() async { await reconciler.refresh() }
}

@MainActor private struct FlowRunActivityDriver: NativeActivityDriver {
    func activities() -> [ActivityKit.Activity<FlowRunAttributes>] { ActivityKit.Activity<FlowRunAttributes>.activities }

    func key(for activity: ActivityKit.Activity<FlowRunAttributes>) -> NativeActivityKey {
        let attributes = activity.attributes
        return NativeActivityKey(scope: attributes.scope, ownerId: attributes.appId, workId: attributes.runId)
    }

    func selection() -> NativeActivitySelection<NativeItem> {
        guard let snapshot = NativeStore.shared.snapshot() else { return .init(scope: nil, items: []) }
        // Carried sections repeat an unconfirmed status, so only live runs and this publish's reads count.
        let runs = snapshot.sections.first(where: { $0.kind == "recent_runs" })
        let active = runs?.items.compactMap { run -> (key: NativeActivityKey, item: NativeItem)? in
            guard ["running", "pending", "queued"].contains(run.status ?? ""),
                  run.live == true || runs?.updatedAt == nil,
                  let runId = run.action.runId, let appId = run.action.appId else { return nil }
            return (NativeActivityKey(scope: snapshot.scope, ownerId: appId, workId: runId), run)
        } ?? []
        return .init(scope: snapshot.scope, items: active)
    }

    func update(_ activity: ActivityKit.Activity<FlowRunAttributes>, item: NativeItem) async {
        await activity.update(content(item))
    }

    func end(_ activity: ActivityKit.Activity<FlowRunAttributes>) async {
        await activity.end(nil, dismissalPolicy: .immediate)
    }

    func start(_ item: NativeItem, key: NativeActivityKey) {
        guard ActivityAuthorizationInfo().areActivitiesEnabled,
              NativeStore.shared.snapshot()?.scope == key.scope else { return }
        _ = try? Activity.request(attributes: FlowRunAttributes(runId: key.workId, appId: key.ownerId, scope: key.scope),
                                  content: content(item), pushType: nil)
    }

    private func content(_ run: NativeItem) -> ActivityContent<FlowRunAttributes.ContentState> {
        ActivityContent(state: .init(title: run.title, status: run.status ?? "running",
                                    progress: run.progress.map { min(1, max(0, $0)) }, updatedAt: Date()),
                        staleDate: Date().addingTimeInterval(300))
    }
}

public struct FlowBotActivityAttributes: ActivityAttributes {
    public struct ContentState: Codable, Hashable {
        public var title: String
        public var status: String
        public var progress: Double?
        public var updatedAt: Double
    }
    public var botId: String
    public var turnId: String
    public var scope: String
    public var conversationId: String? = nil
}

@MainActor enum FlowBotActivities {
    private static let reconciler = NativeActivityReconciler(FlowBotActivityDriver())

    static func refresh() async { await reconciler.refresh() }
}

@MainActor private struct FlowBotActivityDriver: NativeActivityDriver {
    private static var observers: [String: Task<Void, Never>] = [:]
    private static var lifecycleObservers: [String: Task<Void, Never>] = [:]

    func activities() -> [ActivityKit.Activity<FlowBotActivityAttributes>] { ActivityKit.Activity<FlowBotActivityAttributes>.activities }

    func key(for activity: ActivityKit.Activity<FlowBotActivityAttributes>) -> NativeActivityKey {
        let attributes = activity.attributes
        return NativeActivityKey(scope: attributes.scope, ownerId: attributes.botId, workId: attributes.turnId)
    }

    func selection() -> NativeActivitySelection<NativeItem> {
        guard let snapshot = NativeStore.shared.snapshot() else {
            NativeBotActivityTokens.clear()
            return .init(scope: nil, items: [])
        }
        guard let section = snapshot.sections.first(where: { $0.kind == "bot_tasks" }), section.state == "ready",
              let freshness = snapshot.freshness(of: section), !freshness.isStale(at: Date()) else {
            return .init(scope: snapshot.scope, items: nil)
        }
        let active = section.items.compactMap { item -> (key: NativeActivityKey, item: NativeItem)? in
            guard item.live == true, ["queued", "running", "waiting"].contains(item.status ?? ""),
                  let botId = item.action.botId, let turnId = item.action.turnId else { return nil }
            return (NativeActivityKey(scope: snapshot.scope, ownerId: botId, workId: turnId), item)
        }
        return .init(scope: snapshot.scope, items: active)
    }

    func update(_ activity: ActivityKit.Activity<FlowBotActivityAttributes>, item: NativeItem) async {
        await activity.update(content(item))
    }

    func start(_ item: NativeItem, key: NativeActivityKey) {
        guard ActivityAuthorizationInfo().areActivitiesEnabled,
              let snapshot = NativeStore.shared.snapshot(), snapshot.scope == key.scope else { return }
        let local = snapshot.bots?.first(where: { $0.id == key.ownerId })?.local == true
        if let activity = try? Activity.request(attributes: FlowBotActivityAttributes(botId: key.ownerId, turnId: key.workId,
                scope: key.scope, conversationId: item.action.conversationId), content: content(item), pushType: local ? nil : .token) {
            observe(activity)
        }
    }

    private func content(_ item: NativeItem) -> ActivityContent<FlowBotActivityAttributes.ContentState> {
        let updatedAt = item.updatedAt.flatMap(NativeSnapshot.date) ?? item.startedAt.flatMap(NativeSnapshot.date) ?? Date()
        return ActivityContent(state: .init(title: item.title, status: item.status ?? "running",
            progress: item.progress.map { min(1, max(0, $0)) }, updatedAt: updatedAt.timeIntervalSince1970), staleDate: updatedAt.addingTimeInterval(300))
    }

    func observe(_ activity: ActivityKit.Activity<FlowBotActivityAttributes>) {
        guard Self.observers[activity.id] == nil else { return }
        if let token = activity.pushToken { NativeBotActivityTokens.record(activity, token: token) }
        Self.observers[activity.id] = Task {
            for await token in activity.pushTokenUpdates {
                guard !Task.isCancelled else { return }
                NativeBotActivityTokens.record(activity, token: token)
            }
        }
        Self.lifecycleObservers[activity.id] = Task {
            for await state in activity.activityStateUpdates {
                guard !Task.isCancelled else { return }
                if state == .ended || state == .dismissed {
                    Self.observers.removeValue(forKey: activity.id)?.cancel()
                    NativeBotActivityTokens.remove(activity.id)
                    Self.lifecycleObservers.removeValue(forKey: activity.id)
                    return
                }
            }
        }
    }

    func end(_ activity: ActivityKit.Activity<FlowBotActivityAttributes>) async {
        Self.observers.removeValue(forKey: activity.id)?.cancel()
        Self.lifecycleObservers.removeValue(forKey: activity.id)?.cancel()
        await activity.end(nil, dismissalPolicy: .immediate)
        NativeBotActivityTokens.remove(activity.id)
    }
}

private enum NativeBotActivityTokens {
    private static let lock = NSLock()
    private static var values: [String: [String: String]] = [:]

    static func record(_ activity: ActivityKit.Activity<FlowBotActivityAttributes>, token: Data) {
        let attributes = activity.attributes
        guard NativeStore.shared.persistedScope() == attributes.scope else { return }
        lock.lock()
        values[activity.id] = ["activityId": activity.id, "botId": attributes.botId, "turnId": attributes.turnId,
            "scope": attributes.scope, "pushToken": token.map { String(format: "%02x", $0) }.joined()]
        lock.unlock()
        NativeActionNotifier.post()
    }

    static func remove(_ id: String) {
        lock.lock()
        let removed = values.removeValue(forKey: id)
        lock.unlock()
        if removed != nil { NativeActionNotifier.post() }
    }

    static func clear() {
        lock.lock()
        let hadValues = !values.isEmpty
        values.removeAll()
        lock.unlock()
        if hadValues { NativeActionNotifier.post() }
    }

    static func json() -> String {
        lock.lock()
        let entries = Array(values.values)
        lock.unlock()
        return (try? JSONSerialization.data(withJSONObject: entries)).map { String(decoding: $0, as: UTF8.self) } ?? "[]"
    }
}
#endif

@_cdecl("flow_like_native_bot_activity_tokens")
public func flowLikeNativeBotActivityTokens() -> UnsafeMutablePointer<CChar>? {
    #if os(iOS)
    return strdup(NativeBotActivityTokens.json())
    #else
    return strdup("[]")
    #endif
}

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
    static func refresh(_ snapshot: NativeSnapshot?) async {
        let current = Activity<FlowRunAttributes>.activities
        guard let snapshot, snapshot.isCurrent else {
            for activity in current { await activity.end(nil, dismissalPolicy: .immediate) }
            return
        }
        // Carried sections repeat an unconfirmed status, so only live runs and this publish's reads count.
        let runs = snapshot.sections.first(where: { $0.kind == "recent_runs" })
        let active = runs?.items.filter {
            ["running", "pending", "queued"].contains($0.status ?? "") && ($0.live == true || runs?.updatedAt == nil)
        } ?? []
        for activity in current {
            guard activity.attributes.scope == snapshot.scope,
                  let run = active.first(where: { $0.action.runId == activity.attributes.runId }) else {
                await activity.end(nil, dismissalPolicy: .default)
                continue
            }
            await activity.update(content(run))
        }
        guard ActivityAuthorizationInfo().areActivitiesEnabled,
              NativeStore.shared.snapshot()?.scope == snapshot.scope else { return }
        for run in active.prefix(3) {
            guard let runId = run.action.runId, let appId = run.action.appId,
                  !current.contains(where: { $0.attributes.runId == runId && $0.attributes.scope == snapshot.scope }) else { continue }
            _ = try? Activity.request(attributes: FlowRunAttributes(runId: runId, appId: appId, scope: snapshot.scope),
                                      content: content(run), pushType: nil)
        }
    }

    private static func content(_ run: NativeItem) -> ActivityContent<FlowRunAttributes.ContentState> {
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
    private static var observers: [String: Task<Void, Never>] = [:]
    private static var lifecycleObservers: [String: Task<Void, Never>] = [:]

    static func refresh(_ snapshot: NativeSnapshot?) async {
        let current = Activity<FlowBotActivityAttributes>.activities
        guard let snapshot, snapshot.isCurrent else {
            for activity in current { await end(activity) }
            NativeBotActivityTokens.clear()
            return
        }
        for activity in current where activity.attributes.scope != snapshot.scope { await end(activity) }
        guard let section = snapshot.sections.first(where: { $0.kind == "bot_tasks" }), section.state == "ready",
              let freshness = snapshot.freshness(of: section), !freshness.isStale(at: Date()) else { return }
        let active = section.items.filter {
            $0.live == true && ["queued", "running", "waiting"].contains($0.status ?? "") && $0.action.turnId != nil && $0.action.botId != nil
        }
        for activity in current where activity.attributes.scope == snapshot.scope {
            observe(activity)
            guard let item = active.first(where: { $0.action.turnId == activity.attributes.turnId && $0.action.botId == activity.attributes.botId }) else {
                await end(activity)
                continue
            }
            await activity.update(content(item))
        }
        guard ActivityAuthorizationInfo().areActivitiesEnabled else { return }
        for item in active.prefix(3) {
            guard let botId = item.action.botId, let turnId = item.action.turnId,
                  !current.contains(where: { $0.attributes.scope == snapshot.scope && $0.attributes.turnId == turnId && $0.attributes.botId == botId }) else { continue }
            if let activity = try? Activity.request(attributes: FlowBotActivityAttributes(botId: botId, turnId: turnId, scope: snapshot.scope, conversationId: item.action.conversationId),
                                                    content: content(item), pushType: .token) {
                observe(activity)
            }
        }
    }

    private static func content(_ item: NativeItem) -> ActivityContent<FlowBotActivityAttributes.ContentState> {
        let updatedAt = item.updatedAt.flatMap(NativeSnapshot.date) ?? item.startedAt.flatMap(NativeSnapshot.date) ?? Date()
        return ActivityContent(state: .init(title: item.title, status: item.status ?? "running",
            progress: item.progress.map { min(1, max(0, $0)) }, updatedAt: updatedAt.timeIntervalSince1970), staleDate: updatedAt.addingTimeInterval(300))
    }

    private static func observe(_ activity: Activity<FlowBotActivityAttributes>) {
        guard observers[activity.id] == nil else { return }
        if let token = activity.pushToken { NativeBotActivityTokens.record(activity, token: token) }
        observers[activity.id] = Task {
            for await token in activity.pushTokenUpdates {
                guard !Task.isCancelled else { return }
                NativeBotActivityTokens.record(activity, token: token)
            }
        }
        lifecycleObservers[activity.id] = Task {
            for await state in activity.activityStateUpdates {
                guard !Task.isCancelled else { return }
                if state == .ended || state == .dismissed {
                    observers.removeValue(forKey: activity.id)?.cancel()
                    NativeBotActivityTokens.remove(activity.id)
                    lifecycleObservers.removeValue(forKey: activity.id)
                    return
                }
            }
        }
    }

    private static func end(_ activity: Activity<FlowBotActivityAttributes>) async {
        observers.removeValue(forKey: activity.id)?.cancel()
        lifecycleObservers.removeValue(forKey: activity.id)?.cancel()
        await activity.end(nil, dismissalPolicy: .default)
        NativeBotActivityTokens.remove(activity.id)
    }
}

private enum NativeBotActivityTokens {
    private static let lock = NSLock()
    private static var values: [String: [String: String]] = [:]

    static func record(_ activity: Activity<FlowBotActivityAttributes>, token: Data) {
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

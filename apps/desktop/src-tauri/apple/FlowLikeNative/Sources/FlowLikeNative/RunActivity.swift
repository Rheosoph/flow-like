#if os(iOS)
import ActivityKit
import Foundation

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
        let active = snapshot.sections.first(where: { $0.kind == "recent_runs" })?.items.filter {
            ["running", "pending", "queued"].contains($0.status ?? "")
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
#endif

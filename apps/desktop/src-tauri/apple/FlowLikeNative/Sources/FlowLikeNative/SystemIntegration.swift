import AppIntents
import CoreSpotlight
import Foundation
import UniformTypeIdentifiers
import WidgetKit

public enum NativeSystemIntegration {
    public static let activityType = "com.flow-like.app.page"
    @MainActor private static var handoff: NSUserActivity?
    @MainActor public static var refreshAppShortcuts: (() -> Void)?

    @MainActor public static func refresh() {
        // Read after hopping to the main actor so delayed refreshes cannot
        // restore a snapshot that was cleared or replaced in the meantime.
        let snapshot = NativeStore.shared.snapshot()
        WidgetCenter.shared.reloadAllTimelines()
        #if os(iOS)
        refreshAppShortcuts?()
        #else
        FlowLikeShortcuts.updateAppShortcutParameters()
        #endif
        Task { await NativeSpotlightIndexer.shared.replace(snapshot) }
        handoff?.invalidate()
        handoff = nil
        if let snapshot, let activity = handoffActivity(snapshot) {
            activity.becomeCurrent()
            handoff = activity
        }
        #if os(iOS)
        Task { await FlowRunActivities.refresh(snapshot) }
        #endif
    }

    static func handoffActivity(_ snapshot: NativeSnapshot) -> NSUserActivity? {
        guard snapshot.isCurrent, let page = snapshot.activePage, let url = URL(string: page.url),
              let parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              NativeActivityBridge.isUsePathname(parts.percentEncodedPath),
              url.user == nil, url.password == nil, url.fragment == nil,
              NativeActivityBridge.origin(url) == snapshot.webOrigin else { return nil }
        let activity = NSUserActivity(activityType: activityType)
        activity.title = page.title
        activity.webpageURL = url
        activity.userInfo = ["scope": snapshot.scope]
        activity.requiredUserInfoKeys = ["scope"]
        activity.isEligibleForHandoff = true
        activity.isEligibleForSearch = false
        return activity
    }

    public static func handleSearchActivity(_ activity: NSUserActivity, store: NativeStore = .shared) -> URL? {
        guard activity.activityType == CSSearchableItemActionType,
              let identifier = activity.userInfo?[CSSearchableItemActivityIdentifier] as? String else { return nil }
        let snapshot = store.catalog()
        let action: NativeAction
        if identifier == "flow-like:flowpilot" { action = NativeAction(kind: "flowpilot") }
        else if identifier.hasPrefix("flow-like:app:"), let app = snapshot?.apps.first(where: {
            $0.id == String(identifier.dropFirst("flow-like:app:".count)) && $0.spotlightEligible == true
        }) { action = NativeAction(kind: "open_app", appId: app.id) }
        else if identifier.hasPrefix("flow-like:event:"), let event = snapshot?.events.first(where: {
            $0.id == String(identifier.dropFirst("flow-like:event:".count)) && $0.surfaces.contains("spotlight")
        }) { action = event.action }
        else { return nil }
        guard let request = try? store.enqueue(action, expectedScope: snapshot?.scope) else { return nil }
        return NativeStore.actionURL(request)
    }
}

private actor NativeSpotlightIndexer {
    static let shared = NativeSpotlightIndexer()
    private let domain = "com.flow-like.app.events"
    private var pending: Task<Void, Never>?

    // Serialize complete delete/index operations. A later signout always removes
    // the previous account's items after any already-running index operation.
    func replace(_ snapshot: NativeSnapshot?) {
        let previous = pending
        pending = Task {
            await previous?.value
            let index = CSSearchableIndex.default()
            await withCheckedContinuation { continuation in
                index.deleteSearchableItems(withDomainIdentifiers: [domain]) { _ in continuation.resume() }
            }
            guard let snapshot, snapshot.isCurrent,
                  let latest = NativeStore.shared.snapshot(), latest.scope == snapshot.scope,
                  latest.generatedAt == snapshot.generatedAt else { return }
            func item(_ id: String, _ title: String, _ subtitle: String?, _ keywords: [String]) -> CSSearchableItem {
                let attributes = CSSearchableItemAttributeSet(contentType: .item)
                attributes.title = title
                attributes.contentDescription = subtitle
                attributes.keywords = keywords
                let item = CSSearchableItem(uniqueIdentifier: id, domainIdentifier: domain, attributeSet: attributes)
                item.expirationDate = NativeSnapshot.date(snapshot.expiresAt)
                return item
            }
            var items = [item("flow-like:flowpilot", "FlowPilot", "Ask a question in Flow Like", ["Flow Like", "FlowPilot", "assistant"])]
            items += snapshot.apps.filter { $0.spotlightEligible == true }.map {
                item("flow-like:app:\($0.id)", $0.title, "Open in Flow Like", ["Flow Like", "app"])
            }
            items += snapshot.events.filter { $0.surfaces.contains("spotlight") }.map {
                item("flow-like:event:\($0.id)", $0.title, $0.subtitle, ["Flow Like", "Event", $0.eventType])
            }
            await withCheckedContinuation { continuation in
                index.indexSearchableItems(items) { _ in continuation.resume() }
            }
        }
    }
}

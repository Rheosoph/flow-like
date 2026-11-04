import Foundation
#if os(iOS)
import BackgroundTasks
import UIKit
#endif

struct NativeBotRefreshPayload: Encodable {
    var scope: String
    var bots: [NativeBot]
    var actions: [NativeActionRequest]
}

struct NativeBotRefreshResult: Decodable {
    struct Failure: Decodable { var id: String; var error: String }
    struct Turn: Decodable {
        var id: String
        var bot_id: String
        var conversation_id: String
        var status: String
        var started_at: String?
        var updated_at: String?
    }
    var success: Bool
    var turns: [Turn]?
    var deliveredIds: [String]?
    var failedActions: [Failure]?
}

final class NativeBotRefreshLease: @unchecked Sendable {
    struct Entry {
        var id: String
        var scope: String
        var deadline: Date
        var actionIDs: Set<String>
    }
    private let lock = NSLock()
    private var entry: Entry?

    func begin(id: String, scope: String, actionIDs: Set<String>, now: Date = Date()) -> Bool {
        lock.lock(); defer { lock.unlock() }
        guard entry == nil else { return false }
        entry = Entry(id: id, scope: scope, deadline: now.addingTimeInterval(25), actionIDs: actionIDs)
        return true
    }

    func active(id: String, scope: String, persistedScope: String?, now: Date = Date()) -> Bool {
        lock.lock(); defer { lock.unlock() }
        guard let entry else { return false }
        return entry.id == id && entry.scope == scope && scope == persistedScope && entry.deadline > now
    }

    func ownedIDs(scope: String?, now: Date = Date()) -> Set<String> {
        lock.lock(); defer { lock.unlock() }
        guard let entry, entry.scope == scope, entry.deadline > now else { return [] }
        return entry.actionIDs
    }

    @discardableResult func end(id: String) -> Entry? {
        lock.lock(); defer { lock.unlock() }
        guard entry?.id == id else { return nil }
        defer { entry = nil }
        return entry
    }
}

enum NativeBotRefreshBridge {
    typealias Callback = @convention(c) (UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> Void
    static let lease = NativeBotRefreshLease()
    private static let lock = NSLock()
    private static var callback: Callback?
    private static var pending: (id: String, payload: String)?

    static func install(_ callback: Callback?) {
        lock.lock()
        self.callback = callback
        let pending = self.pending
        lock.unlock()
        if let callback, let pending { send(callback, id: pending.id, payload: pending.payload) }
    }

    static func dispatch(id: String, payload: String) {
        lock.lock()
        pending = (id, payload)
        let callback = callback
        lock.unlock()
        if let callback { send(callback, id: id, payload: payload) }
    }

    static func clear(id: String) {
        lock.lock(); defer { lock.unlock() }
        if pending?.id == id { pending = nil }
    }

    static func ownedIDs(scope: String?) -> Set<String> { lease.ownedIDs(scope: scope) }

    private static func send(_ callback: Callback, id: String, payload: String) {
        id.withCString { id in payload.withCString { callback(id, $0) } }
    }
}

#if os(iOS)
@available(iOSApplicationExtension, unavailable)
@MainActor public enum NativeBotBackgroundRefresh {
    public static let identifier = "com.flow-like.app.bot-refresh"
    private struct Running {
        var id: String
        var scope: String
        var actionIDs: Set<String>
        var completions: [(Bool, Bool) -> Void]
        var lifetime: UIBackgroundTaskIdentifier = .invalid
    }
    private static var installed = false
    private static var registered = false
    private static var observers: [NSObjectProtocol] = []
    private static var running: Running?

    /// Register before application launch completes, including launches without a webview.
    public static func install() {
        guard !installed else { return }
        installed = true
        registered = BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: .main) { task in
            MainActor.assumeIsolated {
                guard let task = task as? BGAppRefreshTask else { task.setTaskCompleted(success: false); return }
                schedule()
                let id = start(hasSystemTask: true) { success, _ in task.setTaskCompleted(success: success) }
                task.expirationHandler = {
                    Task { @MainActor in if let id { finish(id: id, result: nil) } }
                }
            }
        }
        observers.append(NotificationCenter.default.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated {
                schedule()
                _ = start(hasSystemTask: false) { _, _ in }
            }
        })
        observers.append(NotificationCenter.default.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { schedule() }
        })
        schedule()
    }

    public static func schedule() {
        guard registered else { return }
        BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: identifier)
        guard NativeStore.shared.persistedScope() != nil else { return }
        let request = BGAppRefreshTaskRequest(identifier: identifier)
        request.earliestBeginDate = Date().addingTimeInterval(15 * 60)
        try? BGTaskScheduler.shared.submit(request)
    }

    public static func performFetch(userInfo: [AnyHashable: Any], completion: @escaping (UIBackgroundFetchResult) -> Void) {
        guard let snapshot = NativeStore.shared.catalog(),
              userInfo["native_scope"] as? String == snapshot.scope,
              userInfo["user_id"] as? String == snapshot.userId else { completion(.noData); return }
        schedule()
        _ = start(hasSystemTask: true) { success, changed in
            completion(success ? (changed ? .newData : .noData) : .failed)
        }
    }

    @discardableResult private static func start(hasSystemTask: Bool, completion: @escaping (Bool, Bool) -> Void) -> String? {
        if let current = running {
            if NativeBotRefreshBridge.lease.active(id: current.id, scope: current.scope, persistedScope: NativeStore.shared.persistedScope()) {
                running?.completions.append(completion)
                return current.id
            }
            finish(id: current.id, result: nil)
        }
        guard let payload = try? NativeStore.shared.botRefreshPayload(includeActions: UIApplication.shared.applicationState != .active),
              let data = try? JSONEncoder().encode(payload), data.count <= 2_097_152 else { completion(false, false); return nil }
        let id = UUID().uuidString
        let actionIDs = Set(payload.actions.map(\.id))
        guard NativeBotRefreshBridge.lease.begin(id: id, scope: payload.scope, actionIDs: actionIDs) else { completion(false, false); return nil }
        running = Running(id: id, scope: payload.scope, actionIDs: actionIDs, completions: [completion])
        if !hasSystemTask {
            let lifetime = UIApplication.shared.beginBackgroundTask(withName: "Update bots") {
                Task { @MainActor in finish(id: id, result: nil) }
            }
            running?.lifetime = lifetime
            guard lifetime != .invalid else { finish(id: id, result: nil); return nil }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 25) { finish(id: id, result: nil) }
        NativeBotRefreshBridge.dispatch(id: id, payload: String(decoding: data, as: UTF8.self))
        return id
    }

    static func finish(id: String, result: NativeBotRefreshResult?) {
        guard let request = running, request.id == id else { return }
        var success = false
        var changed = false
        if let result, NativeBotRefreshBridge.lease.active(id: id, scope: request.scope, persistedScope: NativeStore.shared.persistedScope()) {
            do {
                try NativeStore.shared.applyBotRefresh(result, scope: request.scope, allowedIDs: request.actionIDs)
                success = result.success
                changed = result.turns != nil || !(result.deliveredIds ?? []).isEmpty
            } catch { success = false }
        }
        NativeBotRefreshBridge.lease.end(id: id)
        NativeBotRefreshBridge.clear(id: id)
        running = nil
        if request.lifetime != .invalid { UIApplication.shared.endBackgroundTask(request.lifetime) }
        if changed { NativeSystemIntegration.refreshBotStatus() }
        NativeActionNotifier.post()
        for completion in request.completions { completion(success, changed) }
    }
}
#endif

@_cdecl("flow_like_native_set_bot_refresh_callback")
public func flowLikeNativeSetBotRefreshCallback(_ callback: (@convention(c) (UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> Void)?) {
    NativeBotRefreshBridge.install(callback)
}

@_cdecl("flow_like_native_bot_refresh_active")
public func flowLikeNativeBotRefreshActive(_ id: UnsafePointer<CChar>?, _ scope: UnsafePointer<CChar>?) -> Int32 {
    guard let id, let scope else { return 0 }
    return NativeBotRefreshBridge.lease.active(id: String(cString: id), scope: String(cString: scope),
        persistedScope: NativeStore.shared.persistedScope()) ? 1 : 0
}

@available(iOSApplicationExtension, unavailable)
@_cdecl("flow_like_native_complete_bot_refresh")
public func flowLikeNativeCompleteBotRefresh(_ id: UnsafePointer<CChar>?, _ json: UnsafePointer<CChar>?) {
    guard let id, let json else { return }
    let requestId = String(cString: id)
    let data = Data(String(cString: json).utf8)
    let result = data.count <= 2_097_152 ? try? JSONDecoder().decode(NativeBotRefreshResult.self, from: data) : nil
    #if os(iOS)
    Task { @MainActor in NativeBotBackgroundRefresh.finish(id: requestId, result: result) }
    #else
    _ = (requestId, result)
    #endif
}

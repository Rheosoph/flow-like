import Foundation
import Darwin
#if os(iOS)
import UIKit
#endif

public enum NativeIntegrationError: LocalizedError, Equatable {
    case unavailable, expired, invalidAction, invalidRoute, oversized

    public var errorDescription: String? {
        switch self {
        case .unavailable: return "Open Flow Like to connect your workspace."
        case .expired: return "Open Flow Like to refresh your workspace."
        case .invalidAction: return "This action is no longer available. Open Flow Like to choose an app or Event."
        case .invalidRoute: return "Choose a path inside this app and add matching query names and values. External URLs, fragments, and parent paths are not supported."
        case .oversized: return "This content is too large to share with Flow Like."
        }
    }
}

public struct NativeStore: Sendable {
    public static let appGroup = "group.com.flow-like.app"
    public static let shared = NativeStore()
    private let directory: URL?

    public init(directory: URL? = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup)) {
        self.directory = directory
    }

    private func root() throws -> URL {
        guard let directory else { throw NativeIntegrationError.unavailable }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        #if os(iOS)
        try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
                                             ofItemAtPath: directory.path)
        #endif
        return directory
    }

    public func snapshot() -> NativeSnapshot? {
        guard let snapshot = catalog(), snapshot.isCurrent else { return nil }
        return snapshot
    }

    // Saved entry points can wake the app after widget data expires. The app
    // rechecks the current account, route, and Event before accepting an action.
    public func catalog() -> NativeSnapshot? {
        guard let directory,
              let data = try? Data(contentsOf: directory.appendingPathComponent("snapshot.json")),
              data.count <= 2_097_152,
              let snapshot = try? JSONDecoder().decode(NativeSnapshot.self, from: data),
              snapshot.version == 1, !snapshot.scope.isEmpty else { return nil }
        return snapshot
    }

    public func persistedScope() -> String? {
        catalog()?.scope
    }

    @discardableResult public func publish(_ data: Data) throws -> NativeSnapshot {
        guard data.count <= 2_097_152 else { throw NativeIntegrationError.oversized }
        let incoming = try JSONDecoder().decode(NativeSnapshot.self, from: data)
        guard incoming.isCurrent else { throw NativeIntegrationError.expired }
        return try withLock { directory in
            let previous = self.catalog()
            var snapshot = incoming.retainingLastGood(from: previous)
            snapshot.customWidgets = NativeCustomWidget.normalize(snapshot.customWidgets, apps: snapshot.apps)
            for section in snapshot.sections.indices {
                for item in snapshot.sections[section].items.indices {
                    snapshot.sections[section].items[item].icon = snapshot.sections[section].items[item].icon?.normalized()
                }
            }
            let sanitized = try JSONEncoder().encode(snapshot)
            guard sanitized.count <= 2_097_152 else { throw NativeIntegrationError.oversized }
            if previous?.scope != snapshot.scope {
                try? FileManager.default.removeItem(at: directory.appendingPathComponent("actions.json"))
                try? FileManager.default.removeItem(at: directory.appendingPathComponent("action-results.json"))
                try NativeAppIconStore(directory: directory).clear()
                try NativeGeofenceStore(directory: directory).clear()
            }
            try sanitized.write(to: directory.appendingPathComponent("snapshot.json"), options: .atomic)
            try NativeAppIconStore(directory: directory).prune(scope: snapshot.scope, appIds: NativeAppIconStore.catalogAppIds(snapshot))
            #if os(iOS)
            try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
                                                 ofItemAtPath: directory.appendingPathComponent("snapshot.json").path)
            #endif
            return snapshot
        }
    }

    public func clear() throws {
        try withLock { directory in
            try NativeGeofenceStore(directory: directory).clear()
            try NativeAppIconStore(directory: directory).clear()
            for file in ["snapshot.json", "actions.json", "action-results.json", "SharedFiles"] {
                let path = directory.appendingPathComponent(file)
                if FileManager.default.fileExists(atPath: path.path) {
                    try FileManager.default.removeItem(at: path)
                }
            }
        }
    }

    public func enqueue(_ action: NativeAction, expectedScope: String? = nil,
                        responseMode: String? = nil, responseTimeout: TimeInterval = 90,
                        executionMode: String? = nil) throws -> NativeActionRequest {
        guard responseMode == nil || (["text", "result"].contains(responseMode!) && responseTimeout > 0 && responseTimeout <= 90) else {
            throw NativeIntegrationError.invalidAction
        }
        guard executionMode == nil || executionMode == "foreground" || executionMode == "background" else {
            throw NativeIntegrationError.invalidAction
        }
        if executionMode == "background" {
            guard action.kind == "flowpilot", responseMode == "text", responseTimeout <= 25,
                  action.prompt?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false,
                  action.voice != true, action.files?.isEmpty != false else {
                throw NativeIntegrationError.invalidAction
            }
        }
        let request = try withLock { directory in
            let allowed = ["open_home", "open_inbox", "open_app", "open_event", "run_event", "open_run", "flowpilot", "share"]
            guard allowed.contains(action.kind) else { throw NativeIntegrationError.invalidAction }
            let snapshot = catalog()
            guard let scope = snapshot?.scope else {
                throw NativeIntegrationError.expired
            }
            guard expectedScope == nil || expectedScope == scope else { throw NativeIntegrationError.invalidAction }
            if action.kind == "open_app" {
                guard let appId = action.appId, snapshot?.apps.contains(where: { $0.id == appId }) == true else {
                    throw NativeIntegrationError.invalidAction
                }
                try NativeAppRoute.validate(path: action.path, queryParams: action.queryParams)
            } else if action.path != nil || action.queryParams != nil {
                throw NativeIntegrationError.invalidAction
            }
            if action.kind == "run_event" || action.kind == "open_event" {
                guard snapshot?.events.contains(where: {
                    $0.appId == action.appId && $0.eventId == action.eventId && !$0.surfaces.isEmpty
                        && $0.action.kind == action.kind && $0.action.operation == action.operation
                }) == true else { throw NativeIntegrationError.invalidAction }
            }
            let formatter = ISO8601DateFormatter()
            formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            let now = Date()
            let request = NativeActionRequest(id: UUID().uuidString, scope: scope, action: action,
                                              createdAt: formatter.string(from: now), responseMode: responseMode,
                                              responseDeadline: responseMode.map { _ in formatter.string(from: now.addingTimeInterval(responseTimeout)) },
                                              executionMode: executionMode)
            var actions = readActions(in: directory).filter { $0.scope == scope && $0.isCurrent }
            actions.append(request)
            guard actions.count <= 64 else { throw NativeIntegrationError.oversized }
            let data = try JSONEncoder().encode(actions)
            guard data.count <= 2_097_152 else { throw NativeIntegrationError.oversized }
            if responseMode != nil { try NativeActionResults(directory: directory).register(request) }
            do { try data.write(to: directory.appendingPathComponent("actions.json"), options: .atomic) }
            catch {
                if responseMode != nil { try? NativeActionResults(directory: directory).remove(id: request.id, scope: scope) }
                throw error
            }
            #if os(iOS)
            try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
                                                 ofItemAtPath: directory.appendingPathComponent("actions.json").path)
            #endif
            return request
        }
        NativeActionNotifier.post()
        return request
    }

    public func takeActions() throws -> [NativeActionRequest] {
        try withLock { directory in
            let scope = persistedScope()
            let actions = readActions(in: directory).filter { $0.scope == scope && $0.isCurrent }
            try JSONEncoder().encode(actions.filter(\.isBackground)).write(to: directory.appendingPathComponent("actions.json"), options: .atomic)
            return actions.filter { !$0.isBackground }
        }
    }

    public func pendingActions() throws -> [NativeActionRequest] {
        try withLock { directory in
            let scope = persistedScope()
            let stored = readActions(in: directory)
            let actions = stored.filter { $0.scope == scope && $0.isCurrent }
            if stored.count != actions.count {
                try JSONEncoder().encode(actions).write(to: directory.appendingPathComponent("actions.json"), options: .atomic)
            }
            return actions.filter { !$0.isBackground }
        }
    }

    // Claiming removes work before execution. A crashed process must not replay tool side effects.
    public func claimBackgroundActions() throws -> [NativeActionRequest] {
        try withLock { directory in
            let scope = persistedScope()
            let actions = readActions(in: directory).filter { $0.scope == scope && $0.isCurrent }
            let results = NativeActionResults(directory: directory)
            let claimed = try actions.filter { request in
                guard request.isBackground else { return false }
                return try results.backgroundActionActive(id: request.id, scope: request.scope)
            }
            try JSONEncoder().encode(actions.filter { !$0.isBackground }).write(to: directory.appendingPathComponent("actions.json"), options: .atomic)
            return claimed
        }
    }

    public func backgroundActionActive(id: String, scope: String) throws -> Bool {
        try withLock { directory in
            guard persistedScope() == scope else { return false }
            return try NativeActionResults(directory: directory).backgroundActionActive(id: id, scope: scope)
        }
    }

    public func acknowledgeAction(id: String, scope: String) throws {
        guard !id.isEmpty, id.utf8.count <= 128, !scope.isEmpty else { throw NativeIntegrationError.invalidAction }
        try withLock { directory in
            guard persistedScope() == scope else { return }
            let actions = readActions(in: directory).filter { $0.id != id || $0.scope != scope }
            try JSONEncoder().encode(actions).write(to: directory.appendingPathComponent("actions.json"), options: .atomic)
        }
    }

    public func completeAction(_ result: NativeActionResult) throws {
        try withLock { directory in
            guard persistedScope() == result.scope else { throw NativeIntegrationError.invalidAction }
            try NativeActionResults(directory: directory).complete(result)
        }
    }

    public func waitForActionResult(_ request: NativeActionRequest) async throws -> NativeActionResult {
        defer { try? cancelActionResponse(request) }
        while true {
            try Task.checkCancellation()
            guard request.isCurrent else {
                throw NativeActionFailure(message: request.isBackground
                    ? "FlowPilot reached the background action time limit. Cancellation was requested."
                    : "Flow Like did not return a result before the action deadline. A run already started may still continue in the app.")
            }
            let result = try withLock { directory in
                guard persistedScope() == request.scope else { throw NativeIntegrationError.invalidAction }
                return try NativeActionResults(directory: directory).result(id: request.id, scope: request.scope)
            }
            if let result { return try result.requireSuccess() }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
    }

    public func cancelActionResponse(_ request: NativeActionRequest) throws {
        defer { NativeActionNotifier.post() }
        try withLock { directory in
            guard persistedScope() == request.scope else { return }
            let actions = readActions(in: directory).filter { $0.id != request.id || $0.scope != request.scope }
            try JSONEncoder().encode(actions).write(to: directory.appendingPathComponent("actions.json"), options: .atomic)
            try NativeActionResults(directory: directory).remove(id: request.id, scope: request.scope)
        }
    }

    @available(iOSApplicationExtension, unavailable)
    @available(macOSApplicationExtension, unavailable)
    public func waitForBackgroundActionResult(_ request: NativeActionRequest) async throws -> NativeActionResult {
        #if os(iOS)
        let lifetime = await NativeActionBackgroundLifetime(store: self, request: request)
        do {
            let result = try await waitForActionResult(request)
            await lifetime.end()
            return result
        } catch {
            await lifetime.end()
            throw error
        }
        #else
        return try await waitForActionResult(request)
        #endif
    }

    public func publishAppIcons(_ data: Data) throws {
        try withLock { directory in
            guard let snapshot = catalog() else { throw NativeIntegrationError.expired }
            try NativeAppIconStore(directory: directory).publish(data, catalog: snapshot)
        }
    }

    public func appIconData(scope: String, appId: String) -> Data? {
        try? withLock { directory in
            NativeAppIconStore(directory: directory).data(scope: scope, appId: appId, catalog: catalog())
        }
    }

    public func appIconData(scope: String, eventId: String) -> Data? {
        try? withLock { directory in
            guard let snapshot = catalog(), snapshot.scope == scope,
                  let event = snapshot.events.first(where: { $0.id == eventId }) else { return nil }
            return NativeAppIconStore(directory: directory).data(scope: scope, appId: event.appId, catalog: snapshot)
        }
    }

    public func importSharedFile(_ file: URL) throws -> URL {
        let resources = try file.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey, .isSymbolicLinkKey])
        guard resources.isRegularFile == true, resources.isSymbolicLink != true,
              let size = resources.fileSize, size <= 20_971_520 else {
            throw NativeIntegrationError.oversized
        }
        return try withLock { directory in
            let folder = directory.appendingPathComponent("SharedFiles", isDirectory: true)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try pruneSharedFiles(in: directory, reservingBytes: size, reservingFiles: 1)
            let target = folder.appendingPathComponent(UUID().uuidString + "-" + file.lastPathComponent)
            do {
                try FileManager.default.copyItem(at: file, to: target)
                // Imported files can outlive the queue while the foreground app reads them.
                try FileManager.default.setAttributes([.modificationDate: Date()], ofItemAtPath: target.path)
                let copiedSize = try target.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? Int.max
                guard copiedSize <= 20_971_520 else { throw NativeIntegrationError.oversized }
                try pruneSharedFiles(in: directory, reservingBytes: 0, reservingFiles: 0)
                return target
            } catch {
                try? FileManager.default.removeItem(at: target)
                throw error
            }
        }
    }

    public func sharedFilesDirectory() throws -> URL {
        let folder = try root().appendingPathComponent("SharedFiles", isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        return folder
    }

    public static func actionURL(_ request: NativeActionRequest) -> URL {
        var parts = URLComponents()
        parts.scheme = "flow-like"
        parts.host = "native"
        parts.path = "/action"
        parts.queryItems = [URLQueryItem(name: "id", value: request.id)]
        return parts.url!
    }

    private func readActions(in directory: URL) -> [NativeActionRequest] {
        guard let data = try? Data(contentsOf: directory.appendingPathComponent("actions.json")), data.count <= 2_097_152 else { return [] }
        return (try? JSONDecoder().decode([NativeActionRequest].self, from: data)) ?? []
    }

    private func pruneSharedFiles(in directory: URL, reservingBytes: Int, reservingFiles: Int) throws {
        let folder = directory.appendingPathComponent("SharedFiles", isDirectory: true)
        let protected = Set(readActions(in: directory).filter(\.isCurrent).flatMap { $0.action.files ?? [] }
            .map { URL(fileURLWithPath: $0).resolvingSymlinksInPath().path })
        let now = Date()
        var retained: [(url: URL, size: Int, modified: Date, protected: Bool)] = []
        for file in try FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: [
            .isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey, .contentModificationDateKey,
        ]) {
            let name = file.lastPathComponent
            guard name.count > 37, UUID(uuidString: String(name.prefix(36))) != nil,
                  name.dropFirst(36).first == "-" else { continue }
            let values = try file.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey, .contentModificationDateKey])
            guard values.isRegularFile == true, values.isSymbolicLink != true else { continue }
            let modified = values.contentModificationDate ?? .distantPast
            let queued = protected.contains(file.resolvingSymlinksInPath().path)
            if !queued && now.timeIntervalSince(modified) > 86_400 {
                try FileManager.default.removeItem(at: file)
            } else {
                retained.append((file, values.fileSize ?? 0, modified, queued || now.timeIntervalSince(modified) < 300))
            }
        }
        var bytes = retained.reduce(reservingBytes) { $0 + $1.size }
        var count = retained.count + reservingFiles
        for file in retained.sorted(by: { $0.modified < $1.modified }) where !file.protected {
            if bytes <= 134_217_728 && count <= 64 { break }
            try FileManager.default.removeItem(at: file.url)
            bytes -= file.size
            count -= 1
        }
        guard bytes <= 134_217_728, count <= 64 else { throw NativeIntegrationError.oversized }
    }

    private func withLock<T>(_ body: (URL) throws -> T) throws -> T {
        let directory = try root()
        let descriptor = open(directory.appendingPathComponent("native.lock").path, O_CREAT | O_RDWR, 0o600)
        guard descriptor >= 0 else { throw NativeIntegrationError.unavailable }
        defer { close(descriptor) }
        #if os(iOS)
        try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
                                             ofItemAtPath: directory.appendingPathComponent("native.lock").path)
        #endif
        guard flock(descriptor, LOCK_EX) == 0 else { throw NativeIntegrationError.unavailable }
        defer { flock(descriptor, LOCK_UN) }
        return try body(directory)
    }
}

#if os(iOS)
@available(iOSApplicationExtension, unavailable)
@MainActor private final class NativeActionBackgroundLifetime {
    private var identifier: UIBackgroundTaskIdentifier = .invalid
    private let store: NativeStore
    private let request: NativeActionRequest

    init(store: NativeStore, request: NativeActionRequest) {
        self.store = store
        self.request = request
        identifier = UIApplication.shared.beginBackgroundTask(withName: "FlowPilot Action") { [weak self] in
            MainActor.assumeIsolated { self?.expire() }
        }
        if identifier == .invalid { expire() }
    }

    private func expire() {
        // A completion that won the race must remain available to the waiting intent.
        try? store.completeAction(NativeActionResult(id: request.id, scope: request.scope, status: "error",
            error: "iOS ended the background execution time. Cancellation was requested."))
        NativeActionNotifier.post()
        end()
    }

    func end() {
        guard identifier != .invalid else { return }
        UIApplication.shared.endBackgroundTask(identifier)
        identifier = .invalid
    }
}
#endif

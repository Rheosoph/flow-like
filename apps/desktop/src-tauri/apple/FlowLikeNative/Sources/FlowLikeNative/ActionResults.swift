import Foundation

public struct NativeActionResult: Codable, Sendable, Equatable {
    public var id: String
    public var scope: String
    public var status: String
    public var text: String?
    public var json: String?
    public var error: String?

    public func requireSuccess() throws -> NativeActionResult {
        guard status == "success" else {
            throw NativeActionFailure(message: error ?? (status == "interaction_required"
                ? "Continue in Flow Like to provide the required input."
                : "Flow Like could not finish this request."))
        }
        return self
    }
}

public struct NativeActionFailure: LocalizedError {
    public let message: String
    public var errorDescription: String? { message }
}

// Access only while NativeStore holds the shared process lock. Delivery ACKs
// do not remove these registrations: accepted work may finish afterward.
struct NativeActionResults {
    struct Entry: Codable {
        var request: NativeActionRequest
        var result: NativeActionResult?
    }
    let directory: URL
    private var file: URL { directory.appendingPathComponent("action-results.json") }

    private func read(scope: String) throws -> [Entry] {
        guard FileManager.default.fileExists(atPath: file.path) else { return [] }
        let data = try Data(contentsOf: file)
        guard data.count <= 4_194_304 else { throw NativeIntegrationError.oversized }
        return try JSONDecoder().decode([Entry].self, from: data)
            .filter { $0.request.scope == scope && $0.request.isCurrent }
    }

    private func write(_ entries: [Entry]) throws {
        let data = try JSONEncoder().encode(entries)
        guard entries.count <= 64, data.count <= 4_194_304 else { throw NativeIntegrationError.oversized }
        try data.write(to: file, options: .atomic)
        #if os(iOS)
        try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: file.path)
        #endif
    }

    func register(_ request: NativeActionRequest) throws {
        var entries = try read(scope: request.scope)
        entries.append(Entry(request: request))
        try write(entries)
    }

    func complete(_ result: NativeActionResult) throws {
        guard ["success", "error", "interaction_required"].contains(result.status) else { throw NativeIntegrationError.invalidAction }
        guard (result.text?.utf8.count ?? 0) <= 196_608,
              (result.json?.utf8.count ?? 0) <= 196_608,
              (result.error?.utf8.count ?? 0) <= 4096 else { throw NativeIntegrationError.oversized }
        guard try JSONEncoder().encode(result).count <= 262_144 else { throw NativeIntegrationError.oversized }
        if let json = result.json {
            guard (try? JSONSerialization.jsonObject(with: Data(json.utf8), options: [.fragmentsAllowed])) != nil else {
                throw NativeIntegrationError.invalidAction
            }
        }
        var entries = try read(scope: result.scope)
        guard let index = entries.firstIndex(where: { $0.request.id == result.id }) else { throw NativeIntegrationError.expired }
        if result.status == "success" {
            guard result.text != nil || result.json != nil,
                  entries[index].request.responseMode != "text" || result.text != nil else { throw NativeIntegrationError.invalidAction }
        }
        if let previous = entries[index].result {
            guard previous == result else { throw NativeIntegrationError.invalidAction }
            return
        }
        entries[index].result = result
        try write(entries)
    }

    func result(id: String, scope: String) throws -> NativeActionResult? {
        try read(scope: scope).first(where: { $0.request.id == id })?.result
    }

    func remove(id: String, scope: String) throws {
        try write(read(scope: scope).filter { $0.request.id != id })
    }
}

@_cdecl("flow_like_native_complete_action")
public func flowLikeNativeCompleteAction(_ json: UnsafePointer<CChar>?) -> Int32 {
    guard let json else { return -1 }
    do {
        let data = Data(String(cString: json).utf8)
        guard data.count <= 262_144 else { return -2 }
        try NativeStore.shared.completeAction(JSONDecoder().decode(NativeActionResult.self, from: data))
        return 0
    } catch { return -2 }
}

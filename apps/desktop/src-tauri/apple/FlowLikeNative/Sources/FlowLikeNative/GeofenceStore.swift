import CoreLocation
import CryptoKit
import Darwin
import Foundation

struct NativeGeofenceRegistration: Codable, Equatable {
    let id: String
    let appId: String
    let eventId: String
    let latitude: Double
    let longitude: Double
    let radiusMeters: Double
    let transition: String
    var background: Bool = false

    enum CodingKeys: CodingKey { case id, appId, eventId, latitude, longitude, radiusMeters, transition, background }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        appId = try values.decode(String.self, forKey: .appId)
        eventId = try values.decode(String.self, forKey: .eventId)
        latitude = try values.decode(Double.self, forKey: .latitude)
        longitude = try values.decode(Double.self, forKey: .longitude)
        radiusMeters = try values.decode(Double.self, forKey: .radiusMeters)
        transition = try values.decode(String.self, forKey: .transition)
        background = try values.decodeIfPresent(Bool.self, forKey: .background) ?? false
    }

    func systemID(scope: String) -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        let data = (try? encoder.encode(self)) ?? Data()
        let digest = SHA256.hash(data: Data(scope.utf8) + Data([0]) + data)
        return "flow-like-geo-" + digest.map { String(format: "%02x", $0) }.joined()
    }
}

struct NativeGeofenceConfiguration: Codable {
    let scope: String
    let registrations: [NativeGeofenceRegistration]

    func validate(currentScope: String?) throws {
        guard !scope.isEmpty, scope == currentScope else {
            throw NativeLocationFailure("scope_mismatch", "Refresh the active workspace before enabling location Events.")
        }
        guard registrations.count <= 20 else {
            throw NativeLocationFailure("region_limit", "Apple devices support at most 20 monitored regions per app.")
        }
        guard Set(registrations.map(\.id)).count == registrations.count else {
            throw NativeLocationFailure("invalid_input", "Geofence identifiers must be unique.")
        }
        for region in registrations {
            guard [region.id, region.appId, region.eventId].allSatisfy({ !$0.isEmpty && $0.utf8.count <= 512 }),
                  CLLocationCoordinate2DIsValid(CLLocationCoordinate2D(latitude: region.latitude, longitude: region.longitude)),
                  region.radiusMeters.isFinite, (100...100_000).contains(region.radiusMeters),
                  ["enter", "exit", "both"].contains(region.transition) else {
                throw NativeLocationFailure("invalid_input", "A geofence requires a valid center, positive radius, and enter or exit transition.")
            }
        }
    }
}

struct NativeGeofenceEvent: Codable {
    struct Geometry: Codable {
        let type: String
        let coordinates: [Double]
        init(coordinates: [Double]) { type = "Point"; self.coordinates = coordinates }
    }
    let id: String
    let registrationId: String
    let scope: String
    let appId: String
    let eventId: String
    let transition: String
    let occurredAt: Int64
    let geometry: Geometry
    let radiusMeters: Double
}

struct NativeGeofenceStore {
    static let shared = NativeGeofenceStore()
    private let directory: URL?
    private struct Queued: Codable { let systemID: String; let event: NativeGeofenceEvent }
    private struct State: Codable {
        var configuration: NativeGeofenceConfiguration
        var events: [Queued] = []
        var lastTransitions: [String: String] = [:]
    }
    init(directory: URL? = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: NativeStore.appGroup)) {
        self.directory = directory
    }

    func configuration() -> NativeGeofenceConfiguration? { try? locked { read(in: $0)?.configuration } }

    func replace(_ configuration: NativeGeofenceConfiguration, currentScope: String?) throws {
        try configuration.validate(currentScope: currentScope)
        try locked { directory in
            var state = read(in: directory) ?? State(configuration: configuration)
            if state.configuration.scope != configuration.scope { state = State(configuration: configuration) }
            state.configuration = configuration
            let valid = Set(configuration.registrations.map { $0.systemID(scope: configuration.scope) })
            state.events = current(state.events).filter { valid.contains($0.systemID) }
            state.lastTransitions = state.lastTransitions.filter { valid.contains($0.key) }
            try write(state, in: directory)
        }
    }

    func append(systemID: String, transition: String, currentScope: String?, foreground: Bool, now: Date = Date()) throws -> NativeGeofenceEvent? {
        try locked { directory in
            guard var state = read(in: directory), state.configuration.scope == currentScope,
                  let region = state.configuration.registrations.first(where: { $0.systemID(scope: state.configuration.scope) == systemID }),
                  foreground || region.background, ["enter", "exit"].contains(transition) else { return nil }
            let previous = state.lastTransitions[systemID]
            state.lastTransitions[systemID] = transition
            state.events = current(state.events, now: now)
            guard previous != transition, region.transition == "both" || region.transition == transition else {
                try write(state, in: directory)
                return nil
            }
            // CLLocationManager reports the crossing when delivered, without an original fix timestamp.
            let event = NativeGeofenceEvent(id: UUID().uuidString, registrationId: region.id, scope: state.configuration.scope,
                appId: region.appId, eventId: region.eventId, transition: transition, occurredAt: Int64(now.timeIntervalSince1970 * 1000),
                geometry: .init(coordinates: [region.longitude, region.latitude]), radiusMeters: region.radiusMeters)
            state.events.append(Queued(systemID: systemID, event: event))
            state.events = Array(state.events.suffix(128))
            try write(state, in: directory)
            return event
        }
    }

    func events(currentScope: String?, now: Date = Date()) throws -> [NativeGeofenceEvent] {
        try locked { directory in
            guard var state = read(in: directory), state.configuration.scope == currentScope else { return [] }
            state.events = current(state.events, now: now)
            try write(state, in: directory)
            return state.events.map(\.event)
        }
    }

    func acknowledge(_ ids: Set<String>) throws {
        try locked { directory in
            guard var state = read(in: directory) else { return }
            state.events = current(state.events).filter { !ids.contains($0.event.id) }
            try write(state, in: directory)
        }
    }

    func resetTransitions(_ systemIDs: Set<String>) throws {
        try locked { directory in
            guard var state = read(in: directory) else { return }
            state.lastTransitions = state.lastTransitions.filter { !systemIDs.contains($0.key) }
            try write(state, in: directory)
        }
    }

    func clear() throws {
        try locked { directory in
            let file = directory.appendingPathComponent("geofences.json")
            if FileManager.default.fileExists(atPath: file.path) { try FileManager.default.removeItem(at: file) }
        }
    }

    private func current(_ events: [Queued], now: Date = Date()) -> [Queued] {
        let timestamp = Int64(now.timeIntervalSince1970 * 1000)
        return events.filter { $0.event.occurredAt <= timestamp + 1000 && $0.event.occurredAt > timestamp - 3_600_000 }
    }
    private func read(in directory: URL) -> State? {
        guard let data = try? Data(contentsOf: directory.appendingPathComponent("geofences.json")), data.count <= 1_048_576 else { return nil }
        return try? JSONDecoder().decode(State.self, from: data)
    }
    private func write(_ state: State, in directory: URL) throws {
        let data = try JSONEncoder().encode(state)
        guard data.count <= 1_048_576 else { throw NativeLocationFailure("region_limit", "Too many pending location Events.") }
        let file = directory.appendingPathComponent("geofences.json")
        try data.write(to: file, options: .atomic)
        #if os(iOS)
        try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: file.path)
        #endif
    }
    private func locked<T>(_ body: (URL) throws -> T) throws -> T {
        guard let directory else { throw NativeLocationFailure("unavailable", "Shared location Event storage is unavailable.") }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let file = open(directory.appendingPathComponent("geofences.lock").path, O_CREAT | O_RDWR, 0o600)
        guard file >= 0 else { throw NativeLocationFailure("unavailable", "Location Event storage is unavailable.") }
        defer { close(file) }
        guard flock(file, LOCK_EX) == 0 else { throw NativeLocationFailure("unavailable", "Location Event storage is busy.") }
        defer { flock(file, LOCK_UN) }
        return try body(directory)
    }
}

import Foundation
import Testing
@testable import FlowLikeNative

private func geofenceConfiguration(scope: String = "alice", mode: String = "both", background: Bool = true,
                                   radius: Double = 200, count: Int = 1) throws -> NativeGeofenceConfiguration {
    let data = try JSONSerialization.data(withJSONObject: ["scope": scope, "registrations": (0..<count).map { index in
        ["id": "registration-\(index)", "appId": "app", "eventId": "event-\(index)", "latitude": 52.52,
         "longitude": 13.405, "radiusMeters": radius, "transition": mode, "background": background] as [String: Any]
    }])
    return try JSONDecoder().decode(NativeGeofenceConfiguration.self, from: data)
}

@Test func geofencesRejectScopeMismatchAndUnsupportedRegionConfiguration() throws {
    #expect(throws: (any Error).self) { try geofenceConfiguration().validate(currentScope: "bob") }
    #expect(throws: (any Error).self) { try geofenceConfiguration(count: 21).validate(currentScope: "alice") }
    #expect(throws: (any Error).self) { try geofenceConfiguration(radius: 99).validate(currentScope: "alice") }
    #expect(throws: (any Error).self) { try geofenceConfiguration(radius: 100_001).validate(currentScope: "alice") }
    #expect(throws: (any Error).self) { try geofenceConfiguration(mode: "polygon").validate(currentScope: "alice") }
}

@Test func geofencePeekPersistsUntilExplicitAcknowledgment() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeGeofenceStore(directory: directory)
    let configuration = try geofenceConfiguration()
    try store.replace(configuration, currentScope: "alice")
    let systemID = configuration.registrations[0].systemID(scope: "alice")
    let event = try #require(try store.append(systemID: systemID, transition: "enter", currentScope: "alice", foreground: false))
    #expect(event.geometry.coordinates == [13.405, 52.52])
    #expect(try store.events(currentScope: "alice").first?.id == event.id)
    #expect(try NativeGeofenceStore(directory: directory).events(currentScope: "alice").first?.id == event.id)
    #expect(try store.events(currentScope: "bob").isEmpty)
    try store.acknowledge([event.id])
    #expect(try store.events(currentScope: "alice").isEmpty)
}

@Test func enterOnlyGeofenceFiresAgainAfterExit() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeGeofenceStore(directory: directory)
    let configuration = try geofenceConfiguration(mode: "enter")
    try store.replace(configuration, currentScope: "alice")
    let systemID = configuration.registrations[0].systemID(scope: "alice")
    #expect(try store.append(systemID: systemID, transition: "enter", currentScope: "alice", foreground: false) != nil)
    #expect(try store.append(systemID: systemID, transition: "enter", currentScope: "alice", foreground: false) == nil)
    #expect(try store.append(systemID: systemID, transition: "exit", currentScope: "alice", foreground: false) == nil)
    #expect(try store.append(systemID: systemID, transition: "enter", currentScope: "alice", foreground: false) != nil)
    #expect(try store.events(currentScope: "alice").count == 2)
}

@Test func foregroundGeofenceResetsStateAfterMonitoringRestarts() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeGeofenceStore(directory: directory)
    let configuration = try geofenceConfiguration(background: false)
    try store.replace(configuration, currentScope: "alice")
    let systemID = configuration.registrations[0].systemID(scope: "alice")
    #expect(try store.append(systemID: systemID, transition: "enter", currentScope: "alice", foreground: true) != nil)
    #expect(try store.append(systemID: systemID, transition: "exit", currentScope: "alice", foreground: false) == nil)
    try store.resetTransitions([systemID])
    #expect(try store.append(systemID: systemID, transition: "enter", currentScope: "alice", foreground: true) != nil)
}

@Test func geofenceChangesAndExpiredTransitionsCannotReplay() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeGeofenceStore(directory: directory)
    let configuration = try geofenceConfiguration()
    try store.replace(configuration, currentScope: "alice")
    let systemID = configuration.registrations[0].systemID(scope: "alice")
    _ = try store.append(systemID: systemID, transition: "enter", currentScope: "alice", foreground: true,
                         now: Date().addingTimeInterval(-3601))
    #expect(try store.events(currentScope: "alice").isEmpty)
    _ = try store.append(systemID: systemID, transition: "exit", currentScope: "alice", foreground: true)
    try store.replace(geofenceConfiguration(radius: 300), currentScope: "alice")
    #expect(try store.events(currentScope: "alice").isEmpty)
    #expect(try store.append(systemID: systemID, transition: "enter", currentScope: "alice", foreground: true) == nil)
}

@Test func expiredWidgetSnapshotDoesNotClearSameAccountGeofences() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let native = NativeStore(directory: directory)
    let geofences = NativeGeofenceStore(directory: directory)
    func snapshot(scope: String, expires: Date) throws -> Data {
        try JSONSerialization.data(withJSONObject: ["version": 1, "scope": scope,
            "generatedAt": ISO8601DateFormatter().string(from: Date()), "expiresAt": ISO8601DateFormatter().string(from: expires),
            "sections": [], "events": [], "apps": []])
    }
    _ = try native.publish(snapshot(scope: "alice", expires: Date().addingTimeInterval(3600)))
    let configuration = try geofenceConfiguration()
    try geofences.replace(configuration, currentScope: native.persistedScope())
    _ = try geofences.append(systemID: configuration.registrations[0].systemID(scope: "alice"), transition: "enter",
                             currentScope: "alice", foreground: false)
    try snapshot(scope: "alice", expires: .distantPast).write(to: directory.appendingPathComponent("snapshot.json"))
    #expect(native.snapshot() == nil)
    #expect(native.persistedScope() == "alice")
    _ = try native.publish(snapshot(scope: "alice", expires: Date().addingTimeInterval(3600)))
    #expect(try geofences.events(currentScope: "alice").count == 1)
    _ = try native.publish(snapshot(scope: "bob", expires: Date().addingTimeInterval(3600)))
    #expect(geofences.configuration() == nil)
    #expect(try geofences.events(currentScope: "alice").isEmpty)
}

@Test func foregroundReleaseMakesOldBudgetExpirationHarmless() {
    var lease = NativeGeofenceLeaseState()
    let old = lease.begin(scope: "alice", deadline: Date().addingTimeInterval(25))
    let released = lease.finish()
    #expect(released)
    #expect(lease.deadline == nil)
    // A queued timer callback from the old background session must not affect a new one.
    let current = lease.begin(scope: "alice", deadline: Date().addingTimeInterval(25))
    let staleExpiration = lease.finish(expectedToken: old)
    #expect(!staleExpiration)
    #expect(lease.token == current)
    let currentExpiration = lease.finish(expectedToken: current)
    let repeatedExpiration = lease.finish(expectedToken: current)
    #expect(currentExpiration)
    #expect(!repeatedExpiration)
}

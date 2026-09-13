import CoreLocation
import Foundation
import Testing
@testable import FlowLikeNative

@Test func locationOptionsValidateRangesAndDeadlineWithoutAccessingSensors() throws {
    let decoder = JSONDecoder()
    let defaults = try decoder.decode(NativeLocationOptions.self, from: Data("{}".utf8))
    #expect(!defaults.highAccuracy)
    #expect(defaults.maximumAgeMs == 0)
    #expect(defaults.timeoutMs == 10_000)
    for json in ["{\"highAccuracy\":null}", "{\"highAccuracy\":1}", "{\"maximumAgeMs\":300001}",
                 "{\"maximumAgeMs\":1.5}", "{\"timeoutMs\":99}", "{\"timeoutMs\":120001}"] {
        #expect(throws: (any Error).self) { try decoder.decode(NativeLocationOptions.self, from: Data(json.utf8)) }
    }
    let expired = try decoder.decode(NativeLocationOptions.self, from: Data("{\"requestDeadline\":1}".utf8))
    #expect(expired.remainingTime(at: Date()) < 0)
}

@Test func locationFixUsesGeoJSONLongitudeFirstAndNullForUnavailableMetadata() throws {
    let now = Date()
    let fix = try #require(nativeLocationFix(CLLocation(coordinate: CLLocationCoordinate2D(latitude: 52.52, longitude: 13.405),
        altitude: 0, horizontalAccuracy: 24, verticalAccuracy: -1, course: -1, speed: -1, timestamp: now),
        startedAt: now, maximumAgeMs: 0, now: now))
    let geometry = try #require(fix["geometry"] as? [String: Any])
    #expect(geometry["coordinates"] as? [Double] == [13.405, 52.52])
    #expect(fix["accuracy"] as? Double == 24)
    #expect(fix["timestamp"] as? Double == now.timeIntervalSince1970 * 1000)
    for field in ["altitude", "altitudeAccuracy", "speed", "heading"] { #expect(fix[field] is NSNull) }
}

@Test func locationFixRejectsStaleCacheAndInvalidAccuracy() {
    let now = Date()
    func location(age: TimeInterval, accuracy: Double = 50) -> CLLocation {
        CLLocation(coordinate: CLLocationCoordinate2D(latitude: 52, longitude: 13), altitude: 0,
                   horizontalAccuracy: accuracy, verticalAccuracy: -1, timestamp: now.addingTimeInterval(-age))
    }
    #expect(nativeLocationFix(location(age: 2), startedAt: now, maximumAgeMs: 0, now: now) == nil)
    #expect(nativeLocationFix(location(age: 2), startedAt: now, maximumAgeMs: 3000, now: now) != nil)
    #expect(nativeLocationFix(location(age: 4), startedAt: now, maximumAgeMs: 3000, now: now) == nil)
    #expect(nativeLocationFix(location(age: 0, accuracy: -1), startedAt: now, maximumAgeMs: 0, now: now) == nil)
}

@MainActor private final class TestLocationManager: NativeLocationManaging {
    weak var delegate: CLLocationManagerDelegate?
    var desiredAccuracy: CLLocationAccuracy = 0
    var allowsBackgroundLocationUpdates = false
    #if os(iOS)
    var authorizationStatus: CLAuthorizationStatus = .authorizedWhenInUse
    #else
    var authorizationStatus: CLAuthorizationStatus = .authorizedAlways
    #endif
    var location: CLLocation?
    var authorizationRequests = 0
    var updateRequests = 0
    var stops = 0

    func requestWhenInUseAuthorization() { authorizationRequests += 1 }
    func startUpdatingLocation() { updateRequests += 1 }
    func stopUpdatingLocation() { stops += 1 }
}

private func locationOptions(_ json: String = "{}") throws -> NativeLocationOptions {
    try JSONDecoder().decode(NativeLocationOptions.self, from: Data(json.utf8))
}

private func location(at timestamp: Date, accuracy: Double = 50) -> CLLocation {
    CLLocation(coordinate: CLLocationCoordinate2D(latitude: 52.52, longitude: 13.405), altitude: 0,
               horizontalAccuracy: accuracy, verticalAccuracy: -1, timestamp: timestamp)
}

@Test @MainActor func locationRequestUsesRememberedPermissionAndWaitsForFreshFix() throws {
    let now = Date()
    let manager = TestLocationManager()
    manager.location = location(at: now.addingTimeInterval(-2))
    var replies: [[String: Any]] = []
    let request = NativeLocationRequest(options: try locationOptions(), manager: manager, now: { now },
        isForeground: { true }, canRequestAuthorization: { true }) { replies.append($0) }
    request.start()
    #expect(manager.authorizationRequests == 0)
    #expect(manager.updateRequests == 1)

    request.receiveLocations([manager.location!])
    request.receiveLocations([location(at: now, accuracy: -1)])
    request.receiveError(NSError(domain: kCLErrorDomain, code: CLError.locationUnknown.rawValue))
    #expect(replies.isEmpty)
    #expect(manager.stops == 0)

    request.receiveLocations([location(at: now)])
    #expect(replies.count == 1)
    #expect(replies.first?["ok"] as? Bool == true)
    #expect(manager.stops == 1)
    #expect(manager.delegate == nil)
    request.receiveLocations([location(at: now)])
    request.fail("cancelled", "Cancelled")
    #expect(replies.count == 1)
}

@Test @MainActor func locationRequestWaitsForActiveAppBeforePrompting() throws {
    let manager = TestLocationManager()
    manager.authorizationStatus = .notDetermined
    var active = false
    var replies: [[String: Any]] = []
    let request = NativeLocationRequest(options: try locationOptions(), manager: manager,
        isForeground: { true }, canRequestAuthorization: { active }) { replies.append($0) }
    defer { request.fail("cancelled", "Cancelled") }
    request.start()
    #expect(manager.authorizationRequests == 0)
    #expect(manager.updateRequests == 0)
    #expect(replies.isEmpty)

    active = true
    request.updateAuthorization()
    request.updateAuthorization()
    #expect(manager.authorizationRequests == 1)
    #if os(iOS)
    manager.authorizationStatus = .authorizedWhenInUse
    #else
    manager.authorizationStatus = .authorizedAlways
    #endif
    request.updateAuthorization()
    request.updateAuthorization()
    #expect(manager.updateRequests == 1)
    #expect(replies.isEmpty)
}

@Test @MainActor func locationRequestReportsRememberedDenialWithoutPrompting() throws {
    for status: CLAuthorizationStatus in [.denied, .restricted] {
        let manager = TestLocationManager()
        manager.authorizationStatus = status
        var replies: [[String: Any]] = []
        let request = NativeLocationRequest(options: try locationOptions(), manager: manager,
            isForeground: { true }, canRequestAuthorization: { true }) { replies.append($0) }
        request.start()
        request.updateAuthorization()
        #expect(manager.authorizationRequests == 0)
        #expect(manager.updateRequests == 0)
        #expect(replies.count == 1)
        let error = try #require(replies.first?["error"] as? [String: String])
        #expect(error["code"] == "permission_denied")
        #expect(manager.delegate == nil)
    }
}

@Test @MainActor func locationRequestUsesAllowedCacheWithoutStartingSensors() throws {
    let now = Date()
    let manager = TestLocationManager()
    manager.location = location(at: now.addingTimeInterval(-2))
    var replies: [[String: Any]] = []
    let request = NativeLocationRequest(options: try locationOptions("{\"maximumAgeMs\":3000}"),
        manager: manager, now: { now }, isForeground: { true }, canRequestAuthorization: { true }) { replies.append($0) }
    request.start()
    #expect(manager.updateRequests == 0)
    #expect(replies.count == 1)
    #expect(replies.first?["ok"] as? Bool == true)
}

@Test @MainActor func locationRequestStopsAtDeadlineDespiteTemporaryFailures() throws {
    var now = Date()
    let manager = TestLocationManager()
    var replies: [[String: Any]] = []
    let request = NativeLocationRequest(options: try locationOptions(), manager: manager, now: { now },
        isForeground: { true }, canRequestAuthorization: { true }) { replies.append($0) }
    request.start()
    request.receiveError(NSError(domain: kCLErrorDomain, code: CLError.locationUnknown.rawValue))
    #expect(replies.isEmpty)
    now = now.addingTimeInterval(11)
    request.receiveLocations([location(at: now)])
    #expect(replies.count == 1)
    #expect((replies.first?["error"] as? [String: String])?["code"] == "timeout")
    #expect(manager.stops == 1)
}

@Test @MainActor func locationRequestTimesOutWhenCoreLocationDoesNotReply() async throws {
    let manager = TestLocationManager()
    var replies: [[String: Any]] = []
    let request = NativeLocationRequest(options: try locationOptions("{\"timeoutMs\":100}"), manager: manager,
        isForeground: { true }, canRequestAuthorization: { true }) { replies.append($0) }
    request.start()
    try await Task.sleep(for: .milliseconds(250))
    #expect(replies.count == 1)
    #expect((replies.first?["error"] as? [String: String])?["code"] == "timeout")
    #expect(manager.stops == 1)
    #expect(manager.delegate == nil)
}

@Test @MainActor func locationRequestStopsIfTheAppBecomesHidden() throws {
    let manager = TestLocationManager()
    var visible = true
    var replies: [[String: Any]] = []
    let request = NativeLocationRequest(options: try locationOptions(), manager: manager,
        isForeground: { visible }, canRequestAuthorization: { true }) { replies.append($0) }
    request.start()
    visible = false
    request.receiveLocations([location(at: Date())])
    #expect(replies.count == 1)
    #expect((replies.first?["error"] as? [String: String])?["code"] == "inactive")
    #expect(manager.stops == 1)
}

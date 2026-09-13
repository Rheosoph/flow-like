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

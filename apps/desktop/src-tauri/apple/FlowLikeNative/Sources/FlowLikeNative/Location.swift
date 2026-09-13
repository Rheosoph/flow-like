import CoreLocation
import Foundation
#if os(iOS)
import UIKit
#else
import AppKit
#endif

struct NativeLocationOptions: Decodable {
    var highAccuracy: Bool = false
    var maximumAgeMs: Double = 0
    var timeoutMs: Double = 10_000
    var requestDeadline: Double?

    enum CodingKeys: CodingKey { case highAccuracy, maximumAgeMs, timeoutMs, requestDeadline }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        highAccuracy = try values.contains(.highAccuracy) ? values.decode(Bool.self, forKey: .highAccuracy) : false
        maximumAgeMs = try values.contains(.maximumAgeMs) ? values.decode(Double.self, forKey: .maximumAgeMs) : 0
        timeoutMs = try values.contains(.timeoutMs) ? values.decode(Double.self, forKey: .timeoutMs) : 10_000
        requestDeadline = try values.contains(.requestDeadline) ? values.decode(Double.self, forKey: .requestDeadline) : nil
        guard maximumAgeMs.isFinite, maximumAgeMs.rounded() == maximumAgeMs, (0...300_000).contains(maximumAgeMs),
              timeoutMs.isFinite, timeoutMs.rounded() == timeoutMs, (100...120_000).contains(timeoutMs),
              requestDeadline.map({ $0.isFinite && $0 > 0 }) ?? true else {
            throw NativeLocationFailure("invalid_input", "Invalid location accuracy, cache age, timeout, or deadline.")
        }
    }

    func remainingTime(at now: Date) -> TimeInterval {
        min(timeoutMs / 1000, requestDeadline.map { $0 / 1000 - now.timeIntervalSince1970 } ?? .infinity)
    }
}

struct NativeLocationFailure: Error {
    let code: String
    let message: String
    init(_ code: String, _ message: String) { self.code = code; self.message = message }
    var reply: [String: Any] { ["ok": false, "error": ["code": code, "message": message]] }
}

func nativeLocationFix(_ location: CLLocation, startedAt: Date, maximumAgeMs: Double, now: Date) -> [String: Any]? {
    guard CLLocationCoordinate2DIsValid(location.coordinate), location.horizontalAccuracy.isFinite,
          location.horizontalAccuracy >= 0, location.timestamp.timeIntervalSince(now) <= 1,
          location.timestamp >= startedAt || (maximumAgeMs > 0 && now.timeIntervalSince(location.timestamp) * 1000 <= maximumAgeMs) else {
        return nil
    }
    let coordinate = location.coordinate
    func valid(_ value: Double, nonnegative: Bool = true) -> Any {
        value.isFinite && (!nonnegative || value >= 0) ? value as Any : NSNull()
    }
    return [
        "geometry": ["type": "Point", "coordinates": [coordinate.longitude, coordinate.latitude]],
        "latitude": coordinate.latitude, "longitude": coordinate.longitude,
        "accuracy": location.horizontalAccuracy, "timestamp": location.timestamp.timeIntervalSince1970 * 1000,
        "altitude": location.verticalAccuracy >= 0 ? valid(location.altitude, nonnegative: false) : NSNull(),
        "altitudeAccuracy": valid(location.verticalAccuracy), "speed": valid(location.speed), "heading": valid(location.course),
    ]
}

public typealias NativeLocationCallback = @convention(c) (UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> Void

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@MainActor func nativeLocationScreenVisible() -> Bool {
    #if os(iOS)
    UIApplication.shared.applicationState != .background
    #else
    !NSApplication.shared.isHidden && NSApplication.shared.windows.contains { $0.isVisible && !$0.isMiniaturized && $0.level == .normal }
    #endif
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@MainActor func nativeLocationHiddenNotifications() -> [Notification.Name] {
    #if os(iOS)
    [UIApplication.didEnterBackgroundNotification]
    #else
    [NSApplication.didHideNotification, NSWindow.didMiniaturizeNotification, NSWindow.willCloseNotification]
    #endif
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@MainActor private enum NativeLocationService {
    static var requests: [String: NativeLocationRequest] = [:]

    static var isForeground: Bool {
        nativeLocationScreenVisible()
    }

    static func begin(id: String, json: Data, callback: @escaping NativeLocationCallback) {
        func reply(_ value: [String: Any]) {
            let data = (try? JSONSerialization.data(withJSONObject: value)) ?? Data("{\"ok\":false}".utf8)
            id.withCString { requestID in
                String(decoding: data, as: UTF8.self).withCString { callback(requestID, $0) }
            }
        }
        guard !id.isEmpty, id.utf8.count <= 128, requests[id] == nil, requests.count < 16 else {
            reply(NativeLocationFailure("invalid_input", "This location request cannot be started.").reply)
            return
        }
        guard isForeground else {
            reply(NativeLocationFailure("inactive", "Open Flow Like to request your current location.").reply)
            return
        }
        do {
            let options = try JSONDecoder().decode(NativeLocationOptions.self, from: json)
            guard options.remainingTime(at: Date()) > 0 else {
                reply(NativeLocationFailure("timeout", "The location request has expired.").reply)
                return
            }
            let request = NativeLocationRequest(options: options) { result in
                requests.removeValue(forKey: id)
                reply(result)
            }
            requests[id] = request
            request.start()
        } catch {
            reply(NativeLocationFailure("invalid_input", "Invalid location options.").reply)
        }
    }

    static func cancel(_ id: String) {
        requests[id]?.fail("cancelled", "The location request was cancelled.")
    }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@MainActor private final class NativeLocationRequest: NSObject, @preconcurrency CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    private let options: NativeLocationOptions
    private let startedAt = Date()
    private let completion: ([String: Any]) -> Void
    private var timer: Timer?
    private var observers: [NSObjectProtocol] = []
    private var finished = false
    private var authorizationRequested = false
    private var locationRequested = false

    init(options: NativeLocationOptions, completion: @escaping ([String: Any]) -> Void) {
        self.options = options
        self.completion = completion
        super.init()
        manager.delegate = self
        manager.desiredAccuracy = options.highAccuracy ? kCLLocationAccuracyBest : kCLLocationAccuracyHundredMeters
        manager.allowsBackgroundLocationUpdates = false
    }

    func start() {
        let timer = Timer(timeInterval: options.remainingTime(at: startedAt), repeats: false) { [weak self] _ in
            Task { @MainActor in self?.fail("timeout", "The location request timed out.") }
        }
        self.timer = timer
        RunLoop.main.add(timer, forMode: .common)
        #if os(iOS)
        let active = UIApplication.didBecomeActiveNotification
        #else
        let active = NSApplication.didBecomeActiveNotification
        #endif
        for inactive in nativeLocationHiddenNotifications() {
            observers.append(NotificationCenter.default.addObserver(forName: inactive, object: nil, queue: .main) { [weak self] _ in
                Task { @MainActor in
                    if !nativeLocationScreenVisible() { self?.fail("inactive", "The location request stopped when Flow Like was hidden.") }
                }
            })
        }
        observers.append(NotificationCenter.default.addObserver(forName: active, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.updateAuthorization() }
        })
        updateAuthorization()
    }

    private func updateAuthorization() {
        guard !finished, NativeLocationService.isForeground else { return }
        switch manager.authorizationStatus {
        case .notDetermined:
            if !authorizationRequested {
                authorizationRequested = true
                manager.requestWhenInUseAuthorization()
            }
        case .denied, .restricted:
            fail("permission_denied", "Location access is not allowed for Flow Like.")
        default:
            guard !locationRequested else { return }
            locationRequested = true
            if options.maximumAgeMs > 0, let cached = manager.location,
               let fix = nativeLocationFix(cached, startedAt: startedAt, maximumAgeMs: options.maximumAgeMs, now: Date()) {
                finish(["ok": true, "value": fix])
            } else {
                manager.requestLocation()
            }
        }
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) { updateAuthorization() }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard NativeLocationService.isForeground else {
            fail("inactive", "Open Flow Like to receive the requested location.")
            return
        }
        guard Date() < startedAt.addingTimeInterval(options.remainingTime(at: startedAt)) else {
            fail("timeout", "The location request has expired.")
            return
        }
        guard let fix = locations.reversed().compactMap({
            nativeLocationFix($0, startedAt: startedAt, maximumAgeMs: options.maximumAgeMs, now: Date())
        }).first else {
            fail("position_unavailable", "A current location with valid accuracy is unavailable.")
            return
        }
        finish(["ok": true, "value": fix])
    }

    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        let denied = (error as? CLError)?.code == .denied
        fail(denied ? "permission_denied" : "position_unavailable",
             denied ? "Location access is not allowed for Flow Like." : "Your current location could not be determined.")
    }

    func fail(_ code: String, _ message: String) { finish(NativeLocationFailure(code, message).reply) }

    private func finish(_ reply: [String: Any]) {
        guard !finished else { return }
        finished = true
        timer?.invalidate()
        timer = nil
        manager.stopUpdatingLocation()
        manager.delegate = nil
        observers.forEach(NotificationCenter.default.removeObserver)
        observers.removeAll()
        completion(reply)
    }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@_cdecl("flow_like_native_get_location")
public func flowLikeNativeGetLocation(_ requestID: UnsafePointer<CChar>?, _ options: UnsafePointer<CChar>?, _ callback: @escaping NativeLocationCallback) {
    guard let requestID, let options else { return }
    let id = String(cString: requestID)
    let data = Data(String(cString: options).utf8)
    if Thread.isMainThread {
        MainActor.assumeIsolated { NativeLocationService.begin(id: id, json: data, callback: callback) }
    } else {
        DispatchQueue.main.async { NativeLocationService.begin(id: id, json: data, callback: callback) }
    }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@_cdecl("flow_like_native_cancel_location")
public func flowLikeNativeCancelLocation(_ requestID: UnsafePointer<CChar>?) {
    guard let requestID else { return }
    let id = String(cString: requestID)
    if Thread.isMainThread {
        MainActor.assumeIsolated { NativeLocationService.cancel(id) }
    } else {
        DispatchQueue.main.async { NativeLocationService.cancel(id) }
    }
}

import CoreLocation
import Foundation
#if os(iOS)
import UIKit
#else
import AppKit
#endif

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@MainActor enum NativeGeofenceIntegration {
    static var service: NativeGeofenceService?
    static func instance() -> NativeGeofenceService {
        if let service { return service }
        let created = NativeGeofenceService()
        service = created
        return created
    }
    static func scopeChanged() { service?.scopeChanged() }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@MainActor final class NativeGeofenceService: NSObject, @preconcurrency CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    var callback: (@convention(c) () -> Void)?
    var expirationCallback: (@convention(c) () -> Void)?
    private var registryAuthorized = false
    private var authorizedScope: String?
    private var observers: [NSObjectProtocol] = []
    private var lastError: NativeLocationFailure?
    #if os(iOS)
    private var backgroundTask: UIBackgroundTaskIdentifier = .invalid
    private var backgroundLease = NativeGeofenceLeaseState()
    private var backgroundTimer: Timer?
    #endif

    override init() {
        super.init()
        manager.delegate = self
        #if os(iOS)
        let foreground = UIApplication.didBecomeActiveNotification
        #else
        let foreground = NSApplication.didBecomeActiveNotification
        #endif
        for name in nativeLocationHiddenNotifications() + [foreground] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                Task { @MainActor in
                    guard let self else { return }
                    #if os(iOS)
                    if UIApplication.shared.applicationState != .background { self.finishBackgroundTask(cancelDispatch: false) }
                    #endif
                    if self.registryAuthorized { self.reconcile() }
                    self.callback?()
                }
            })
        }
    }

    func permission(mode: String) throws -> [String: Any] {
        guard ["status", "foreground", "background"].contains(mode) else {
            throw NativeLocationFailure("invalid_input", "Choose status, foreground, or background location permission.")
        }
        if mode != "status" {
            guard nativeLocationScreenVisible() else { throw NativeLocationFailure("inactive", "Open Flow Like to change location access.") }
            #if os(iOS)
            if mode == "background" { manager.requestAlwaysAuthorization() }
            else { manager.requestWhenInUseAuthorization() }
            #else
            manager.requestWhenInUseAuthorization()
            #endif
        }
        return status()
    }

    private var authorization: String {
        switch manager.authorizationStatus {
        case .notDetermined: "not_determined"
        case .restricted: "restricted"
        case .denied: "denied"
        #if os(iOS)
        case .authorizedWhenInUse: "when_in_use"
        #endif
        case .authorizedAlways: "always"
        @unknown default: "denied"
        }
    }

    func status() -> [String: Any] {
        var value: [String: Any] = [
            "authorization": authorization,
            "accuracyAuthorization": manager.accuracyAuthorization == .fullAccuracy ? "full" : "reduced",
            "backgroundSupported": CLLocationManager.isMonitoringAvailable(for: CLCircularRegion.self),
            "monitoredRegionCount": manager.monitoredRegions.filter { $0.identifier.hasPrefix("flow-like-geo-") }.count,
            "maxMonitoredRegions": 20,
        ]
        #if os(iOS)
        value["backgroundDelivery"] = "system_monitored"
        #else
        value["backgroundDelivery"] = "app_running"
        #endif
        if let lastError { value["error"] = ["code": lastError.code, "message": lastError.message] }
        return value
    }

    func sync(_ configuration: NativeGeofenceConfiguration) throws -> [String: Any] {
        try configuration.validate(currentScope: NativeStore.shared.persistedScope())
        let others = manager.monitoredRegions.filter { !$0.identifier.hasPrefix("flow-like-geo-") }.count
        guard configuration.registrations.count + others <= 20 else {
            throw NativeLocationFailure("region_limit", "Apple devices support at most 20 monitored regions per app.")
        }
        if !configuration.registrations.isEmpty {
            guard CLLocationManager.isMonitoringAvailable(for: CLCircularRegion.self) else {
                throw NativeLocationFailure("unsupported", "Region monitoring is unavailable on this device.")
            }
            let maximum = manager.maximumRegionMonitoringDistance
            guard maximum > 0, configuration.registrations.allSatisfy({ $0.radiusMeters <= maximum }) else {
                throw NativeLocationFailure("invalid_input", "A geofence radius exceeds this device's monitoring limit.")
            }
        }
        try NativeGeofenceStore.shared.replace(configuration, currentScope: NativeStore.shared.persistedScope())
        if let authorizedScope, authorizedScope != configuration.scope { finishBackgroundTask(cancelDispatch: true) }
        #if os(iOS)
        if let scope = backgroundLease.scope, scope != configuration.scope { finishBackgroundTask(cancelDispatch: true) }
        #endif
        authorizedScope = configuration.scope
        registryAuthorized = true
        reconcile()
        return status()
    }

    private func reconcile() {
        let current = manager.monitoredRegions.filter { $0.identifier.hasPrefix("flow-like-geo-") }
        guard let configuration = NativeGeofenceStore.shared.configuration(),
              configuration.scope == NativeStore.shared.persistedScope() else {
            clearMonitoring()
            return
        }
        lastError = nil
        let authorized = authorization == "always" || authorization == "when_in_use"
        if !authorized && !configuration.registrations.isEmpty {
            lastError = NativeLocationFailure("permission_required", "Allow location access to activate location Events.")
        } else if manager.accuracyAuthorization != .fullAccuracy && !configuration.registrations.isEmpty {
            lastError = NativeLocationFailure("precise_location_required", "Region monitoring needs Precise Location in system settings.")
        } else if configuration.registrations.contains(where: \.background) && authorization != "always" {
            lastError = NativeLocationFailure("background_permission_required", "Allow Always location access to activate background location Events.")
        }
        let visible = nativeLocationScreenVisible()
        let desired = configuration.registrations.filter {
            authorized && manager.accuracyAuthorization == .fullAccuracy
                && ($0.background ? authorization == "always" : visible)
        }
        let desiredIDs = Set(desired.map { $0.systemID(scope: configuration.scope) })
        for region in current where !desiredIDs.contains(region.identifier) {
            manager.stopMonitoring(for: region)
            try? NativeGeofenceStore.shared.resetTransitions([region.identifier])
        }
        let existing = Set(current.map(\.identifier))
        for registration in desired {
            let identifier = registration.systemID(scope: configuration.scope)
            guard !existing.contains(identifier) else { continue }
            try? NativeGeofenceStore.shared.resetTransitions([identifier])
            let region = CLCircularRegion(center: CLLocationCoordinate2D(latitude: registration.latitude, longitude: registration.longitude),
                                          radius: registration.radiusMeters, identifier: identifier)
            // Observe both sides so an enter-only Event can fire again after a later exit.
            region.notifyOnEntry = true
            region.notifyOnExit = true
            manager.startMonitoring(for: region)
        }
    }

    func clearMonitoring() {
        registryAuthorized = false
        authorizedScope = nil
        for region in manager.monitoredRegions where region.identifier.hasPrefix("flow-like-geo-") {
            manager.stopMonitoring(for: region)
        }
        finishBackgroundTask(cancelDispatch: true)
        callback?()
    }

    func scopeChanged() {
        let scope = NativeStore.shared.persistedScope()
        if authorizedScope != scope || NativeGeofenceStore.shared.configuration()?.scope != scope { clearMonitoring() }
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        if registryAuthorized { reconcile() }
        callback?()
    }
    func locationManager(_ manager: CLLocationManager, monitoringDidFailFor region: CLRegion?, withError error: Error) {
        lastError = NativeLocationFailure("region_monitoring_failed", "A location Event could not be monitored. Check location access and region limits.")
        callback?()
    }
    func locationManager(_ manager: CLLocationManager, didEnterRegion region: CLRegion) { transition(region, "enter") }
    func locationManager(_ manager: CLLocationManager, didExitRegion region: CLRegion) { transition(region, "exit") }

    private func transition(_ region: CLRegion, _ transition: String) {
        do {
            if try NativeGeofenceStore.shared.append(systemID: region.identifier, transition: transition,
                    currentScope: NativeStore.shared.persistedScope(), foreground: nativeLocationScreenVisible()) != nil {
                beginBackgroundTask()
                callback?()
            }
        } catch {
            lastError = NativeLocationFailure("storage_unavailable", "A location Event could not be saved for delivery.")
            callback?()
        }
    }

    private func beginBackgroundTask() {
        #if os(iOS)
        guard UIApplication.shared.applicationState == .background, backgroundTask == .invalid else { return }
        let token = backgroundLease.begin(scope: NativeStore.shared.persistedScope(), deadline: Date().addingTimeInterval(25))
        backgroundTask = UIApplication.shared.beginBackgroundTask(withName: "Deliver location Events") { [weak self] in
            MainActor.assumeIsolated { self?.expireBackgroundTask(token: token) }
        }
        guard backgroundTask != .invalid else {
            _ = backgroundLease.finish(expectedToken: token)
            return
        }
        let duration = min(25, UIApplication.shared.backgroundTimeRemaining)
        let timer = Timer(timeInterval: duration, repeats: false) { [weak self] _ in
            Task { @MainActor in self?.expireBackgroundTask(token: token) }
        }
        backgroundTimer = timer
        RunLoop.main.add(timer, forMode: .common)
        #endif
    }

    func backgroundTimeRemaining() -> Double {
        #if os(iOS)
        if UIApplication.shared.applicationState != .background { return -1 }
        guard backgroundTask != .invalid, let backgroundDeadline = backgroundLease.deadline else { return 0 }
        return max(0, min(backgroundDeadline.timeIntervalSinceNow, UIApplication.shared.backgroundTimeRemaining))
        #else
        return -1
        #endif
    }

    private func expireBackgroundTask(token: UUID) {
        #if os(iOS)
        finishBackgroundTask(expectedToken: token, cancelDispatch: UIApplication.shared.applicationState == .background)
        #endif
    }

    private func finishBackgroundTask(expectedToken: UUID? = nil, cancelDispatch: Bool) {
        #if os(iOS)
        guard backgroundLease.finish(expectedToken: expectedToken) else { return }
        backgroundTimer?.invalidate()
        backgroundTimer = nil
        if backgroundTask != .invalid {
            UIApplication.shared.endBackgroundTask(backgroundTask)
            backgroundTask = .invalid
        }
        #endif
        if cancelDispatch { expirationCallback?() }
    }
}

private func geofenceReply(_ body: () throws -> [String: Any]) -> UnsafeMutablePointer<CChar>? {
    let reply: [String: Any]
    do { reply = ["ok": true, "value": try body()] }
    catch let failure as NativeLocationFailure { reply = failure.reply }
    catch { reply = NativeLocationFailure("unavailable", "Location Event storage is unavailable.").reply }
    let data = (try? JSONSerialization.data(withJSONObject: reply)) ?? Data("{\"ok\":false}".utf8)
    return strdup(String(decoding: data, as: UTF8.self))
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
private func geofenceOnMain<T>(_ operation: @MainActor () -> T) -> T {
    if Thread.isMainThread { return MainActor.assumeIsolated { operation() } }
    return DispatchQueue.main.sync { operation() }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
func nativeGeofenceScopeChanged() { geofenceOnMain { NativeGeofenceIntegration.scopeChanged() } }

@_cdecl("flow_like_native_scope")
public func flowLikeNativeScope() -> UnsafeMutablePointer<CChar>? {
    NativeStore.shared.persistedScope().flatMap { strdup($0) }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@_cdecl("flow_like_native_sync_geofences")
public func flowLikeNativeSyncGeofences(_ json: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    guard let json else { return nil }
    let data = Data(String(cString: json).utf8)
    return geofenceOnMain {
        geofenceReply {
            guard data.count <= 131_072 else { throw NativeLocationFailure("invalid_input", "The location Event configuration is too large.") }
            return try NativeGeofenceIntegration.instance().sync(JSONDecoder().decode(NativeGeofenceConfiguration.self, from: data))
        }
    }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@_cdecl("flow_like_native_geofence_permission")
public func flowLikeNativeGeofencePermission(_ mode: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    let mode = mode.map { String(cString: $0) } ?? "status"
    return geofenceOnMain { geofenceReply { try NativeGeofenceIntegration.instance().permission(mode: mode) } }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@_cdecl("flow_like_native_set_geofence_callback")
public func flowLikeNativeSetGeofenceCallback(_ callback: @escaping @convention(c) () -> Void) {
    geofenceOnMain {
        NativeGeofenceIntegration.instance().callback = callback
        callback()
    }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@_cdecl("flow_like_native_set_geofence_expiration_callback")
public func flowLikeNativeSetGeofenceExpirationCallback(_ callback: @escaping @convention(c) () -> Void) {
    geofenceOnMain { NativeGeofenceIntegration.instance().expirationCallback = callback }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@_cdecl("flow_like_native_geofence_background_time_remaining")
public func flowLikeNativeGeofenceBackgroundTimeRemaining() -> Double {
    geofenceOnMain { NativeGeofenceIntegration.instance().backgroundTimeRemaining() }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@_cdecl("flow_like_native_geofence_foreground")
public func flowLikeNativeGeofenceForeground() -> Int32 {
    geofenceOnMain { nativeLocationScreenVisible() ? 1 : 0 }
}

@_cdecl("flow_like_native_take_geofence_events")
public func flowLikeNativeTakeGeofenceEvents() -> UnsafeMutablePointer<CChar>? {
    guard let events = try? NativeGeofenceStore.shared.events(currentScope: NativeStore.shared.persistedScope()),
          let data = try? JSONEncoder().encode(events) else { return strdup("[]") }
    return strdup(String(decoding: data, as: UTF8.self))
}

@_cdecl("flow_like_native_ack_geofence_events")
public func flowLikeNativeAckGeofenceEvents(_ json: UnsafePointer<CChar>?) -> Int32 {
    guard let json else { return -1 }
    do {
        let data = Data(String(cString: json).utf8)
        guard data.count <= 65_536 else { return -1 }
        let ids = try JSONDecoder().decode([String].self, from: data)
        try NativeGeofenceStore.shared.acknowledge(Set(ids))
        return 0
    } catch { return -2 }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
public enum FlowLikeNativeRuntime {
    public static func prepareForAppLifecycle() {
        #if os(iOS)
        NotificationCenter.default.addObserver(forName: UIApplication.didFinishLaunchingNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated {
                // UIKit delivers launch activities before Rust finishes initializing.
                NativeActivityBridge.install()
                NativeSystemIntegration.refreshAppShortcuts?()
                // Receive persisted region callbacks before Rust and the webview finish launching.
                // Rust authorizes the current account before restoring the active registry.
                if NativeGeofenceStore.shared.configuration() != nil { _ = NativeGeofenceIntegration.instance() }
            }
        }
        #endif
    }
}

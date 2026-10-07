#if canImport(UIKit)
import FlowLikeMLX
import FlowLikeNative
import AppIntents
import UIKit
import UserNotifications
import ObjectiveC
import FirebaseCore
import FirebaseMessaging

/// Bridges iOS push-notification lifecycle events into the Tauri plugin.
///
/// Tao (Tauri's windowing layer) creates the ``AppDelegate`` Obj-C class at
/// runtime via `ClassDecl::new`.  We cannot subclass it from Swift, so we
/// inject the missing delegate methods using `class_addMethod` after
/// UIApplication finishes launching.
@objc(PushNotificationBridge)
final class PushNotificationBridge: NSObject, UNUserNotificationCenterDelegate, @unchecked Sendable {
    @objc static let shared = PushNotificationBridge()
    private weak var localNotificationDelegate: UNUserNotificationCenterDelegate?

    /// Keys used to bridge a tapped notification's userInfo to JS on cold-start.
    /// On a tap that launches the app from a terminated state, the plugin
    /// fires `notification-tapped` before the webview's JS bundle has had a
    /// chance to register a listener - so the event is lost. We additionally
    /// persist the userInfo into UserDefaults so JS can retrieve and consume
    /// it after startup via the `get_pending_notification_tap` Tauri command.
    static let pendingTapDefaultsKey = "FlowLike.PendingNotificationTap"
    static let pendingTapTimestampKey = "FlowLike.PendingNotificationTap.Timestamp"

    /// Call from `main()` **before** `ffi::start_app()`.
    /// Registers a one-shot observer that fires once the app has launched and
    /// Tao's AppDelegate class exists.
    @objc static func prepareForLaunch() {
        // Touch the static Swift package product so its C ABI symbols are
        // retained for Rust, and install MLX's iOS memory policy early.
        FlowLikeMLXRuntime.prepareForAppLifecycle()
        FlowLikeNativeRuntime.prepareForAppLifecycle()
        MainActor.assumeIsolated {
            NativeSystemIntegration.refreshAppShortcuts = {
                FlowLikeShortcuts.updateAppShortcutParameters()
            }
        }
        NotificationCenter.default.addObserver(
            forName: UIApplication.didFinishLaunchingNotification,
            object: nil,
            queue: nil // synchronous on posting thread
        ) { _ in
            MainActor.assumeIsolated { install() }
        }
    }

    // MARK: - Installation

    @MainActor private static func install() {
        let application = UIApplication.shared
        guard let delegate = application.delegate,
              let delegateClass = object_getClass(delegate) else {
            NSLog("[FlowLikePush] Cannot install push callbacks: application delegate is unavailable")
            return
        }

        // application:didRegisterForRemoteNotificationsWithDeviceToken:
        do {
            let sel = sel_registerName(
                "application:didRegisterForRemoteNotificationsWithDeviceToken:")
            let block: @convention(block) (AnyObject, UIApplication, Data) -> Void = {
                _, _, token in
                callPlugin(
                    "applicationDidRegisterForRemoteNotificationsWithDeviceToken:",
                    with: token as NSData)
                MainActor.assumeIsolated {
                    RemotePushTokenRequests.shared.didRegister(token)
                }
            }
            installMethod(
                delegateClass, sel,
                imp_implementationWithBlock(block), "v@:@@")
        }

        // application:didFailToRegisterForRemoteNotificationsWithError:
        do {
            let sel = sel_registerName(
                "application:didFailToRegisterForRemoteNotificationsWithError:")
            let block: @convention(block) (AnyObject, UIApplication, NSError) -> Void = {
                _, _, error in
                callPlugin(
                    "applicationDidFailToRegisterForRemoteNotificationsWithError:",
                    with: error)
                MainActor.assumeIsolated {
                    RemotePushTokenRequests.shared.didFail(error)
                }
            }
            installMethod(
                delegateClass, sel,
                imp_implementationWithBlock(block), "v@:@@")
        }

        // application:didReceiveRemoteNotification:fetchCompletionHandler:
        do {
            let sel = sel_registerName(
                "application:didReceiveRemoteNotification:fetchCompletionHandler:")
            let block:
                @convention(block) (
                    AnyObject, UIApplication, NSDictionary,
                    @escaping (UIBackgroundFetchResult) -> Void
                ) -> Void = { _, _, userInfo, handler in
                    callPlugin(
                        "applicationDidReceiveRemoteNotificationWithUserInfo:",
                        with: userInfo)
                    handler(.newData)
                }
            installMethod(
                delegateClass, sel,
                imp_implementationWithBlock(block), "v@:@@@")
        }

        // UIKit caches optional delegate methods when the delegate is assigned.
        // Refresh that cache after adding Tao's missing push callbacks.
        application.delegate = nil
        application.delegate = delegate
        installNotificationDelegate()
        callPlugin("configureFirebaseAppIfAvailable")
    }

    @MainActor static func installNotificationDelegate() {
        let center = UNUserNotificationCenter.current()
        if let previous = center.delegate, previous !== shared {
            shared.localNotificationDelegate = previous
        }
        center.delegate = shared
    }

    private static func installMethod(
        _ delegateClass: AnyClass, _ selector: Selector, _ implementation: IMP, _ types: String
    ) {
        guard class_addMethod(delegateClass, selector, implementation, types) else {
            imp_removeBlock(implementation)
            NSLog("[FlowLikePush] Push callback already exists: %@", NSStringFromSelector(selector))
            return
        }
    }

    // MARK: - Plugin RPC

    private static func callPlugin(_ selector: String, with arg: Any? = nil) {
        guard let cls = NSClassFromString("PushNotificationPlugin") else { return }
        let sel = NSSelectorFromString(selector)
        guard (cls as AnyObject).responds(to: sel) else { return }
        if let arg = arg {
            _ = (cls as AnyObject).perform(sel, with: arg)
        } else {
            _ = (cls as AnyObject).perform(sel)
        }
    }

    // MARK: - UNUserNotificationCenterDelegate

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping @Sendable (UNNotificationPresentationOptions) -> Void
    ) {
        if notification.request.trigger is UNPushNotificationTrigger {
            Self.callPlugin(
                "applicationDidReceiveRemoteNotificationWithUserInfo:",
                with: notification.request.content.userInfo as NSDictionary)
        } else if let previous = localNotificationDelegate,
                  previous.responds(to: #selector(UNUserNotificationCenterDelegate.userNotificationCenter(_:willPresent:withCompletionHandler:))) {
            previous.userNotificationCenter?(center, willPresent: notification, withCompletionHandler: completionHandler)
            return
        }
        if #available(iOS 14.0, *) {
            completionHandler([.banner, .sound, .badge])
        } else {
            completionHandler([.alert, .sound, .badge])
        }
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping @Sendable () -> Void
    ) {
        guard response.notification.request.trigger is UNPushNotificationTrigger else {
            if let previous = localNotificationDelegate,
               previous.responds(to: #selector(UNUserNotificationCenterDelegate.userNotificationCenter(_:didReceive:withCompletionHandler:))) {
                previous.userNotificationCenter?(center, didReceive: response, withCompletionHandler: completionHandler)
                return
            }
            completionHandler()
            return
        }
        let userInfo = response.notification.request.content.userInfo
        NSLog("[FlowLikePush] userNotificationCenter:didReceive: keys=\(userInfo.keys)")
        Self.persistPendingTap(userInfo)
        Self.callPlugin(
            "applicationDidReceiveNotificationResponseWithUserInfo:",
            with: userInfo as NSDictionary)
        completionHandler()
    }

    // MARK: - Cold-start tap persistence

    private static func persistPendingTap(_ userInfo: [AnyHashable: Any]) {
        let stringKeyed = userInfo.reduce(into: [String: Any]()) { acc, kv in
            if let key = kv.key as? String {
                acc[key] = kv.value
            }
        }
        guard JSONSerialization.isValidJSONObject(stringKeyed) else {
            NSLog("[FlowLikePush] persistPendingTap: payload not JSON-serializable, keys=\(stringKeyed.keys)")
            return
        }
        guard let data = try? JSONSerialization.data(withJSONObject: stringKeyed, options: []),
              let json = String(data: data, encoding: .utf8) else {
            NSLog("[FlowLikePush] persistPendingTap: JSON encoding failed")
            return
        }
        let defaults = UserDefaults.standard
        defaults.set(json, forKey: pendingTapDefaultsKey)
        defaults.set(Date().timeIntervalSince1970, forKey: pendingTapTimestampKey)
        NSLog("[FlowLikePush] persistPendingTap: stored \(json.count) bytes under \(pendingTapDefaultsKey)")
    }
}

typealias RemotePushTokenCallback = @convention(c) @Sendable (
    UnsafePointer<CChar>?, UnsafePointer<CChar>?, UnsafePointer<CChar>?
) -> Void

@MainActor
private final class RemotePushTokenRequests {
    static let shared = RemotePushTokenRequests()

    private struct Request {
        let callback: RemotePushTokenCallback
        var fetching = false
    }

    private var requests: [String: Request] = [:]

    func request(_ id: String, callback: @escaping RemotePushTokenCallback) {
        requests[id] = Request(callback: callback)
        guard Bundle.main.path(forResource: "GoogleService-Info", ofType: "plist") != nil else {
            finish(id, error: "Firebase configuration is missing from this app build.")
            return
        }
        if FirebaseApp.app() == nil {
            FirebaseApp.configure()
        }
        PushNotificationBridge.installNotificationDelegate()
        DispatchQueue.main.asyncAfter(deadline: .now() + 20) { [weak self] in
            self?.finish(id, error: "Push registration timed out. Check your connection and try again.")
        }
        // Always obtain the current APNs token before asking Firebase for its FCM token.
        UIApplication.shared.registerForRemoteNotifications()
    }

    func didRegister(_ token: Data) {
        guard FirebaseApp.app() != nil else { return }
        Messaging.messaging().apnsToken = token
        let waiting = requests.keys.filter { requests[$0]?.fetching == false }
        guard !waiting.isEmpty else { return }
        for id in waiting { requests[id]?.fetching = true }
        Messaging.messaging().token { token, error in
            DispatchQueue.main.async {
                for id in waiting {
                    if let error {
                        self.finish(id, error: "Firebase push registration failed: \(error.localizedDescription)")
                    } else if let token, !token.isEmpty {
                        self.finish(id, token: token)
                    } else {
                        self.finish(id, error: "Firebase returned an empty push token.")
                    }
                }
            }
        }
    }

    func didFail(_ error: Error) {
        for id in Array(requests.keys) {
            finish(id, error: "Apple push registration failed: \(error.localizedDescription)")
        }
    }

    func cancel(_ id: String) {
        requests.removeValue(forKey: id)
    }

    private func finish(_ id: String, token: String? = nil, error: String? = nil) {
        guard let request = requests.removeValue(forKey: id) else { return }
        id.withCString { idPointer in
            if let token {
                token.withCString { request.callback(idPointer, $0, nil) }
            } else {
                (error ?? "Push registration failed.").withCString { request.callback(idPointer, nil, $0) }
            }
        }
    }
}

@_cdecl("flow_like_request_remote_push_token")
func flowLikeRequestRemotePushToken(
    _ requestID: UnsafePointer<CChar>?, _ callback: @escaping RemotePushTokenCallback
) {
    guard let requestID else { return }
    let id = String(cString: requestID)
    DispatchQueue.main.async {
        RemotePushTokenRequests.shared.request(id, callback: callback)
    }
}

@_cdecl("flow_like_cancel_remote_push_token")
func flowLikeCancelRemotePushToken(_ requestID: UnsafePointer<CChar>?) {
    guard let requestID else { return }
    let id = String(cString: requestID)
    DispatchQueue.main.async {
        RemotePushTokenRequests.shared.cancel(id)
    }
}
#endif

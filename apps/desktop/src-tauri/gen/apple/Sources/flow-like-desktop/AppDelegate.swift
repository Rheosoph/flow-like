#if canImport(UIKit)
import UIKit
import UserNotifications
import ObjectiveC

/// Bridges iOS push-notification lifecycle events into the Tauri plugin.
///
/// Tao (Tauri's windowing layer) creates the ``AppDelegate`` Obj-C class at
/// runtime via `ClassDecl::new`.  We cannot subclass it from Swift, so we
/// inject the missing delegate methods using `class_addMethod` after
/// UIApplication finishes launching.
@objc(PushNotificationBridge)
final class PushNotificationBridge: NSObject, UNUserNotificationCenterDelegate, @unchecked Sendable {
    @objc static let shared = PushNotificationBridge()

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
        NotificationCenter.default.addObserver(
            forName: UIApplication.didFinishLaunchingNotification,
            object: nil,
            queue: nil // synchronous on posting thread
        ) { _ in
            install()
        }
    }

    // MARK: - Installation

    private static func install() {
        callPlugin("configureFirebaseAppIfAvailable")
        UNUserNotificationCenter.current().delegate = shared

        guard let delegateClass = NSClassFromString("AppDelegate") else { return }

        // application:didRegisterForRemoteNotificationsWithDeviceToken:
        do {
            let sel = sel_registerName(
                "application:didRegisterForRemoteNotificationsWithDeviceToken:")
            let block: @convention(block) (AnyObject, UIApplication, Data) -> Void = {
                _, _, token in
                callPlugin(
                    "applicationDidRegisterForRemoteNotificationsWithDeviceToken:",
                    with: token as NSData)
            }
            class_addMethod(
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
            }
            class_addMethod(
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
            class_addMethod(
                delegateClass, sel,
                imp_implementationWithBlock(block), "v@:@@@")
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
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        Self.callPlugin(
            "applicationDidReceiveRemoteNotificationWithUserInfo:",
            with: notification.request.content.userInfo as NSDictionary)
        if #available(iOS 14.0, *) {
            completionHandler([.banner, .sound, .badge])
        } else {
            completionHandler([.alert, .sound, .badge])
        }
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
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
#endif

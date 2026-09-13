import Foundation
#if os(iOS)
import UIKit
#else
import AppKit
#endif

enum NativeActionNotifier {
    static let name = "com.flow-like.app.native-action" as CFString
    private static let lock = NSLock()
    private static var callback: (@convention(c) () -> Void)?

    static func install(_ callback: (@convention(c) () -> Void)?) {
        lock.lock()
        self.callback = callback
        lock.unlock()
        let center = CFNotificationCenterGetDarwinNotifyCenter()
        CFNotificationCenterRemoveObserver(center, nil, CFNotificationName(name), nil)
        CFNotificationCenterAddObserver(center, nil, { _, _, _, _, _ in
            NativeActionNotifier.deliver()
        }, name, nil, .deliverImmediately)
    }

    static func post() {
        CFNotificationCenterPostNotification(CFNotificationCenterGetDarwinNotifyCenter(), CFNotificationName(name), nil, nil, true)
    }

    private static func deliver() {
        lock.lock()
        let handler = callback
        lock.unlock()
        handler?()
    }
}

@_cdecl("flow_like_native_set_action_callback")
public func flowLikeNativeSetActionCallback(_ callback: (@convention(c) () -> Void)?) {
    NativeActionNotifier.install(callback)
    Task { @MainActor in NativeActivityBridge.install() }
}

private enum NativeInactiveNotifier {
    static var observer: NSObjectProtocol?
}

@_cdecl("flow_like_native_set_inactive_callback")
public func flowLikeNativeSetInactiveCallback(_ callback: (@convention(c) () -> Void)?) {
    if let observer = NativeInactiveNotifier.observer { NotificationCenter.default.removeObserver(observer) }
    #if os(iOS)
    let name = UIApplication.didEnterBackgroundNotification
    #else
    let name = NSApplication.didResignActiveNotification
    #endif
    NativeInactiveNotifier.observer = NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { _ in
        callback?()
    }
}

@MainActor private enum NativeLocationBackgroundNotifier {
    static var observers: [NSObjectProtocol] = []
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@_cdecl("flow_like_native_set_location_background_callback")
public func flowLikeNativeSetLocationBackgroundCallback(_ callback: (@convention(c) () -> Void)?) {
    Task { @MainActor in
        NativeLocationBackgroundNotifier.observers.forEach(NotificationCenter.default.removeObserver)
        NativeLocationBackgroundNotifier.observers = nativeLocationHiddenNotifications().map { name in
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { _ in
                Task { @MainActor in if !nativeLocationScreenVisible() { callback?() } }
            }
        }
    }
}

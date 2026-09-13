import CoreSpotlight
import Foundation
import ObjectiveC
#if os(iOS)
import UIKit
#endif

enum NativeActivityBridge {
    @MainActor private static var installed: Set<ObjectIdentifier> = []

    static func origin(_ url: URL) -> String? {
        guard url.scheme == "https", let host = url.host else { return nil }
        return "https://\(host)" + (url.port.map { ":\($0)" } ?? "")
    }

    @MainActor static func install() {
        #if os(iOS)
        let className = "AppDelegate"
        #else
        let className = "TaoAppDelegateParent"
        #endif
        if let delegate = NSClassFromString(className) { install(delegate: delegate) }
        #if os(iOS)
        if let scene = NSClassFromString("TaoSceneDelegate") { install(scene: scene) }
        #endif
    }

    static func handle(_ activity: NSUserActivity, store: NativeStore = .shared) -> Bool {
        if NativeSystemIntegration.handleSearchActivity(activity, store: store) != nil { return true }
        guard activity.activityType == NativeSystemIntegration.activityType,
              let url = activity.webpageURL, url.path == "/use", url.user == nil, url.password == nil,
              url.fragment == nil, url.absoluteString.utf8.count <= 131_072,
              let parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let snapshot = store.catalog(),
              activity.userInfo?["scope"] as? String == snapshot.scope,
              let origin = origin(url), origin == snapshot.webOrigin else { return false }
        // Handoff URLs come from URLSearchParams. Unlike URLComponents.queryItems,
        // form decoding distinguishes a space (+) from a literal plus (%2B).
        let query = NativeAppRoute.queryParameters(encodedQuery: parts.percentEncodedQuery ?? "")
        guard ["id", "eventId", "route", "appQuery"].allSatisfy({ key in query.filter { $0.name == key }.count <= 1 }),
              let appId = query.first(where: { $0.name == "id" })?.value,
              snapshot.apps.contains(where: { $0.id == appId }) else { return false }
        let route = query.first(where: { $0.name == "route" })?.value
        let appQuery = query.first(where: { $0.name == "appQuery" })?.value
        let action: NativeAction
        if route != nil || appQuery != nil {
            let parameters = NativeAppRoute.queryParameters(encodedQuery: appQuery ?? "")
            guard (try? NativeAppRoute.validate(path: route, queryParams: parameters)) != nil else { return false }
            action = NativeAction(kind: "open_app", appId: appId, path: route, queryParams: parameters)
        } else if let eventId = query.first(where: { $0.name == "eventId" })?.value {
            guard let event = snapshot.events.first(where: {
                $0.appId == appId && $0.eventId == eventId && $0.action.kind == "open_event"
            }) else { return false }
            action = event.action
        } else {
            action = NativeAction(kind: "open_app", appId: appId)
        }
        return (try? store.enqueue(action, expectedScope: snapshot.scope)) != nil
    }

    @MainActor static func install(delegate: AnyClass, handle: @escaping (NSUserActivity) -> Bool = { handle($0) }) {
        guard installed.insert(ObjectIdentifier(delegate)).inserted else { return }
        let selector = NSSelectorFromString("application:continueUserActivity:restorationHandler:")
        typealias Handler = @convention(c) (AnyObject, Selector, AnyObject, NSUserActivity, AnyObject) -> Bool
        let original = class_getInstanceMethod(delegate, selector).map { unsafeBitCast(method_getImplementation($0), to: Handler.self) }
        let block: @convention(block) (AnyObject, AnyObject, NSUserActivity, AnyObject) -> Bool = { receiver, application, activity, restoration in
            if handle(activity) { return true }
            return original?(receiver, selector, application, activity, restoration) ?? false
        }
        let implementation = imp_implementationWithBlock(block)
        if let method = class_getInstanceMethod(delegate, selector) { method_setImplementation(method, implementation) }
        else { class_addMethod(delegate, selector, implementation, "B@:@@@") }

        let willSelector = NSSelectorFromString("application:willContinueUserActivityWithType:")
        typealias WillHandler = @convention(c) (AnyObject, Selector, AnyObject, NSString) -> Bool
        let originalWill = class_getInstanceMethod(delegate, willSelector).map { unsafeBitCast(method_getImplementation($0), to: WillHandler.self) }
        let willBlock: @convention(block) (AnyObject, AnyObject, NSString) -> Bool = { receiver, application, type in
            if type as String == CSSearchableItemActionType || type as String == NativeSystemIntegration.activityType { return true }
            return originalWill?(receiver, willSelector, application, type) ?? false
        }
        let willImplementation = imp_implementationWithBlock(willBlock)
        if let method = class_getInstanceMethod(delegate, willSelector) { method_setImplementation(method, willImplementation) }
        else { class_addMethod(delegate, willSelector, willImplementation, "B@:@@") }
        #if os(iOS)
        installSceneConfiguration(delegate: delegate)
        #endif
    }

    #if os(iOS)
    @MainActor private static func installSceneConfiguration(delegate: AnyClass) {
        let selector = NSSelectorFromString("application:configurationForConnectingSceneSession:options:")
        typealias Configuration = @convention(c) (AnyObject, Selector, AnyObject, UISceneSession, UIScene.ConnectionOptions) -> UISceneConfiguration
        guard let method = class_getInstanceMethod(delegate, selector) else { return }
        let original = unsafeBitCast(method_getImplementation(method), to: Configuration.self)
        let block: @convention(block) (AnyObject, AnyObject, UISceneSession, UIScene.ConnectionOptions) -> UISceneConfiguration = { receiver, application, session, options in
            let configuration = original(receiver, selector, application, session, options)
            // Tao registers this class lazily while creating the configuration.
            // Install before UIKit delivers the first scene's launch activities.
            if let scene = configuration.delegateClass { install(scene: scene) }
            return configuration
        }
        class_replaceMethod(delegate, selector, imp_implementationWithBlock(block), method_getTypeEncoding(method))
    }

    @MainActor private static func install(scene: AnyClass) {
        guard installed.insert(ObjectIdentifier(scene)).inserted else { return }
        let selector = NSSelectorFromString("scene:continueUserActivity:")
        typealias Handler = @convention(c) (AnyObject, Selector, AnyObject, NSUserActivity) -> Void
        let original = class_getInstanceMethod(scene, selector).map { unsafeBitCast(method_getImplementation($0), to: Handler.self) }
        let block: @convention(block) (AnyObject, AnyObject, NSUserActivity) -> Void = { receiver, scene, activity in
            if !handle(activity) { original?(receiver, selector, scene, activity) }
        }
        class_replaceMethod(scene, selector, imp_implementationWithBlock(block), "v@:@@")

        let connectSelector = NSSelectorFromString("scene:willConnectToSession:options:")
        typealias Connect = @convention(c) (AnyObject, Selector, AnyObject, AnyObject, UIScene.ConnectionOptions) -> Void
        let originalConnect = class_getInstanceMethod(scene, connectSelector).map { unsafeBitCast(method_getImplementation($0), to: Connect.self) }
        let connect: @convention(block) (AnyObject, AnyObject, AnyObject, UIScene.ConnectionOptions) -> Void = { receiver, scene, session, options in
            // A cold scene receives its launch activity here, before continueUserActivity.
            options.userActivities.forEach { _ = handle($0) }
            originalConnect?(receiver, connectSelector, scene, session, options)
        }
        class_replaceMethod(scene, connectSelector, imp_implementationWithBlock(connect), "v@:@@@")
    }
    #endif
}

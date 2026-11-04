import UIKit
import Tauri
import UserNotifications
import tauri_plugin_remote_push
import FlowLikeNative

class AppDelegate: TauriAppDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    NativeBotBackgroundRefresh.install()
    NativeBotNotifications.registerCategories()
    PushNotificationPlugin.configureFirebaseAppIfAvailable()
    UNUserNotificationCenter.current().delegate = self
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  override func application(
    _ application: UIApplication,
    didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
  ) {
    PushNotificationPlugin.applicationDidRegisterForRemoteNotifications(deviceToken: deviceToken)
  }

  override func application(
    _ application: UIApplication,
    didFailToRegisterForRemoteNotificationsWithError error: Error
  ) {
    PushNotificationPlugin.applicationDidFailToRegisterForRemoteNotifications(error: error)
    print("Failed to register for remote notifications: \(error.localizedDescription)")
  }

  override func application(
    _ application: UIApplication,
    didReceiveRemoteNotification userInfo: [AnyHashable: Any],
    fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void
  ) {
    if userInfo["bot_id"] != nil {
      NativeBotBackgroundRefresh.performFetch(userInfo: userInfo, completion: completionHandler)
      return
    }
    PushNotificationPlugin.applicationDidReceiveRemoteNotification(userInfo: userInfo)
    completionHandler(.newData)
  }

  override func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    willPresent notification: UNNotification,
    withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
  ) {
    if notification.request.trigger is UNPushNotificationTrigger {
      PushNotificationPlugin.applicationDidReceiveRemoteNotification(userInfo: notification.request.content.userInfo)
    }
    completionHandler([.banner, .sound, .badge])
  }

  override func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    didReceive response: UNNotificationResponse,
    withCompletionHandler completionHandler: @escaping () -> Void
  ) {
    if NativeBotNotifications.handleResponse(response) {
      completionHandler()
      return
    }
    if response.notification.request.trigger is UNPushNotificationTrigger {
      PushNotificationPlugin.applicationDidReceiveNotificationResponse(userInfo: response.notification.request.content.userInfo)
    }
    completionHandler()
  }
}

import UIKit
import Tauri
import UserNotifications
import tauri_plugin_remote_push

class AppDelegate: TauriAppDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    PushNotificationPlugin.configureFirebaseAppIfAvailable()
    UNUserNotificationCenter.current().delegate = self
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  override func application(
    _ application: UIApplication,
    didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
  ) {
    PushNotificationPlugin.handleToken(deviceToken)
  }

  override func application(
    _ application: UIApplication,
    didFailToRegisterForRemoteNotificationsWithError error: Error
  ) {
    PushNotificationPlugin.handleRegistrationFailure(error)
    print("Failed to register for remote notifications: \(error.localizedDescription)")
  }

  override func application(
    _ application: UIApplication,
    didReceiveRemoteNotification userInfo: [AnyHashable: Any],
    fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void
  ) {
    PushNotificationPlugin.handleNotification(userInfo)
    completionHandler(.newData)
  }

  override func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    willPresent notification: UNNotification,
    withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
  ) {
    PushNotificationPlugin.handleNotification(notification.request.content.userInfo)
    completionHandler([.banner, .sound, .badge])
  }

  override func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    didReceive response: UNNotificationResponse,
    withCompletionHandler completionHandler: @escaping () -> Void
  ) {
    PushNotificationPlugin.handleNotificationTapped(response.notification.request.content.userInfo)
    completionHandler()
  }
}
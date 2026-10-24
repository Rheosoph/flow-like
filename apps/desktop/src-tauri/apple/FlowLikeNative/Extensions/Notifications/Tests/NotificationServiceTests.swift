// Compile with NotificationService.swift and run on macOS to check image decoding and fallback behavior.
import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers
import UserNotifications

@main struct NotificationServiceChecks {
    static func main() throws {
        var checks = 0
        func check(_ result: @autoclosure () -> Bool, _ label: String) {
            if !result() { print("Failed:", label); exit(1) }
            checks += 1
        }
        check(NotificationService.imageURL(from: ["fcm_options": ["image": "https://example.com/signed?id=123"]])?.host == "example.com", "FCM extensionless image")
        check(NotificationService.imageURL(from: ["image": "https://example.com/image.png"]) != nil, "Custom image")
        check(NotificationService.imageURL(from: ["icon": "https://example.com/image.png"]) != nil, "Legacy icon")
        for value in ["http://example.com/image.png", "file:///tmp/icon.png", "data:image/png;base64,AAA", "https://user:pass@example.com/icon.png", "https://"] {
            check(NotificationService.imageURL(from: ["image": value]) == nil, "Reject \(value)")
        }
        check(NotificationService.thumbnail(from: Data()) == nil, "Empty image")
        check(NotificationService.thumbnail(from: Data("<svg></svg>".utf8)) == nil, "Invalid image")
        check(NotificationService.thumbnail(from: Data(repeating: 0, count: 5 * 1024 * 1024 + 1)) == nil, "Size bound")

        func plainContent(title: String, body: String) -> UNNotificationContent {
            let content = UNMutableNotificationContent()
            content.title = title
            content.body = body
            return content
        }
        func encodedImage(_ type: UTType, width: Int) -> Data {
            let context = CGContext(data: nil, width: width, height: 2, bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
            let bytes = NSMutableData()
            let destination = CGImageDestinationCreateWithData(bytes, type.identifier as CFString, 1, nil)!
            CGImageDestinationAddImage(destination, context.makeImage()!, nil)
            precondition(CGImageDestinationFinalize(destination))
            return bytes as Data
        }
        for type in [UTType.png, UTType.jpeg, UTType.gif] {
            let thumbnail = NotificationService.thumbnail(from: encodedImage(type, width: 2048))
            check(thumbnail != nil, "Decode \(type)")
            if let thumbnail {
                check(thumbnail.width <= 1024, "Bound decoded dimensions")
            }
        }
        if let attachment = NotificationService.attachment(from: encodedImage(.png, width: 32)) {
            check(FileManager.default.isReadableFile(atPath: attachment.url.path), "Keep attachment readable until delivery")
            check((try? Data(contentsOf: attachment.url).isEmpty) == false, "Keep attachment contents until delivery")
            try? FileManager.default.removeItem(at: attachment.url)
        } else {
            print("Attachment lifetime check skipped: OS attachment creation is unavailable in this process")
        }
        check(NotificationService.thumbnail(from: encodedImage(.png, width: 8193)) == nil, "Reject excessive source dimensions")

        check(NotificationService.senderName(from: ["app_name": "  Momo \n"]) == "Momo", "Trim sender name")
        check(NotificationService.senderName(from: ["app_name": "  "]) == nil, "Reject blank sender name")
        check(NotificationService.avatarURL(from: ["app_name": "Momo", "app_icon": "https://example.com/icon.webp?sig=1"]) != nil, "Sender avatar")
        check(NotificationService.avatarURL(from: ["app_icon": "https://example.com/icon.webp"]) == nil, "Avatar needs a sender")
        check(NotificationService.avatarURL(from: ["app_name": "Momo", "app_icon": "http://example.com/icon.webp"]) == nil, "Reject insecure avatar")
        check(NotificationService.messageBody(title: "Build finished", body: "All green", sender: "Momo") == "Build finished\nAll green", "Keep title in body")
        check(NotificationService.messageBody(title: "Momo", body: "All green", sender: "Momo") == "All green", "Drop title that repeats sender")
        check(NotificationService.messageBody(title: "Build finished", body: "", sender: "Momo") == "Build finished", "Title-only body")
        let avatar = NotificationService.thumbnail(from: encodedImage(.png, width: 1024), maxPixelSize: 256, types: NotificationService.avatarTypes)
        check(avatar.map { $0.width <= 256 } == true, "Bound avatar dimensions")
        check(NotificationService.avatarImage(from: encodedImage(.png, width: 64)) != nil, "Avatar image")
        let webP = NSMutableData()
        if let destination = CGImageDestinationCreateWithData(webP, UTType.webP.identifier as CFString, 1, nil) {
            let context = CGContext(data: nil, width: 8, height: 8, bitsPerComponent: 8, bytesPerRow: 32, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
            CGImageDestinationAddImage(destination, context.makeImage()!, nil)
            if CGImageDestinationFinalize(destination) {
                check(NotificationService.avatarImage(from: webP as Data) != nil, "WebP app icons become avatars")
                check(NotificationService.thumbnail(from: webP as Data) == nil, "WebP stays out of attachments")
            }
        } else {
            print("WebP avatar check skipped: this OS cannot encode WebP")
        }

        let message = UNMutableNotificationContent()
        message.title = "Build finished"
        message.body = "All green"
        message.userInfo = ["app_name": "Momo", "app_id": "app-1", "notification_id": "two"]
        let intent = NotificationService.messageIntent(for: message, avatar: nil)
        check(intent?.sender?.displayName == "Momo", "App is the sender")
        check(intent?.conversationIdentifier == "app-1", "One conversation per app")
        check(intent?.content == "Build finished\nAll green", "Intent keeps title and body")
        check(NotificationService.messageIntent(for: plainContent(title: "System", body: "Update"), avatar: nil) == nil, "Flow Like notifications have no sender")

        let senderService = NotificationService()
        var senderCallbacks = 0
        var senderResult: UNNotificationContent?
        senderService.didReceive(UNNotificationRequest(identifier: "two", content: message, trigger: nil)) { result in
            senderCallbacks += 1
            senderResult = result
        }
        let deadline = Date().addingTimeInterval(5)
        while senderCallbacks == 0 && Date() < deadline {
            RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        }
        senderService.serviceExtensionTimeWillExpire()
        senderService.serviceExtensionTimeWillExpire()
        check(senderCallbacks == 1, "Deliver sender notification once")
        check(senderResult?.userInfo["notification_id"] as? String == "two", "Sender notification keeps tap metadata")
        check(senderResult.map { !$0.body.isEmpty } == true, "Sender notification keeps its text")

        let service = NotificationService()
        let content = UNMutableNotificationContent()
        content.title = "Workflow finished"
        content.body = "Download unavailable"
        content.userInfo = ["image": "file:///tmp/icon.png", "notification_id": "one"]
        var callbacks = 0
        service.didReceive(UNNotificationRequest(identifier: "one", content: content, trigger: nil)) { result in
            callbacks += 1
            check(result.title == content.title && result.body == content.body, "Preserve fallback content")
            check(result.userInfo["notification_id"] as? String == "one", "Preserve tap metadata")
        }
        service.serviceExtensionTimeWillExpire()
        service.serviceExtensionTimeWillExpire()
        check(callbacks == 1, "Finish fallback once")
        print("Notification service: \(checks) checks passed")
    }
}

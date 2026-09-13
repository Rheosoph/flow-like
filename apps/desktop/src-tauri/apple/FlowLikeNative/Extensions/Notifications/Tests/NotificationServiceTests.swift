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

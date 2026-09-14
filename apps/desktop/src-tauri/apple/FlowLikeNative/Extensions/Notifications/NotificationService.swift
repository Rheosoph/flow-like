import Foundation
import ImageIO
import UniformTypeIdentifiers
import UserNotifications

final class NotificationService: UNNotificationServiceExtension, URLSessionDataDelegate, @unchecked Sendable {
    private static let maximumBytes = 5 * 1024 * 1024
    private let lock = NSLock()
    private var handler: ((UNNotificationContent) -> Void)?
    private var content: UNMutableNotificationContent?
    private var session: URLSession?
    private var imageData = Data()
    private var redirects = 0

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        guard let mutableContent = request.content.mutableCopy() as? UNMutableNotificationContent,
              let url = Self.imageURL(from: request.content.userInfo) else {
            contentHandler(request.content)
            return
        }

        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 15
        configuration.timeoutIntervalForResource = 20
        configuration.urlCache = nil
        configuration.httpCookieStorage = nil
        configuration.urlCredentialStorage = nil
        let queue = OperationQueue()
        queue.maxConcurrentOperationCount = 1
        let session = URLSession(configuration: configuration, delegate: self, delegateQueue: queue)

        lock.lock()
        handler = contentHandler
        content = mutableContent
        self.session = session
        imageData = Data()
        redirects = 0
        lock.unlock()
        session.dataTask(with: url).resume()
    }

    override func serviceExtensionTimeWillExpire() {
        finish()
    }

    static func imageURL(from userInfo: [AnyHashable: Any]) -> URL? {
        let fcmOptions = userInfo["fcm_options"] as? [String: Any]
        let candidates = [fcmOptions?["image"], userInfo["image"], userInfo["icon"]]
        return candidates.compactMap { $0 as? String }.compactMap { value in
            guard let url = URL(string: value), isHTTPS(url) else { return nil }
            return url
        }.first
    }

    private static func isHTTPS(_ url: URL) -> Bool {
        url.scheme?.lowercased() == "https" && url.host?.isEmpty == false
            && url.user == nil && url.password == nil
    }

    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest,
        completionHandler: @escaping (URLRequest?) -> Void
    ) {
        lock.lock()
        redirects += 1
        let allowed = handler != nil && redirects <= 3 && request.url.map(Self.isHTTPS) == true
        lock.unlock()
        completionHandler(allowed ? request : nil)
        if !allowed { finish() }
    }

    func urlSession(
        _ session: URLSession,
        dataTask: URLSessionDataTask,
        didReceive response: URLResponse,
        completionHandler: @escaping (URLSession.ResponseDisposition) -> Void
    ) {
        guard let http = response as? HTTPURLResponse,
              (200...299).contains(http.statusCode),
              response.url.map(Self.isHTTPS) == true,
              response.expectedContentLength <= Self.maximumBytes else {
            completionHandler(.cancel)
            finish()
            return
        }
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        lock.lock()
        let allowed = handler != nil && data.count <= Self.maximumBytes - imageData.count
        if allowed { imageData.append(data) }
        lock.unlock()
        if !allowed { finish() }
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        lock.lock()
        let data = handler != nil && error == nil ? imageData : nil
        imageData = Data()
        lock.unlock()
        finish(attachment: data.flatMap(Self.attachment))
    }

    static func thumbnail(from data: Data) -> CGImage? {
        guard !data.isEmpty, data.count <= maximumBytes,
              let source = CGImageSourceCreateWithData(data as CFData, nil),
              CGImageSourceGetStatus(source) == .statusComplete,
              let type = CGImageSourceGetType(source) as String?,
              [UTType.jpeg.identifier, UTType.png.identifier, UTType.gif.identifier].contains(type),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, width <= 8192, height <= 8192,
              width * height <= 20_000_000 else { return nil }

        // Decode a bounded first frame and write a real PNG, including for extensionless signed URLs.
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: 1024,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }

    static func attachment(from data: Data) -> UNNotificationAttachment? {
        guard let thumbnail = thumbnail(from: data) else { return nil }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            var attachmentCreated = false
            defer {
                if !attachmentCreated { try? FileManager.default.removeItem(at: directory) }
            }
            let file = directory.appendingPathComponent("image.png")
            guard let destination = CGImageDestinationCreateWithURL(
                file as CFURL, UTType.png.identifier as CFString, 1, nil
            ) else { return nil }
            CGImageDestinationAddImage(destination, thumbnail, nil)
            guard CGImageDestinationFinalize(destination) else { return nil }
            let attachment = try UNNotificationAttachment(
                identifier: "flow-like-image", url: file,
                options: [UNNotificationAttachmentOptionsTypeHintKey: UTType.png.identifier]
            )
            // iOS consumes the remote attachment after the extension returns its content.
            attachmentCreated = true
            return attachment
        } catch {
            return nil
        }
    }

    private func finish(attachment: UNNotificationAttachment? = nil) {
        lock.lock()
        guard let handler, let content else {
            lock.unlock()
            return
        }
        if let attachment { content.attachments.append(attachment) }
        let session = self.session
        self.handler = nil
        self.content = nil
        self.session = nil
        imageData = Data()
        lock.unlock()

        session?.invalidateAndCancel()
        handler(content)
    }
}

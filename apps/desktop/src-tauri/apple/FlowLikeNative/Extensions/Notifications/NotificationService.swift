import Foundation
import ImageIO
import Intents
import UniformTypeIdentifiers
import UserNotifications

final class NotificationService: UNNotificationServiceExtension, URLSessionDataDelegate, @unchecked Sendable {
    enum Media { case image, avatar }

    private struct Download {
        let media: Media
        var data = Data()
        var redirects = 0
    }

    private static let maximumBytes = 5 * 1024 * 1024
    static let attachmentTypes = [UTType.jpeg, .png, .gif].map(\.identifier)
    static let avatarTypes = attachmentTypes + [UTType.webP.identifier, UTType.heic.identifier]
    private let lock = NSLock()
    private var handler: ((UNNotificationContent) -> Void)?
    private var content: UNMutableNotificationContent?
    private var session: URLSession?
    private var downloads: [Int: Download] = [:]
    private var media: [Media: Data] = [:]
    private var finishing = false

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        let userInfo = request.content.userInfo
        let sources = [(Media.image, Self.imageURL(from: userInfo)), (.avatar, Self.avatarURL(from: userInfo))]
            .compactMap { media, url in url.map { (media, $0) } }
        guard let mutableContent = request.content.mutableCopy() as? UNMutableNotificationContent,
              !sources.isEmpty || Self.senderName(from: userInfo) != nil else {
            contentHandler(request.content)
            return
        }

        lock.lock()
        handler = contentHandler
        content = mutableContent
        downloads = [:]
        media = [:]
        finishing = false
        lock.unlock()
        guard !sources.isEmpty else {
            finish()
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
        guard handler != nil else {
            lock.unlock()
            session.invalidateAndCancel()
            return
        }
        self.session = session
        let tasks = sources.map { media, url in
            let task = session.dataTask(with: url)
            downloads[task.taskIdentifier] = Download(media: media)
            return task
        }
        lock.unlock()
        tasks.forEach { $0.resume() }
    }

    override func serviceExtensionTimeWillExpire() {
        deliver()
    }

    static func imageURL(from userInfo: [AnyHashable: Any]) -> URL? {
        let fcmOptions = userInfo["fcm_options"] as? [String: Any]
        let candidates = [fcmOptions?["image"], userInfo["image"], userInfo["icon"]]
        return candidates.compactMap { $0 as? String }.compactMap { value in
            guard let url = URL(string: value), isHTTPS(url) else { return nil }
            return url
        }.first
    }

    static func senderName(from userInfo: [AnyHashable: Any]) -> String? {
        let name = (userInfo["app_name"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines)
        return name?.isEmpty == false ? name : nil
    }

    static func avatarURL(from userInfo: [AnyHashable: Any]) -> URL? {
        guard senderName(from: userInfo) != nil,
              let value = userInfo["app_icon"] as? String,
              let url = URL(string: value), isHTTPS(url) else { return nil }
        return url
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
        downloads[task.taskIdentifier]?.redirects += 1
        let redirects = downloads[task.taskIdentifier]?.redirects ?? .max
        lock.unlock()
        // A refused redirect completes with the 3xx response, which the response check rejects.
        completionHandler(redirects <= 3 && request.url.map(Self.isHTTPS) == true ? request : nil)
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
            return
        }
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        lock.lock()
        let fits = downloads[dataTask.taskIdentifier].map { data.count <= Self.maximumBytes - $0.data.count } ?? false
        if fits { downloads[dataTask.taskIdentifier]?.data.append(data) }
        lock.unlock()
        if !fits { dataTask.cancel() }
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        lock.lock()
        if let download = downloads.removeValue(forKey: task.taskIdentifier), error == nil {
            media[download.media] = download.data
        }
        let done = downloads.isEmpty
        lock.unlock()
        if done { finish() }
    }

    static func thumbnail(
        from data: Data, maxPixelSize: Int = 1024, types: [String] = attachmentTypes
    ) -> CGImage? {
        guard !data.isEmpty, data.count <= maximumBytes,
              let source = CGImageSourceCreateWithData(data as CFData, nil),
              CGImageSourceGetStatus(source) == .statusComplete,
              let type = CGImageSourceGetType(source) as String?,
              types.contains(type),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, width <= 8192, height <= 8192,
              width * height <= 20_000_000 else { return nil }

        // Decode a bounded first frame and write a real PNG, including for extensionless signed URLs.
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }

    static func pngData(from image: CGImage) -> Data? {
        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(
            data, UTType.png.identifier as CFString, 1, nil
        ) else { return nil }
        CGImageDestinationAddImage(destination, image, nil)
        return CGImageDestinationFinalize(destination) ? data as Data : nil
    }

    static func attachment(from data: Data) -> UNNotificationAttachment? {
        guard let png = thumbnail(from: data).flatMap(pngData) else { return nil }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            var attachmentCreated = false
            defer {
                if !attachmentCreated { try? FileManager.default.removeItem(at: directory) }
            }
            let file = directory.appendingPathComponent("image.png")
            try png.write(to: file)
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

    static func avatarImage(from data: Data) -> INImage? {
        thumbnail(from: data, maxPixelSize: 256, types: avatarTypes)
            .flatMap(pngData)
            .map { INImage(imageData: $0) }
    }

    /// iOS titles a communication notification with its sender, so the original title moves into the body.
    static func messageBody(title: String, body: String, sender: String) -> String {
        [title == sender ? "" : title, body].filter { !$0.isEmpty }.joined(separator: "\n")
    }

    /// The app that sent the notification becomes a one-to-one conversation partner.
    static func messageIntent(for content: UNNotificationContent, avatar: Data?) -> INSendMessageIntent? {
        guard let name = senderName(from: content.userInfo) else { return nil }
        let conversation = (content.userInfo["app_id"] as? String) ?? name
        let sender = INPerson(
            personHandle: INPersonHandle(value: "flow-like-app:\(conversation)", type: .unknown),
            nameComponents: nil,
            displayName: name,
            image: avatar.flatMap(avatarImage),
            contactIdentifier: nil,
            customIdentifier: conversation
        )
        return INSendMessageIntent(
            recipients: nil,
            outgoingMessageType: .outgoingMessageText,
            content: messageBody(title: content.title, body: content.body, sender: name),
            speakableGroupName: nil,
            conversationIdentifier: conversation,
            serviceName: nil,
            sender: sender,
            attachments: nil
        )
    }

    static func communicationContent(
        from content: UNNotificationContent, intent: INSendMessageIntent
    ) -> UNNotificationContent? {
        guard let message = content.mutableCopy() as? UNMutableNotificationContent else { return nil }
        message.body = intent.content ?? content.body
        return try? message.updating(from: intent)
    }

    private func finish() {
        lock.lock()
        guard handler != nil, !finishing, let content else {
            lock.unlock()
            return
        }
        finishing = true
        let media = self.media
        let session = self.session
        self.session = nil
        lock.unlock()
        session?.invalidateAndCancel()

        if let attachment = media[.image].flatMap(Self.attachment) {
            lock.lock()
            content.attachments.append(attachment)
            lock.unlock()
        }
        guard let intent = Self.messageIntent(for: content, avatar: media[.avatar]) else {
            deliver()
            return
        }
        let interaction = INInteraction(intent: intent, response: nil)
        interaction.direction = .incoming
        interaction.donate { [self] error in
            deliver(error == nil ? Self.communicationContent(from: content, intent: intent) : nil)
        }
    }

    /// Hands iOS the best content so far exactly once: the communication content when it is
    /// ready, otherwise the original notification with any finished attachment.
    private func deliver(_ replacement: UNNotificationContent? = nil) {
        lock.lock()
        let handler = self.handler
        let delivered = replacement ?? (content?.copy() as? UNNotificationContent)
        let session = self.session
        self.handler = nil
        self.content = nil
        self.session = nil
        downloads = [:]
        media = [:]
        lock.unlock()

        session?.invalidateAndCancel()
        if let handler, let delivered { handler(delivered) }
    }
}

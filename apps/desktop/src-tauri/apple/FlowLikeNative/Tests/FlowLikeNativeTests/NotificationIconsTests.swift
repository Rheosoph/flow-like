import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers
import Testing
@testable import FlowLikeNative

@Test func notificationIconsNormalizeImagesAndPreserveTemplateTint() throws {
    let directory = notificationIconDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    let png = try notificationPNG(width: 256, height: 128)
    let published = try store.publish(notificationSnapshot(icons: [["png": png.base64EncodedString(), "template": true]]))
    let item = try #require(published.sections.first?.items.first)
    let stored = try #require(store.snapshot()?.sections.first?.items.first?.icon)
    #expect(stored == item.icon)
    guard case let .image(image, template) = store.notificationIcon(for: item, scope: "alice") else {
        Issue.record("Expected a custom notification image")
        return
    }
    #expect(template)
    #expect(image.count <= 32_768)
    let source = try #require(CGImageSourceCreateWithData(image as CFData, nil))
    let properties = try #require(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
    #expect(properties[kCGImagePropertyPixelWidth] as? Int == 128)
    #expect(properties[kCGImagePropertyPixelHeight] as? Int == 64)
    let exif = properties[kCGImagePropertyExifDictionary] as? [CFString: Any]
    #expect(exif?[kCGImagePropertyExifUserComment] == nil)
}

@Test func notificationIconsDiscardMalformedDecorationsWithoutLosingRows() throws {
    let directory = notificationIconDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    let bad: [Any] = [
        "https://example.com/private.png", 17, ["png": true], ["text": ["unexpected"]],
        ["png": "invalid"], ["png": Data("https://example.com/private.png".utf8).base64EncodedString()],
        ["png": Data([137, 80, 78, 71, 13, 10, 26, 10]).base64EncodedString()],
        ["png": Data(repeating: 0, count: 32_769).base64EncodedString()],
        ["png": try notificationPNG(width: 1025, height: 1).base64EncodedString()],
        ["text": "https://example.com"], ["text": "🔔\n"], ["text": String(repeating: "🔔", count: 9)],
        ["text": String(repeating: "👨‍👩‍👧‍👦", count: 3)],
    ]
    let snapshot = try store.publish(notificationSnapshot(icons: bad))
    #expect(snapshot.sections[0].items.count == bad.count)
    #expect(snapshot.sections[0].items.allSatisfy { $0.icon == nil })
    #expect(store.snapshot()?.sections[0].items.allSatisfy { $0.icon == nil } == true)
    #expect(store.notificationIcon(for: snapshot.sections[0].items[0], scope: "alice") == .flowLike)
}

@Test func notificationIconsKeepEmojiAndPreferValidImages() throws {
    let directory = notificationIconDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    let snapshot = try store.publish(notificationSnapshot(icons: [
        ["text": "👨‍👩‍👧‍👦", "template": true], ["png": "broken", "text": "🇩🇪"],
        ["png": try notificationPNG().base64EncodedString(), "text": "🔔", "template": false],
        NSNull(), ["text": "1️⃣"],
    ]))
    let items = snapshot.sections[0].items
    #expect(items[0].icon?.template == nil)
    #expect(store.notificationIcon(for: items[0], scope: "alice") == .text("👨‍👩‍👧‍👦"))
    #expect(store.notificationIcon(for: items[1], scope: "alice") == .text("🇩🇪"))
    guard case let .image(_, template) = store.notificationIcon(for: items[2], scope: "alice") else {
        Issue.record("A valid custom image takes precedence over text")
        return
    }
    #expect(!template)
    #expect(store.notificationIcon(for: items[3], scope: "alice") == .flowLike)
    #expect(store.notificationIcon(for: items[4], scope: "alice") == .text("1️⃣"))
}

@Test func notificationIconsUseScopedSourceArtworkAndClearAfterAccountChanges() throws {
    let directory = notificationIconDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    let snapshot = try store.publish(notificationSnapshot(icons: [NSNull(), ["text": "✅"]]))
    let png = try notificationPNG()
    let publication = try JSONSerialization.data(withJSONObject: [
        "scope": "alice", "icons": [["appId": "app", "data": png.base64EncodedString()]],
    ])
    try store.publishAppIcons(publication)
    let item = snapshot.sections[0].items[0]
    #expect(store.notificationIcon(for: item, scope: "alice") == .image(try NativeAppIconStore.normalize(png), template: false))
    #expect(store.notificationIcon(for: item, scope: "bob") == .flowLike)
    var unknown = item
    unknown.action.appId = "unknown"
    #expect(store.notificationIcon(for: unknown, scope: "alice") == .flowLike)
    #expect(store.notificationIcon(for: snapshot.sections[0].items[1], scope: "alice") == .text("✅"))
    try store.publish(notificationSnapshot(scope: "bob", icons: [NSNull()]))
    #expect(store.notificationIcon(for: item, scope: "alice") == .flowLike)
    #expect(store.notificationIcon(for: snapshot.sections[0].items[1], scope: "alice") == .flowLike)
    #expect(store.notificationIcon(for: item, scope: "bob") == .flowLike)
    try store.clear()
    #expect(store.notificationIcon(for: item, scope: "bob") == .flowLike)
}

private func notificationIconDirectory() -> URL {
    FileManager.default.temporaryDirectory.appendingPathComponent("flow-like-notification-icons-" + UUID().uuidString)
}

private func notificationSnapshot(scope: String = "alice", icons: [Any]) throws -> Data {
    try JSONSerialization.data(withJSONObject: [
        "version": 1, "scope": scope,
        "generatedAt": ISO8601DateFormatter().string(from: Date()),
        "expiresAt": ISO8601DateFormatter().string(from: Date().addingTimeInterval(300)),
        "sections": [["kind": "inbox", "title": "Notifications", "state": "ready", "items": icons.enumerated().map { index, icon in
            ["id": "notification-\(index)", "title": "Report ready", "icon": icon,
             "action": ["kind": "open_inbox", "appId": "app"]] as [String: Any]
        }]],
        "events": [], "apps": [["id": "app", "title": "Example", "spotlightEligible": true]],
    ])
}

private func notificationPNG(width: Int = 32, height: Int = 32) throws -> Data {
    let context = try #require(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
                                        bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
    context.setFillColor(CGColor(red: 0.1, green: 0.5, blue: 0.8, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: width, height: height))
    let image = try #require(context.makeImage())
    let data = NSMutableData()
    let destination = try #require(CGImageDestinationCreateWithData(data, UTType.png.identifier as CFString, 1, nil))
    CGImageDestinationAddImage(destination, image, [kCGImagePropertyExifDictionary: [kCGImagePropertyExifUserComment: "Private source comment"]] as CFDictionary)
    #expect(CGImageDestinationFinalize(destination))
    return data as Data
}

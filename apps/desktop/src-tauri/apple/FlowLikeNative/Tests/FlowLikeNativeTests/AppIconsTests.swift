import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers
import Testing
@testable import FlowLikeNative

@Test func appIconsNormalizeAndStripMetadata() throws {
    let original = try iconPNG(width: 256, height: 192)
    let normalized = try NativeAppIconStore.normalize(original)
    let source = try #require(CGImageSourceCreateWithData(normalized as CFData, nil))
    let properties = try #require(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
    #expect(properties[kCGImagePropertyPixelWidth] as? Int == 128)
    #expect(properties[kCGImagePropertyPixelHeight] as? Int == 96)
    #expect(normalized.count <= NativeAppIconStore.maxImageBytes)
    let exif = properties[kCGImagePropertyExifDictionary] as? [CFString: Any]
    #expect(exif?[kCGImagePropertyExifUserComment] == nil)
}

@Test func appIconsAreBoundToCurrentAccountAndCatalog() throws {
    let directory = iconDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    try store.publish(iconSnapshot(scope: "alice"))
    try store.publishAppIcons(iconPublication(scope: "alice", appId: "app", png: iconPNG()))
    #expect(store.appIconData(scope: "alice", appId: "app") != nil)
    #expect(store.appIconData(scope: "bob", appId: "app") == nil)
    #expect(store.appIconData(scope: "alice", appId: "removed") == nil)
    try store.publish(iconSnapshot(scope: "bob"))
    #expect(store.appIconData(scope: "alice", appId: "app") == nil)
    #expect(store.appIconData(scope: "bob", appId: "app") == nil)
    #expect(throws: NativeIntegrationError.invalidAction) {
        try store.publishAppIcons(iconPublication(scope: "alice", appId: "app", png: iconPNG()))
    }
}

@Test func appIconsAreRemovedWithAppOrExplicitNullAndSignOut() throws {
    let directory = iconDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    try store.publish(iconSnapshot())
    let publication = try iconPublication(appId: "app", png: iconPNG())
    try store.publishAppIcons(publication)
    try store.publishAppIcons(iconPublication(appId: "app", png: nil))
    #expect(store.appIconData(scope: "alice", appId: "app") == nil)
    try store.publishAppIcons(publication)
    try store.publish(iconSnapshot(appIds: []))
    #expect(store.appIconData(scope: "alice", appId: "app") == nil)
    try store.publish(iconSnapshot())
    #expect(store.appIconData(scope: "alice", appId: "app") == nil)
    try store.publishAppIcons(publication)
    try store.clear()
    #expect(!FileManager.default.fileExists(atPath: directory.appendingPathComponent("AppIcons").path))
}

@Test func appIconsRejectInvalidImagesAndUnknownAppsBeforeChangingCache() throws {
    let directory = iconDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    try store.publish(iconSnapshot())
    try store.publishAppIcons(iconPublication(appId: "app", png: iconPNG()))
    let previous = store.appIconData(scope: "alice", appId: "app")
    for data in [Data("https://example.com/private.png".utf8), Data([137, 80, 78, 71, 13, 10, 26, 10]), Data(repeating: 0, count: 32_769)] {
        #expect(throws: (any Error).self) { try store.publishAppIcons(iconPublication(appId: "app", png: data)) }
        #expect(store.appIconData(scope: "alice", appId: "app") == previous)
    }
    #expect(throws: NativeIntegrationError.invalidAction) {
        try store.publishAppIcons(iconPublication(appId: "../foreign", png: iconPNG()))
    }
    #expect(throws: NativeIntegrationError.oversized) {
        try store.publishAppIcons(Data(repeating: 0, count: 1_048_577))
    }
}

@Test func appIconsRejectManifestPathsAndSymbolicLinks() throws {
    let directory = iconDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    try store.publish(iconSnapshot())
    try store.publishAppIcons(iconPublication(appId: "app", png: iconPNG()))
    let folder = directory.appendingPathComponent("AppIcons")
    let file = try #require(FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil).first { $0.pathExtension == "png" })
    let outside = directory.appendingPathComponent("outside.png")
    try iconPNG().write(to: outside)
    try FileManager.default.removeItem(at: file)
    try FileManager.default.createSymbolicLink(at: file, withDestinationURL: outside)
    #expect(store.appIconData(scope: "alice", appId: "app") == nil)
    let malicious = try JSONSerialization.data(withJSONObject: ["scope": "alice", "icons": ["app": "../outside.png"]])
    try malicious.write(to: folder.appendingPathComponent("manifest.json"))
    #expect(store.appIconData(scope: "alice", appId: "app") == nil)
}

@Test func appIconsPruneToTheSameBoundedCatalogSelectionAsFrontend() throws {
    let directory = iconDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = NativeStore(directory: directory)
    let original = (0..<128).map { "app-\($0)" }
    try store.publish(iconSnapshot(appIds: original + ["new-app"]))
    let png = try iconPNG().base64EncodedString()
    let publication = try JSONSerialization.data(withJSONObject: [
        "scope": "alice", "icons": original.map { ["appId": $0, "data": png] },
    ])
    try store.publishAppIcons(publication)
    try store.publish(iconSnapshot(appIds: ["new-app"] + original))
    try store.publishAppIcons(iconPublication(appId: "new-app", png: iconPNG()))
    #expect(store.appIconData(scope: "alice", appId: "new-app") != nil)
    #expect(store.appIconData(scope: "alice", appId: "app-127") == nil)
    #expect(store.appIconData(scope: "alice", appId: "app-0") != nil)
}

private func iconDirectory() -> URL { FileManager.default.temporaryDirectory.appendingPathComponent("flow-like-icons-" + UUID().uuidString) }
private func iconPNG(width: Int = 32, height: Int = 32) throws -> Data {
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
private func iconPublication(scope: String = "alice", appId: String, png: Data?) throws -> Data {
    try JSONSerialization.data(withJSONObject: ["scope": scope, "icons": [["appId": appId, "data": png?.base64EncodedString() as Any? ?? NSNull()]]])
}
private func iconSnapshot(scope: String = "alice", appIds: [String] = ["app"]) throws -> Data {
    try JSONSerialization.data(withJSONObject: [
        "version": 1, "scope": scope,
        "generatedAt": ISO8601DateFormatter().string(from: Date()),
        "expiresAt": ISO8601DateFormatter().string(from: Date().addingTimeInterval(300)),
        "sections": [], "events": [],
        "apps": appIds.map { ["id": $0, "title": "Example", "spotlightEligible": true] },
    ])
}

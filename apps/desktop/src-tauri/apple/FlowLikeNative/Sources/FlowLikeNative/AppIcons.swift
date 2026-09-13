import Foundation
import CryptoKit
import ImageIO
import UniformTypeIdentifiers
import WidgetKit

// Callers serialize this cache with NativeStore's account and snapshot lock.
struct NativeAppIconStore {
    static let maxImageBytes = 32_768
    static let maxBatchBytes = 1_048_576
    static let maxIcons = 128
    private let directory: URL
    private var folder: URL { directory.appendingPathComponent("AppIcons", isDirectory: true) }
    private var manifestURL: URL { folder.appendingPathComponent("manifest.json") }

    init(directory: URL) { self.directory = directory }

    private struct Publication: Decodable {
        var scope: String
        var icons: [Icon]
        struct Icon: Decodable { var appId: String; var data: String? }
    }
    private struct Manifest: Codable {
        var scope: String
        var icons: [String: String]
    }

    static func catalogAppIds(_ catalog: NativeSnapshot) -> Set<String> {
        var ids = Set<String>()
        for app in catalog.apps where app.id.utf8.count <= 512 {
            ids.insert(app.id)
            if ids.count == maxIcons { break }
        }
        return ids
    }

    func publish(_ data: Data, catalog: NativeSnapshot) throws {
        guard data.count <= Self.maxBatchBytes else { throw NativeIntegrationError.oversized }
        let publication = try JSONDecoder().decode(Publication.self, from: data)
        guard publication.scope == catalog.scope, !publication.scope.isEmpty else {
            throw NativeIntegrationError.invalidAction
        }
        guard publication.icons.count <= Self.maxIcons else { throw NativeIntegrationError.oversized }
        let allowed = Self.catalogAppIds(catalog)
        var updates: [(String, Data?)] = []
        var seen = Set<String>()
        for icon in publication.icons {
            guard allowed.contains(icon.appId), icon.appId.utf8.count <= 512,
                  seen.insert(icon.appId).inserted else { throw NativeIntegrationError.invalidAction }
            if let encoded = icon.data {
                guard encoded.utf8.count <= 43_692,
                      let png = Data(base64Encoded: encoded), png.count <= Self.maxImageBytes else {
                    throw NativeIntegrationError.oversized
                }
                updates.append((icon.appId, try Self.normalize(png)))
            } else { updates.append((icon.appId, nil)) }
        }
        try prune(scope: catalog.scope, appIds: allowed)
        var manifest = readManifest() ?? Manifest(scope: catalog.scope, icons: [:])
        let resultingIds = updates.reduce(into: Set(manifest.icons.keys)) { ids, update in
            if update.1 == nil { ids.remove(update.0) } else { ids.insert(update.0) }
        }
        guard resultingIds.count <= Self.maxIcons else { throw NativeIntegrationError.oversized }
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        for (appId, png) in updates {
            if let png {
                let digest = SHA256.hash(data: Data((catalog.scope + "\0" + appId + "\0").utf8) + png)
                    .map { String(format: "%02x", $0) }.joined()
                let name = digest + ".png"
                try png.write(to: folder.appendingPathComponent(name), options: .atomic)
                manifest.icons[appId] = name
            } else { manifest.icons.removeValue(forKey: appId) }
        }
        try write(manifest)
    }

    func data(scope: String, appId: String, catalog: NativeSnapshot?) -> Data? {
        guard let catalog, catalog.scope == scope, catalog.apps.contains(where: { $0.id == appId }),
              let manifest = readManifest(), manifest.scope == scope,
              let name = manifest.icons[appId], Self.validName(name),
              let data = boundedFile(folder.appendingPathComponent(name), maximum: Self.maxImageBytes),
              Self.isPNG(data), let source = CGImageSourceCreateWithData(data as CFData, nil),
              CGImageSourceGetStatus(source) == .statusComplete else { return nil }
        return data
    }

    func prune(scope: String, appIds: Set<String>) throws {
        guard var manifest = readManifest(), manifest.scope == scope else {
            try clear()
            return
        }
        manifest.icons = manifest.icons.filter { appIds.contains($0.key) && Self.validName($0.value) }
        try write(manifest)
    }

    func clear() throws {
        if FileManager.default.fileExists(atPath: folder.path) { try FileManager.default.removeItem(at: folder) }
    }

    private func readManifest() -> Manifest? {
        guard let data = boundedFile(manifestURL, maximum: 131_072),
              let manifest = try? JSONDecoder().decode(Manifest.self, from: data),
              manifest.icons.count <= Self.maxIcons else { return nil }
        return manifest
    }

    private func write(_ manifest: Manifest) throws {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try JSONEncoder().encode(manifest).write(to: manifestURL, options: .atomic)
        let retained = Set(manifest.icons.values).union(["manifest.json"])
        for file in try FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)
            where !retained.contains(file.lastPathComponent) {
            try FileManager.default.removeItem(at: file)
        }
        #if os(iOS)
        for name in retained {
            let file = folder.appendingPathComponent(name)
            if FileManager.default.fileExists(atPath: file.path) {
                try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
                                                     ofItemAtPath: file.path)
            }
        }
        #endif
    }

    private func boundedFile(_ url: URL, maximum: Int) -> Data? {
        guard let values = try? url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey]),
              values.isRegularFile == true, values.isSymbolicLink != true,
              let size = values.fileSize, size <= maximum,
              let data = try? Data(contentsOf: url), data.count <= maximum else { return nil }
        return data
    }

    private static func validName(_ name: String) -> Bool {
        name.count == 68 && name.hasSuffix(".png") && name.prefix(64).allSatisfy { $0.isHexDigit && !$0.isUppercase }
    }

    private static func isPNG(_ data: Data) -> Bool {
        data.starts(with: [137, 80, 78, 71, 13, 10, 26, 10])
    }

    static func normalize(_ data: Data) throws -> Data {
        guard isPNG(data), let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
              CGImageSourceGetCount(source) == 1, CGImageSourceGetStatus(source) == .statusComplete,
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, width <= 1024, height <= 1024,
              let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceThumbnailMaxPixelSize: 128,
              ] as CFDictionary) else { throw NativeIntegrationError.invalidAction }
        let encoded = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(encoded, UTType.png.identifier as CFString, 1, nil) else {
            throw NativeIntegrationError.invalidAction
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination), encoded.length <= maxImageBytes else { throw NativeIntegrationError.oversized }
        return encoded as Data
    }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@_cdecl("flow_like_native_publish_app_icons")
public func flowLikeNativePublishAppIcons(_ json: UnsafePointer<CChar>?) -> Int32 {
    guard let json else { return -1 }
    do {
        try NativeStore.shared.publishAppIcons(Data(String(cString: json).utf8))
        WidgetCenter.shared.reloadAllTimelines()
        return 0
    } catch { return -2 }
}

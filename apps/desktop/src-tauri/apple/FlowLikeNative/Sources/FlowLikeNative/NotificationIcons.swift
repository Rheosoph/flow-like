import Foundation

public struct NativeNotificationIcon: Codable, Sendable, Equatable {
    public var png: String?
    public var text: String?
    public var template: Bool?

    public init(png: String? = nil, text: String? = nil, template: Bool? = nil) {
        self.png = png
        self.text = text
        self.template = template
    }

    private enum CodingKeys: String, CodingKey { case png, text, template }

    // A malformed decoration must not hide an otherwise usable notification.
    public init(from decoder: Decoder) throws {
        let values = try? decoder.container(keyedBy: CodingKeys.self)
        png = try? values?.decodeIfPresent(String.self, forKey: .png)
        text = try? values?.decodeIfPresent(String.self, forKey: .text)
        template = try? values?.decodeIfPresent(Bool.self, forKey: .template)
    }

    func normalized() -> NativeNotificationIcon? {
        if let png, png.utf8.count <= 43_692,
           let bytes = Data(base64Encoded: png), bytes.count <= NativeAppIconStore.maxImageBytes,
           let image = try? NativeAppIconStore.normalize(bytes) {
            return NativeNotificationIcon(png: image.base64EncodedString(), template: template == true ? true : nil)
        }
        guard let text, !text.isEmpty, text.count <= 8, text.utf8.count <= 64,
              text.allSatisfy({ character in
                  (character.unicodeScalars.contains { $0.value > 127 && $0.properties.isEmoji } ||
                   character.unicodeScalars.contains { $0.value == 0x20E3 }) &&
                  !character.unicodeScalars.contains { $0.properties.generalCategory == .control || $0.properties.isWhitespace }
              }) else { return nil }
        return NativeNotificationIcon(text: text)
    }
}

public enum NativeNotificationIconContent: Sendable, Equatable {
    case image(Data, template: Bool)
    case text(String)
    case flowLike
}

extension NativeStore {
    public func notificationIcon(for item: NativeItem, scope: String) -> NativeNotificationIconContent {
        guard !scope.isEmpty, persistedScope() == scope else { return .flowLike }
        if let icon = item.icon?.normalized() {
            if let png = icon.png, let data = Data(base64Encoded: png) {
                return .image(data, template: icon.template == true)
            }
            if let text = icon.text { return .text(text) }
        }
        if let appId = item.action.appId, let data = appIconData(scope: scope, appId: appId) {
            return .image(data, template: false)
        }
        return .flowLike
    }
}

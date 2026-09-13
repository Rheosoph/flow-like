import Foundation
import UniformTypeIdentifiers
#if os(iOS)
import UIKit
#else
import AppKit
#endif

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@_cdecl("flow_like_native_publish_snapshot")
public func flowLikeNativePublishSnapshot(_ json: UnsafePointer<CChar>?) -> Int32 {
    guard let json else { return -1 }
    do {
        let previousScope = NativeStore.shared.persistedScope()
        let snapshot = try NativeStore.shared.publish(Data(String(cString: json).utf8))
        if previousScope != snapshot.scope { nativeGeofenceScopeChanged() }
        Task { @MainActor in
            NativeSystemIntegration.refresh()
        }
        return 0
    } catch { return -2 }
}

@available(iOSApplicationExtension, unavailable)
@available(macOSApplicationExtension, unavailable)
@_cdecl("flow_like_native_clear_snapshot")
public func flowLikeNativeClearSnapshot() -> Int32 {
    do {
        try NativeStore.shared.clear()
        nativeGeofenceScopeChanged()
        Task { @MainActor in
            NativeSystemIntegration.refresh()
        }
        return 0
    } catch { return -2 }
}

@_cdecl("flow_like_native_take_actions")
public func flowLikeNativeTakeActions() -> UnsafeMutablePointer<CChar>? {
    guard let actions = try? NativeStore.shared.takeActions(), let data = try? JSONEncoder().encode(actions),
          let json = String(data: data, encoding: .utf8) else { return strdup("[]") }
    return strdup(json)
}

@_cdecl("flow_like_native_pending_actions")
public func flowLikeNativePendingActions() -> UnsafeMutablePointer<CChar>? {
    guard let actions = try? NativeStore.shared.pendingActions(),
          let data = try? JSONEncoder().encode(actions), let json = String(data: data, encoding: .utf8) else { return nil }
    return strdup(json)
}

@_cdecl("flow_like_native_ack_action")
public func flowLikeNativeAcknowledgeAction(_ id: UnsafePointer<CChar>?, _ scope: UnsafePointer<CChar>?) -> Int32 {
    guard let id, let scope else { return -1 }
    do {
        try NativeStore.shared.acknowledgeAction(id: String(cString: id), scope: String(cString: scope))
        return 0
    } catch { return -2 }
}

@_cdecl("flow_like_native_free_string")
public func flowLikeNativeFreeString(_ string: UnsafeMutablePointer<CChar>?) { free(string) }

@_cdecl("flow_like_native_shared_files_directory")
public func flowLikeNativeSharedFilesDirectory() -> UnsafeMutablePointer<CChar>? {
    guard let directory = try? NativeStore.shared.sharedFilesDirectory() else { return nil }
    return strdup(directory.path)
}

private struct ClipboardRequest: Decodable {
    var format: String
    var text: String?
    var html: String?
    var imageBase64: String?
    var localOnly: Bool?
    var expiresAt: Double?
}

@_cdecl("flow_like_native_write_clipboard")
public func flowLikeNativeWriteClipboard(_ json: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    guard let json, let request = try? JSONDecoder().decode(ClipboardRequest.self, from: Data(String(cString: json).utf8)) else {
        return clipboardError("invalid_input", "Invalid clipboard content.")
    }
    // UIKit and AppKit pasteboards are only touched on the main thread.
    if Thread.isMainThread { return writeClipboard(request) }
    return DispatchQueue.main.sync { writeClipboard(request) }
}

private func writeClipboard(_ request: ClipboardRequest) -> UnsafeMutablePointer<CChar>? {
    var item: [String: Any] = [:]
    switch request.format {
    case "text":
        guard let text = request.text else { return clipboardError("invalid_input", "Text is required.") }
        item[UTType.utf8PlainText.identifier] = text
    case "html":
        guard let html = request.html else { return clipboardError("invalid_input", "HTML is required.") }
        item[UTType.html.identifier] = Data(html.utf8)
        if let text = request.text { item[UTType.utf8PlainText.identifier] = text }
    case "image":
        guard let base64 = request.imageBase64, base64.count <= 55_924_056,
              let data = Data(base64Encoded: base64), data.starts(with: [137, 80, 78, 71, 13, 10, 26, 10]) else {
            return clipboardError("invalid_input", "A PNG image is required.")
        }
        item[UTType.png.identifier] = data
    default: return clipboardError("unsupported", "This clipboard format is not supported.")
    }
    #if os(iOS)
    var options: [UIPasteboard.OptionsKey: Any] = [:]
    if let localOnly = request.localOnly { options[.localOnly] = localOnly }
    if let expiry = request.expiresAt {
        let date = Date(timeIntervalSince1970: expiry / 1000)
        guard date > Date() else { return clipboardError("invalid_input", "Clipboard expiration must be in the future.") }
        options[.expirationDate] = date
    }
    UIPasteboard.general.setItems([item], options: options)
    #else
    guard request.localOnly != true, request.expiresAt == nil else {
        return clipboardError("unsupported", "Clipboard expiration and local-only writes are unavailable on macOS.")
    }
    let pasteboardItem = NSPasteboardItem()
    for (type, value) in item {
        let type = NSPasteboard.PasteboardType(type)
        if let text = value as? String { pasteboardItem.setString(text, forType: type) }
        if let data = value as? Data { pasteboardItem.setData(data, forType: type) }
    }
    NSPasteboard.general.clearContents()
    guard NSPasteboard.general.writeObjects([pasteboardItem]) else { return clipboardError("write_failed", "The clipboard could not be updated.") }
    #endif
    return strdup("{\"ok\":true}")
}

private func clipboardError(_ code: String, _ message: String) -> UnsafeMutablePointer<CChar>? {
    let value: [String: Any] = ["ok": false, "error": ["code": code, "message": message]]
    let data = try! JSONSerialization.data(withJSONObject: value)
    return strdup(String(decoding: data, as: UTF8.self))
}

import FlowLikeNative
import UniformTypeIdentifiers
#if os(iOS)
import UIKit
typealias SharePlatformController = UIViewController
#else
import AppKit
typealias SharePlatformController = NSViewController
#endif

final class ShareViewController: SharePlatformController {
    #if os(iOS)
    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        let label = UILabel()
        label.text = "Preparing content for Flow Like…"
        label.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(label)
        NSLayoutConstraint.activate([label.centerXAnchor.constraint(equalTo: view.centerXAnchor),
                                     label.centerYAnchor.constraint(equalTo: view.centerYAnchor)])
        receive()
    }
    #else
    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 360, height: 160))
        let label = NSTextField(labelWithString: "Preparing content for Flow Like…")
        label.frame = NSRect(x: 24, y: 72, width: 312, height: 24)
        view.addSubview(label)
    }
    override func viewDidLoad() { super.viewDidLoad(); receive() }
    #endif

    private func receive() {
        Task { @MainActor in
            do {
                let providers = (extensionContext?.inputItems as? [NSExtensionItem] ?? []).flatMap { $0.attachments ?? [] }
                guard !providers.isEmpty, providers.count <= 16 else { throw NativeIntegrationError.oversized }
                var text: [String] = []
                var files: [String] = []
                for provider in providers {
                    if provider.hasItemConformingToTypeIdentifier(UTType.url.identifier),
                       let url = try await item(provider, type: UTType.url.identifier) as? URL {
                        if url.isFileURL { files.append(try NativeStore.shared.importSharedFile(url).path) }
                        else { text.append(url.absoluteString) }
                    } else if provider.hasItemConformingToTypeIdentifier(UTType.plainText.identifier),
                              let value = try await item(provider, type: UTType.plainText.identifier) as? String {
                        text.append(value)
                    } else {
                        let type = provider.registeredTypeIdentifiers.first(where: {
                            UTType($0)?.conforms(to: .data) == true
                        }) ?? UTType.data.identifier
                        files.append(try await importFile(provider, type: type).path)
                    }
                }
                let combined = text.joined(separator: "\n\n")
                guard combined.utf8.count <= 1_048_576 else { throw NativeIntegrationError.oversized }
                chooseDestination(text: combined.isEmpty ? nil : combined, files: files)
            } catch {
                extensionContext?.cancelRequest(withError: error)
            }
        }
    }

    @MainActor private func chooseDestination(text: String?, files: [String]) {
        let snapshot = NativeStore.shared.snapshot()
        let bots = (snapshot?.bots ?? []).filter { $0.exposes("share") }.prefix(32)
        #if os(iOS)
        let chooser = UIAlertController(title: "Send to a bot", message: "Choose which bot receives this content when you open Flow Like.", preferredStyle: .actionSheet)
        for bot in bots {
            chooser.addAction(UIAlertAction(title: bot.title, style: .default) { [weak self] _ in
                self?.send(text: text, files: files, botId: bot.id, scope: snapshot?.scope)
            })
        }
        chooser.addAction(UIAlertAction(title: "Choose in Flow Like", style: .default) { [weak self] _ in
            self?.send(text: text, files: files, botId: nil, scope: snapshot?.scope)
        })
        chooser.addAction(UIAlertAction(title: "Cancel", style: .cancel) { [weak self] _ in
            self?.extensionContext?.completeRequest(returningItems: nil)
        })
        chooser.popoverPresentationController?.sourceView = view
        chooser.popoverPresentationController?.sourceRect = CGRect(x: view.bounds.midX, y: view.bounds.midY, width: 1, height: 1)
        present(chooser, animated: true)
        #else
        let chooser = NSAlert()
        chooser.messageText = "Send to a bot"
        chooser.informativeText = "Choose which bot receives this content in Flow Like."
        let menu = NSPopUpButton(frame: NSRect(x: 0, y: 0, width: 280, height: 28))
        menu.addItem(withTitle: "Choose in Flow Like")
        bots.forEach { menu.addItem(withTitle: $0.title) }
        chooser.accessoryView = menu
        chooser.addButton(withTitle: "Send")
        chooser.addButton(withTitle: "Cancel")
        let complete: (NSApplication.ModalResponse) -> Void = { [weak self] response in
            guard response == .alertFirstButtonReturn else {
                self?.extensionContext?.completeRequest(returningItems: nil)
                return
            }
            let index = menu.indexOfSelectedItem - 1
            let botId = bots.indices.contains(index) ? bots[index].id : nil
            self?.send(text: text, files: files, botId: botId, scope: snapshot?.scope)
        }
        if let window = view.window { chooser.beginSheetModal(for: window, completionHandler: complete) }
        else { complete(chooser.runModal()) }
        #endif
    }

    @MainActor private func send(text: String?, files: [String], botId: String?, scope: String?) {
        do {
            _ = try NativeStore.shared.enqueue(NativeAction(kind: "share", text: text, files: files, botId: botId), expectedScope: scope)
            showCompletion()
        } catch { extensionContext?.cancelRequest(withError: error) }
    }

    @MainActor private func showCompletion() {
        #if os(iOS)
        let alert = UIAlertController(title: "Sent to Flow Like", message: "Open Flow Like to choose what to do with this content.", preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "Done", style: .default) { [weak self] _ in
            self?.extensionContext?.completeRequest(returningItems: nil)
        })
        present(alert, animated: true)
        #else
        extensionContext?.completeRequest(returningItems: nil)
        #endif
    }

    private func item(_ provider: NSItemProvider, type: String) async throws -> NSSecureCoding? {
        try await withCheckedThrowingContinuation { continuation in
            provider.loadItem(forTypeIdentifier: type, options: nil) { value, error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume(returning: value) }
            }
        }
    }

    private func importFile(_ provider: NSItemProvider, type: String) async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            provider.loadFileRepresentation(forTypeIdentifier: type) { url, error in
                if let error { continuation.resume(throwing: error); return }
                guard let url else { continuation.resume(throwing: NativeIntegrationError.invalidAction); return }
                // The provider's temporary URL is valid only during this callback.
                do { continuation.resume(returning: try NativeStore.shared.importSharedFile(url)) }
                catch { continuation.resume(throwing: error) }
            }
        }
    }
}

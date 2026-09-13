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
                _ = try NativeStore.shared.enqueue(NativeAction(kind: "share", text: combined.isEmpty ? nil : combined, files: files))
                // Share extensions cannot force their containing app to launch on iOS.
                showCompletion()
            } catch {
                extensionContext?.cancelRequest(withError: error)
            }
        }
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

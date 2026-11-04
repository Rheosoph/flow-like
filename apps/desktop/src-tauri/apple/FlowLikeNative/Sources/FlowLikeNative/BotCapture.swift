#if os(iOS)
import AVFoundation
import Foundation
import Speech
import UIKit
import Vision
import VisionKit

@available(iOSApplicationExtension, unavailable)
@MainActor extension NativeBotDeviceRequest {
    func startCapture() {
        guard let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first(where: { $0.activationState == .foregroundActive }),
              var presenter = scene.windows.first(where: \.isKeyWindow)?.rootViewController else {
            fail("inactive", "Open Flow Like before recording or scanning.")
            return
        }
        while let presented = presenter.presentedViewController { presenter = presented }
        if options.operation == "voice" {
            guard Bundle.main.object(forInfoDictionaryKey: "NSSpeechRecognitionUsageDescription") != nil,
                  Bundle.main.object(forInfoDictionaryKey: "NSMicrophoneUsageDescription") != nil else {
                fail("unavailable", "Speech capture is unavailable in this app build. Use keyboard dictation instead.")
                return
            }
            let controller = BotVoiceCaptureController()
            controller.onUseText = { [weak self] text in self?.finish(["text": text]) }
            controller.onCancel = { [weak self] in self?.fail("cancelled", "Dictation was cancelled.") }
            let navigation = UINavigationController(rootViewController: controller)
            navigation.isModalInPresentation = true
            captureCleanup = { [controller, navigation] in controller.stop(); navigation.dismiss(animated: true) }
            presenter.present(navigation, animated: true)
            return
        }
        guard Bundle.main.object(forInfoDictionaryKey: "NSCameraUsageDescription") != nil else {
            fail("unavailable", "Camera capture is unavailable in this app build.")
            return
        }
        let delegate = BotDocumentCaptureDelegate(request: self)
        let controller: UIViewController
        if options.operation == "photo" {
            guard UIImagePickerController.isSourceTypeAvailable(.camera) else {
                fail("unavailable", "This device has no available camera.")
                return
            }
            let picker = UIImagePickerController()
            picker.sourceType = .camera
            picker.delegate = delegate
            controller = picker
        } else {
            guard VNDocumentCameraViewController.isSupported else {
                fail("unavailable", "Document scanning is unavailable on this device. Attach a document from Files instead.")
                return
            }
            let scanner = VNDocumentCameraViewController()
            scanner.delegate = delegate
            controller = scanner
        }
        controller.isModalInPresentation = true
        captureCleanup = { [controller, delegate] in
            _ = delegate
            controller.dismiss(animated: true)
        }
        presenter.present(controller, animated: true)
    }
}

@available(iOSApplicationExtension, unavailable)
@MainActor private final class BotVoiceCaptureController: UIViewController {
    var onUseText: ((String) -> Void)?
    var onCancel: (() -> Void)?
    private let transcript = UITextView()
    private let status = UILabel()
    private let record = UIButton(type: .system)
    private let engine = AVAudioEngine()
    private var recognition: SFSpeechRecognitionTask?
    private var audioRequest: SFSpeechAudioBufferRecognitionRequest?
    private var tapInstalled = false
    private var starting = false
    private var closed = false
    private var recognitionGeneration = UUID()

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Speak to bot"
        view.backgroundColor = .systemBackground
        navigationItem.leftBarButtonItem = UIBarButtonItem(title: "Cancel", style: .plain, target: self, action: #selector(cancel))
        navigationItem.rightBarButtonItem = UIBarButtonItem(title: "Use text", style: .done, target: self, action: #selector(useText))
        status.text = "Tap Start dictation, then review the text before using it."
        status.numberOfLines = 0
        status.font = .preferredFont(forTextStyle: .body)
        status.adjustsFontForContentSizeCategory = true
        transcript.font = .preferredFont(forTextStyle: .body)
        transcript.adjustsFontForContentSizeCategory = true
        transcript.accessibilityLabel = "Dictated message"
        transcript.layer.borderWidth = 1
        transcript.layer.borderColor = UIColor.separator.cgColor
        transcript.layer.cornerRadius = 8
        record.setTitle("Start dictation", for: .normal)
        record.titleLabel?.font = .preferredFont(forTextStyle: .headline)
        record.addTarget(self, action: #selector(toggle), for: .touchUpInside)
        let stack = UIStackView(arrangedSubviews: [status, transcript, record])
        stack.axis = .vertical
        stack.spacing = 20
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -20),
            stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 20),
            stack.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor, constant: -20),
            transcript.heightAnchor.constraint(greaterThanOrEqualToConstant: 120),
            record.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])
    }

    @objc private func toggle() {
        if engine.isRunning || starting { stop(); return }
        starting = true
        record.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            let speech = await withCheckedContinuation { continuation in
                SFSpeechRecognizer.requestAuthorization { continuation.resume(returning: $0) }
            }
            let microphone = await withCheckedContinuation { continuation in
                AVAudioApplication.requestRecordPermission { continuation.resume(returning: $0) }
            }
            guard !closed, starting, view.window != nil else { return }
            starting = false
            record.isEnabled = true
            guard speech == .authorized, microphone else {
                status.text = "Allow microphone and speech access in Settings, or type your message here."
                return
            }
            do { try beginRecognition() }
            catch { stop(); status.text = "Dictation could not start. Try again or type your message." }
        }
    }

    private func beginRecognition() throws {
        guard let recognizer = SFSpeechRecognizer(), recognizer.isAvailable else {
            status.text = "Speech recognition is unavailable. Type your message here."
            return
        }
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.record, mode: .measurement, options: [.duckOthers])
        try session.setActive(true)
        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        if recognizer.supportsOnDeviceRecognition { request.requiresOnDeviceRecognition = true }
        audioRequest = request
        let generation = UUID()
        recognitionGeneration = generation
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            throw NativeBotDeviceFailure("unavailable", "The microphone is unavailable.")
        }
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in request.append(buffer) }
        tapInstalled = true
        recognition = recognizer.recognitionTask(with: request) { [weak self] result, error in
            Task { @MainActor in
                guard let self, !self.closed, self.recognitionGeneration == generation else { return }
                if let result { self.transcript.text = result.bestTranscription.formattedString }
                if error != nil || result?.isFinal == true {
                    self.stop()
                    self.status.text = error == nil ? "Review the text, then tap Use text." : "Dictation stopped. Review the text or try again."
                }
            }
        }
        engine.prepare()
        try engine.start()
        transcript.isEditable = false
        record.setTitle("Stop dictation", for: .normal)
        status.text = "Listening. Your message stays here until you tap Use text."
    }

    func stop() {
        recognitionGeneration = UUID()
        starting = false
        engine.stop()
        if tapInstalled { engine.inputNode.removeTap(onBus: 0); tapInstalled = false }
        audioRequest?.endAudio()
        recognition?.cancel()
        recognition = nil
        audioRequest = nil
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        transcript.isEditable = true
        record.isEnabled = true
        record.setTitle("Start dictation", for: .normal)
    }

    @objc private func useText() {
        let text = transcript.text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { status.text = "Record or type a message first."; return }
        closed = true
        stop()
        onUseText?(text)
    }

    @objc private func cancel() { closed = true; stop(); onCancel?() }
    override func viewDidDisappear(_ animated: Bool) { super.viewDidDisappear(animated); closed = true; stop() }
}

@available(iOSApplicationExtension, unavailable)
@MainActor private final class BotDocumentCaptureDelegate: NSObject, @preconcurrency VNDocumentCameraViewControllerDelegate, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
    private weak var request: NativeBotDeviceRequest?
    init(request: NativeBotDeviceRequest) { self.request = request }

    func documentCameraViewControllerDidCancel(_ controller: VNDocumentCameraViewController) { request?.fail("cancelled", "Document scanning was cancelled.") }
    func documentCameraViewController(_ controller: VNDocumentCameraViewController, didFailWithError error: Error) { request?.fail("failed", "The camera could not scan this document.") }
    func imagePickerControllerDidCancel(_ picker: UIImagePickerController) { request?.fail("cancelled", "Photo capture was cancelled.") }

    func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
        guard let image = info[.originalImage] as? UIImage, let data = resized(image).jpegData(compressionQuality: 0.85) else {
            request?.fail("failed", "The camera returned no image.")
            return
        }
        deliver(data: data, name: "photo.jpg", pages: [data])
    }

    func documentCameraViewController(_ controller: VNDocumentCameraViewController, didFinishWith scan: VNDocumentCameraScan) {
        guard (1...16).contains(scan.pageCount) else {
            request?.fail("invalid_input", "Scan between 1 and 16 pages at a time.")
            return
        }
        let images = (0..<scan.pageCount).map { resized(scan.imageOfPage(at: $0)) }
        let bounds = CGRect(x: 0, y: 0, width: 612, height: 792)
        let data = UIGraphicsPDFRenderer(bounds: bounds).pdfData { context in
            for image in images {
                context.beginPage()
                let scale = min(bounds.width / image.size.width, bounds.height / image.size.height)
                let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
                image.draw(in: CGRect(x: (bounds.width - size.width) / 2, y: (bounds.height - size.height) / 2, width: size.width, height: size.height))
            }
        }
        deliver(data: data, name: "scan.pdf", pages: images.compactMap { $0.jpegData(compressionQuality: 0.8) })
    }

    private func resized(_ image: UIImage) -> UIImage {
        let scale = min(1, 1600 / max(image.size.width, image.size.height))
        let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        return UIGraphicsImageRenderer(size: size, format: format).image { _ in image.draw(in: CGRect(origin: .zero, size: size)) }
    }

    private func deliver(data: Data, name: String, pages: [Data]) {
        guard let request else { return }
        guard data.count <= 20_971_520 else { request.fail("invalid_input", "This capture exceeds the 20 MiB attachment limit."); return }
        Task { [weak request] in
            let text = await Task.detached(priority: .userInitiated) {
                pages.compactMap { data -> String? in
                    let recognition = VNRecognizeTextRequest()
                    recognition.recognitionLevel = .accurate
                    recognition.automaticallyDetectsLanguage = true
                    do { try VNImageRequestHandler(data: data).perform([recognition]) } catch { return nil }
                    return recognition.results?.compactMap { $0.topCandidates(1).first?.string }.joined(separator: "\n")
                }.joined(separator: "\n\n")
            }.value
            guard let request, NativeBotDeviceService.requests[request.id] === request else { return }
            guard NativeStore.shared.persistedScope() == request.options.scope else {
                request.fail("scope_changed", "The active workspace changed. The captured content was not delivered.")
                return
            }
            let temporary = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + "-" + name)
            defer { try? FileManager.default.removeItem(at: temporary) }
            do {
                try data.write(to: temporary, options: .atomic)
                let path = try NativeStore.shared.importSharedFile(temporary)
                request.finish(["text": String(text.prefix(32_000)), "files": [path.path]])
            } catch { request.fail("failed", "The scanned document could not be saved. Try fewer pages.") }
        }
    }
}
#endif

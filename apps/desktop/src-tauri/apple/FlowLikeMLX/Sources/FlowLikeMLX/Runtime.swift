import Foundation
import MLX
import MLXLLM
import MLXLMCommon
import MLXVLM

#if canImport(UIKit)
import UIKit
#endif

#if os(iOS)
import os
#endif

#if os(iOS)
private let flowLikeMLXDefaultCacheLimit = 20 * 1024 * 1024
#else
private let flowLikeMLXDefaultCacheLimit = 128 * 1024 * 1024
#endif

public typealias FlowLikeMLXEventHandler = @Sendable (String) -> Void

/// A synchronous admission gate closes before the asynchronous lifecycle task
/// reaches the engine actor. The epoch prevents a delayed deactivation task
/// from cancelling work submitted after a rapid foreground reactivation.
final class FlowLikeMLXForegroundGate: @unchecked Sendable {
    private let lock = NSLock()
    private var active = true
    private var epoch: UInt64 = 0

    var allowsExecution: Bool {
        lock.lock()
        defer { lock.unlock() }
        return active
    }

    @discardableResult
    func deactivate() -> UInt64 {
        lock.lock()
        defer { lock.unlock() }
        active = false
        epoch &+= 1
        return epoch
    }

    func activate() {
        lock.lock()
        active = true
        epoch &+= 1
        lock.unlock()
    }

    func isCurrentDeactivation(_ candidate: UInt64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return !active && epoch == candidate
    }
}

/// Public façade shared by the iOS C ABI and the macOS NDJSON executable.
///
/// The engine deliberately runs one generation at a time. This avoids
/// multiplying KV-cache and temporary-buffer pressure on iOS, while still
/// retaining loaded model weights in a small platform-specific LRU cache.
public final class FlowLikeMLXRuntime: @unchecked Sendable {
    public static let shared = FlowLikeMLXRuntime()

    private let foregroundGate: FlowLikeMLXForegroundGate
    private let engine: FlowLikeMLXEngine
    private let lifecycleLock = NSLock()
    private var lifecyclePrepared = false
    private var lifecycleObservers: [NSObjectProtocol] = []

    private init() {
        let foregroundGate = FlowLikeMLXForegroundGate()
        self.foregroundGate = foregroundGate
        self.engine = FlowLikeMLXEngine(foregroundGate: foregroundGate)
    }

    public static var isAvailable: Bool {
        #if targetEnvironment(simulator)
            return false
        #elseif arch(arm64)
            #if os(iOS) || os(macOS)
                return GPU.deviceInfo().maxBufferSize > 0
            #else
                return false
            #endif
        #else
            return false
        #endif
    }

    /// Apply the MLX memory policy and install iOS lifecycle hooks once.
    public static func prepareForAppLifecycle() {
        shared.prepareForAppLifecycle()
    }

    private func prepareForAppLifecycle() {
        lifecycleLock.lock()
        defer { lifecycleLock.unlock() }
        guard !lifecyclePrepared else { return }
        lifecyclePrepared = true

        // The official MLX iOS guidance uses a 20 MiB recyclable-buffer
        // cache as a conservative starting point for jetsam-limited apps.
        MLX.Memory.cacheLimit = flowLikeMLXDefaultCacheLimit

        #if canImport(UIKit)
            lifecycleObservers.append(
                NotificationCenter.default.addObserver(
                    forName: UIApplication.didReceiveMemoryWarningNotification,
                    object: nil,
                    queue: .main
                ) { [engine] _ in
                    Task {
                        await engine.handleMemoryWarning()
                    }
                }
            )
            lifecycleObservers.append(
                NotificationCenter.default.addObserver(
                    // Stop submitting GPU work before iOS backgrounds the app.
                    // Waiting for didEnterBackground is too late when a prompt
                    // prefill already has Metal command buffers in flight.
                    forName: UIApplication.willResignActiveNotification,
                    object: nil,
                    queue: .main
                ) { [engine, foregroundGate] _ in
                    let deactivation = foregroundGate.deactivate()
                    Task {
                        await engine.releaseForAppDeactivation(deactivation)
                    }
                }
            )
            lifecycleObservers.append(
                NotificationCenter.default.addObserver(
                    forName: UIApplication.didBecomeActiveNotification,
                    object: nil,
                    queue: .main
                ) { [engine, foregroundGate] _ in
                    foregroundGate.activate()
                    Task {
                        await engine.resumeAfterAppActivation()
                    }
                }
            )
        #endif
    }

    /// Accept a validated generation command.
    ///
    /// Every accepted submission produces exactly one terminal event through
    /// `eventHandler`, even when queued work is cancelled.
    public func submit(
        _ command: FlowLikeMLXCommand,
        eventHandler: @escaping FlowLikeMLXEventHandler
    ) async {
        await engine.submit(command, eventHandler: eventHandler)
    }

    public func cancel(requestID: String) async {
        await engine.cancel(requestID: requestID)
    }

    public func unload(modelDirectory: String) async {
        await engine.unload(modelDirectory: modelDirectory)
    }

    public func clearCache() async {
        await engine.clearCache()
    }

    public func waitUntilIdle() async {
        await engine.waitUntilIdle()
    }
}

/// Serializes callbacks and permanently closes the stream before invoking a
/// terminal callback. Holding the lock across the C callback guarantees that
/// no later callback can race with Rust freeing its opaque context.
final class FlowLikeMLXEventEmitter: @unchecked Sendable {
    private let lock = NSLock()
    private let handler: FlowLikeMLXEventHandler
    private var terminalDelivered = false

    init(handler: @escaping FlowLikeMLXEventHandler) {
        self.handler = handler
    }

    func chunk(id: String, data: OpenAIChatCompletionChunk) {
        emit(
            FlowLikeMLXEventCodec.encode(id: id, event: "chunk", data: data),
            terminal: false
        )
    }

    func complete(id: String, data: OpenAIChatCompletionResponse) {
        emit(
            FlowLikeMLXEventCodec.encode(id: id, event: "complete", data: data),
            terminal: true
        )
    }

    func error(id: String, message: String) {
        emit(
            FlowLikeMLXEventCodec.error(id: id, message: message),
            terminal: true
        )
    }

    private func emit(_ event: String, terminal: Bool) {
        lock.lock()
        defer { lock.unlock() }
        guard !terminalDelivered else { return }
        if terminal {
            terminalDelivered = true
        }
        handler(event)
    }
}

/// MLX's `AsyncStream` is backed by a separate producer task. Cancelling or
/// breaking out of the consumer does not mean that producer has stopped using
/// its model/KV cache yet, so every exit must cancel and join it before the
/// engine clears memory or starts another request.
func cancelAndJoinMLXProducer(_ producer: Task<Void, Never>) async {
    producer.cancel()
    await producer.value
}

enum FlowLikeMLXMemoryWarningAction: Equatable, Sendable {
    case releaseImmediately
    case releaseWhenIdle
    case cancelAndRelease
}

/// A foreground memory warning is a request to discard reclaimable state, not
/// proof that the active user operation must fail. Give each request one
/// warning in which to settle naturally; a repeated warning for that same
/// request escalates so sustained pressure still has a path to release weights.
struct FlowLikeMLXMemoryWarningPolicy: Sendable {
    private var warnedActiveRequestID: String?

    mutating func action(
        activeRequestID: String?
    ) -> FlowLikeMLXMemoryWarningAction {
        guard let activeRequestID else {
            warnedActiveRequestID = nil
            return .releaseImmediately
        }
        guard warnedActiveRequestID == activeRequestID else {
            warnedActiveRequestID = activeRequestID
            return .releaseWhenIdle
        }
        return .cancelAndRelease
    }

    mutating func requestDidFinish(_ requestID: String) {
        if warnedActiveRequestID == requestID {
            warnedActiveRequestID = nil
        }
    }
}

private actor FlowLikeMLXEngine {
    private struct Job: Sendable {
        let command: FlowLikeMLXCommand
        let emitter: FlowLikeMLXEventEmitter
    }

    private struct ActiveJob: Sendable {
        let job: Job
        let task: Task<Void, Never>
        var cancellationMessage: String?
    }

    private struct ModelCacheKey: Hashable, Sendable {
        let directory: String
        let kind: FlowLikeMLXModelKind
    }

    private struct ModelCacheEntry: Sendable {
        let container: ModelContainer
        var lastAccess: UInt64
    }

    private var active: ActiveJob?
    private var pending: [Job] = []
    private var models: [ModelCacheKey: ModelCacheEntry] = [:]
    private var accessSequence: UInt64 = 0
    private var idleWaiters: [CheckedContinuation<Void, Never>] = []
    private var cancelledBeforeSubmission: Set<String> = []
    private var cancellationOrder: [String] = []
    private var deferredModelUnloads: Set<String> = []
    private var releaseAllModelsWhenIdle = false
    private var clearCacheWhenIdle = false
    private var restoreCacheLimitWhenIdle = false
    private var memoryWarningPolicy = FlowLikeMLXMemoryWarningPolicy()
    private let foregroundGate: FlowLikeMLXForegroundGate

    private static let maximumRememberedEarlyCancellations = 256

    init(foregroundGate: FlowLikeMLXForegroundGate) {
        self.foregroundGate = foregroundGate
    }

    private var maximumCachedModels: Int {
        #if os(iOS)
            1
        #else
            2
        #endif
    }

    func submit(
        _ command: FlowLikeMLXCommand,
        eventHandler: @escaping FlowLikeMLXEventHandler
    ) {
        let emitter = FlowLikeMLXEventEmitter(handler: eventHandler)
        guard foregroundGate.allowsExecution else {
            emitter.error(
                id: command.id,
                message: "MLX generation is unavailable while the application is inactive"
            )
            return
        }
        do {
            _ = try command.validatedGenerate()
        } catch {
            emitter.error(id: command.id, message: error.localizedDescription)
            return
        }

        // C ABI calls enter the actor from independent Swift Tasks. Remember a
        // cancellation that wins that scheduling race so an accepted request
        // cannot start after Rust has already cancelled it.
        if cancelledBeforeSubmission.remove(command.id) != nil {
            cancellationOrder.removeAll { $0 == command.id }
            emitter.error(
                id: command.id,
                message: "MLX generation was cancelled"
            )
            return
        }

        guard !contains(requestID: command.id) else {
            emitter.error(
                id: command.id,
                message: "A generation request with id \"\(command.id)\" already exists"
            )
            return
        }

        pending.append(Job(command: command, emitter: emitter))
        startNextIfNeeded()
    }

    func cancel(requestID: String) {
        if let active, active.job.command.id == requestID {
            cancelActive(
                message: "MLX generation was cancelled"
            )
            return
        }

        guard let index = pending.firstIndex(where: { $0.command.id == requestID }) else {
            rememberEarlyCancellation(requestID)
            return
        }
        let job = pending.remove(at: index)
        job.emitter.error(id: requestID, message: "MLX generation was cancelled")
        resumeIdleWaitersIfNeeded()
    }

    func unload(modelDirectory: String) {
        let normalizedDirectory = normalizedPath(modelDirectory)

        if let active,
            normalizedPath(active.job.command.modelDirectory ?? "") == normalizedDirectory
        {
            deferredModelUnloads.insert(normalizedDirectory)
            clearCacheWhenIdle = true
            cancelActive(
                message: "MLX generation cancelled: model was unloaded"
            )
        }

        let cancelled = pending.filter {
            normalizedPath($0.command.modelDirectory ?? "") == normalizedDirectory
        }
        pending.removeAll {
            normalizedPath($0.command.modelDirectory ?? "") == normalizedDirectory
        }
        for job in cancelled {
            job.emitter.error(
                id: job.command.id,
                message: "MLX model was unloaded before generation started"
            )
        }

        if active == nil {
            models = models.filter { $0.key.directory != normalizedDirectory }
            MLX.Memory.clearCache()
        } else {
            deferredModelUnloads.insert(normalizedDirectory)
            clearCacheWhenIdle = true
        }
        resumeIdleWaitersIfNeeded()
    }

    func clearCache() {
        guard active == nil else {
            clearCacheWhenIdle = true
            return
        }
        MLX.Memory.clearCache()
    }

    func handleMemoryWarning() {
        // Applying a zero limit is non-destructive: MLX returns recyclable
        // buffers as they are deallocated, while model weights and in-flight
        // command buffers stay alive until the producer has quiesced.
        reduceCacheForMemoryPressure()
        switch memoryWarningPolicy.action(
            activeRequestID: active?.job.command.id
        ) {
        case .releaseImmediately:
            models.removeAll(keepingCapacity: false)
            deferredModelUnloads.removeAll(keepingCapacity: false)
            releaseAllModelsWhenIdle = false
            clearCacheWhenIdle = false
            MLX.Memory.clearCache()
            restoreCacheLimitAfterMemoryPressure()

        case .releaseWhenIdle:
            releaseAllModelsWhenIdle = true
            clearCacheWhenIdle = true

        case .cancelAndRelease:
            releaseForMemoryPressure(reason: "repeated iOS memory warning")
        }
    }

    func releaseForMemoryPressure(reason: String) {
        reduceCacheForMemoryPressure()
        cancelActive(message: "MLX generation cancelled: \(reason)")
        let cancelled = pending
        pending.removeAll(keepingCapacity: false)
        for job in cancelled {
            job.emitter.error(
                id: job.command.id,
                message: "MLX generation cancelled: \(reason)"
            )
        }
        if active == nil {
            models.removeAll(keepingCapacity: false)
            MLX.Memory.clearCache()
            restoreCacheLimitAfterMemoryPressure()
        } else {
            releaseAllModelsWhenIdle = true
            clearCacheWhenIdle = true
        }
        resumeIdleWaitersIfNeeded()
    }

    func releaseForAppDeactivation(_ deactivation: UInt64) {
        guard foregroundGate.isCurrentDeactivation(deactivation) else { return }
        releaseForMemoryPressure(reason: "application is leaving the foreground")
    }

    func resumeAfterAppActivation() {
        guard foregroundGate.allowsExecution else { return }
        startNextIfNeeded()
    }

    func waitUntilIdle() async {
        guard active != nil || !pending.isEmpty else { return }
        await withCheckedContinuation { continuation in
            idleWaiters.append(continuation)
        }
    }

    private func contains(requestID: String) -> Bool {
        active?.job.command.id == requestID
            || pending.contains(where: { $0.command.id == requestID })
    }

    private func rememberEarlyCancellation(_ requestID: String) {
        guard !requestID.isEmpty,
            cancelledBeforeSubmission.insert(requestID).inserted
        else {
            return
        }
        cancellationOrder.append(requestID)
        if cancellationOrder.count > Self.maximumRememberedEarlyCancellations {
            let expired = cancellationOrder.removeFirst()
            cancelledBeforeSubmission.remove(expired)
        }
    }

    private func startNextIfNeeded() {
        guard foregroundGate.allowsExecution, active == nil, !pending.isEmpty else { return }
        let job = pending.removeFirst()
        let task = Task<Void, Never> { [weak self] in
            guard let self else { return }
            await self.run(job)
        }
        active = ActiveJob(
            job: job,
            task: task,
            cancellationMessage: nil
        )
    }

    private func run(_ job: Job) async {
        defer {
            memoryWarningPolicy.requestDidFinish(job.command.id)
            if active?.job.command.id == job.command.id {
                active = nil
            }
            performDeferredCleanupIfIdle()
            startNextIfNeeded()
            resumeIdleWaitersIfNeeded()
        }

        do {
            try Task.checkCancellation()
            let validated = try job.command.validatedGenerate()
            try await generate(
                job: job,
                modelDirectory: validated.modelDirectory,
                modelKind: validated.modelKind,
                request: validated.request
            )
        } catch is CancellationError {
            clearCacheWhenIdle = true
            job.emitter.error(
                id: job.command.id,
                message: activeCancellationMessage(for: job.command.id)
            )
        } catch {
            clearCacheWhenIdle = true
            job.emitter.error(
                id: job.command.id,
                message: error.localizedDescription
            )
        }
    }

    private func generate(
        job: Job,
        modelDirectory: String,
        modelKind: FlowLikeMLXModelKind,
        request: OpenAIChatCompletionRequest
    ) async throws {
        let mapped = try FlowLikeMLXRequestMapper.map(request, modelKind: modelKind)
        let userInput = try await FlowLikeMLXRequestMapper.makeUserInput(
            mapped: mapped,
            request: request,
            modelKind: modelKind
        )
        let parameters = try FlowLikeMLXRequestMapper.makeGenerationParameters(request)
        try Task.checkCancellation()

        let container = try await modelContainer(
            directory: modelDirectory,
            kind: modelKind
        )
        try Task.checkCancellation()

        let prepared = try await container.prepare(input: userInput)
        try Task.checkCancellation()
        // Read before `prepared` is consumed by TokenIterator; a stop sequence
        // ends the loop before the terminal info event carries these counts.
        let promptTokenCount = prepared.text.tokens.size
        let (generations, generationTask) = try await container.perform(
            nonSendable: prepared
        ) { context, input in
            let iterator = try TokenIterator(
                input: input,
                model: context.model,
                parameters: parameters
            )
            return MLXLMCommon.generateTask(
                promptTokenCount: promptTokenCount,
                modelConfiguration: context.configuration,
                tokenizer: context.tokenizer,
                iterator: iterator
            )
        }

        let responseID = "chatcmpl-\(job.command.id)"
        let created = Int(Date().timeIntervalSince1970)
        let modelName =
            request.model
            ?? URL(fileURLWithPath: modelDirectory).lastPathComponent
        let streaming = request.stream == true

        if streaming {
            job.emitter.chunk(
                id: job.command.id,
                data: makeChunk(
                    id: responseID,
                    created: created,
                    model: modelName,
                    delta: OpenAIChunkDelta(
                        role: "assistant",
                        content: nil,
                        toolCalls: nil
                    )
                )
            )
        }

        var filter = StopSequenceFilter(stops: mapped.stopSequences)
        var content = ""
        var generatedText = ""
        var toolCalls: [OpenAIResponseToolCall] = []
        var completionInfo: GenerateCompletionInfo?
        var stoppedBySequence = false

        do {
            generationLoop: for await generation in generations {
                try Task.checkCancellation()
                switch generation {
                case .chunk(let text):
                    generatedText += text
                    let filtered = filter.consume(text)
                    if !filtered.isEmpty {
                        content += filtered
                        if streaming {
                            job.emitter.chunk(
                                id: job.command.id,
                                data: makeChunk(
                                    id: responseID,
                                    created: created,
                                    model: modelName,
                                    delta: OpenAIChunkDelta(
                                        role: nil,
                                        content: filtered,
                                        toolCalls: nil
                                    )
                                )
                            )
                        }
                    }
                    if filter.didStop {
                        stoppedBySequence = true
                        break generationLoop
                    }

                case .toolCall(let toolCall):
                    let responseToolCall = try makeToolCall(
                        toolCall,
                        requestID: job.command.id,
                        index: toolCalls.count
                    )
                    toolCalls.append(responseToolCall)
                    if streaming {
                        job.emitter.chunk(
                            id: job.command.id,
                            data: makeChunk(
                                id: responseID,
                                created: created,
                                model: modelName,
                                delta: OpenAIChunkDelta(
                                    role: nil,
                                    content: nil,
                                    toolCalls: [
                                        OpenAIChunkToolCall(
                                            index: toolCalls.count - 1,
                                            id: responseToolCall.id,
                                            type: responseToolCall.type,
                                            function: OpenAIChunkToolCall.Function(
                                                name: responseToolCall.function.name,
                                                arguments: responseToolCall.function.arguments
                                            )
                                        )
                                    ]
                                )
                            )
                        )
                    }

                case .info(let info):
                    completionInfo = info
                }
            }
        } catch {
            await cancelAndJoinMLXProducer(generationTask)
            throw error
        }
        await cancelAndJoinMLXProducer(generationTask)

        try Task.checkCancellation()
        if !stoppedBySequence {
            let tail = filter.finish()
            if !tail.isEmpty {
                content += tail
                if streaming {
                    job.emitter.chunk(
                        id: job.command.id,
                        data: makeChunk(
                            id: responseID,
                            created: created,
                            model: modelName,
                            delta: OpenAIChunkDelta(
                                role: nil,
                                content: tail,
                                toolCalls: nil
                            )
                        )
                    )
                }
            }
        }

        let finishReason = makeFinishReason(
            info: completionInfo,
            hasToolCalls: !toolCalls.isEmpty,
            stoppedBySequence: stoppedBySequence
        )
        // Stopping on a stop sequence leaves the generation stream without its
        // terminal info event, so fall back to counting locally rather than
        // reporting an empty usage block.
        var fallbackCompletionTokens = 0
        if completionInfo == nil, !generatedText.isEmpty {
            fallbackCompletionTokens = await container.tokenizer
                .encode(text: generatedText, addSpecialTokens: false)
                .count
        }
        let usage = FlowLikeMLXUsageAccounting.make(
            info: completionInfo,
            fallbackPromptTokens: promptTokenCount,
            fallbackCompletionTokens: fallbackCompletionTokens
        )

        if FlowLikeMLXOutputGuard.droppedEveryToken(
            content: content,
            toolCallCount: toolCalls.count,
            completionTokens: usage.completionTokens
        ) {
            throw FlowLikeMLXError.unparsableToolCall(
                "the model generated \(usage.completionTokens) tokens that produced "
                    + "neither text nor a usable tool call; the output was most likely a "
                    + "malformed tool call and was discarded by the parser"
            )
        }

        if streaming {
            job.emitter.chunk(
                id: job.command.id,
                data: OpenAIChatCompletionChunk(
                    id: responseID,
                    object: "chat.completion.chunk",
                    created: created,
                    model: modelName,
                    choices: [
                        OpenAIChunkChoice(
                            index: 0,
                            delta: OpenAIChunkDelta(
                                role: nil,
                                content: nil,
                                toolCalls: nil
                            ),
                            finishReason: finishReason
                        )
                    ],
                    usage: nil
                )
            )
        }

        let response = OpenAIChatCompletionResponse(
            id: responseID,
            object: "chat.completion",
            created: created,
            model: modelName,
            choices: [
                OpenAICompletionChoice(
                    index: 0,
                    message: OpenAIAssistantMessage(
                        role: "assistant",
                        content: content.isEmpty && !toolCalls.isEmpty ? nil : content,
                        toolCalls: toolCalls.isEmpty ? nil : toolCalls
                    ),
                    finishReason: finishReason
                )
            ],
            usage:
                !streaming || request.streamOptions?.includeUsage == true
                ? usage : nil
        )
        job.emitter.complete(id: job.command.id, data: response)
    }

    private func modelContainer(
        directory: String,
        kind: FlowLikeMLXModelKind
    ) async throws -> ModelContainer {
        let modelURL = try validateModelDirectory(directory)
        let key = ModelCacheKey(
            directory: modelURL.path,
            kind: kind
        )
        accessSequence &+= 1
        if var cached = models[key] {
            cached.lastAccess = accessSequence
            models[key] = cached
            return cached.container
        }

        // Evict before loading, which is important on iOS where briefly holding
        // two model weight sets can cross the jetsam threshold.
        while models.count >= maximumCachedModels {
            guard let oldest = models.min(by: {
                $0.value.lastAccess < $1.value.lastAccess
            })?.key else {
                break
            }
            models.removeValue(forKey: oldest)
            MLX.Memory.clearCache()
        }

        #if os(iOS)
            try validateIOSModelFitsAvailableMemory(modelURL)
        #endif

        let tokenizerLoader = FlowLikeLocalTokenizerLoader()
        let container: ModelContainer
        switch kind {
        case .llm:
            container = try await LLMModelFactory.shared.loadContainer(
                from: modelURL,
                using: tokenizerLoader
            )
        case .vlm:
            container = try await VLMModelFactory.shared.loadContainer(
                from: modelURL,
                using: tokenizerLoader
            )
        }
        models[key] = ModelCacheEntry(
            container: container,
            lastAccess: accessSequence
        )
        return container
    }

    private func validateModelDirectory(_ path: String) throws -> URL {
        guard !path.isEmpty else {
            throw FlowLikeMLXError.invalidModelDirectory("path must not be empty")
        }
        let directory =
            URL(fileURLWithPath: path, isDirectory: true)
            .standardizedFileURL
            .resolvingSymlinksInPath()
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(
            atPath: directory.path,
            isDirectory: &isDirectory
        ), isDirectory.boolValue
        else {
            throw FlowLikeMLXError.invalidModelDirectory(
                "directory does not exist: \(directory.path)"
            )
        }
        guard FileManager.default.fileExists(
            atPath: directory.appendingPathComponent("config.json").path
        ) else {
            throw FlowLikeMLXError.invalidModelDirectory(
                "config.json is missing from \(directory.path)"
            )
        }

        let enumerator = FileManager.default.enumerator(
            at: directory,
            includingPropertiesForKeys: [.isRegularFileKey],
            options: [.skipsHiddenFiles, .skipsPackageDescendants]
        )
        var foundWeights = false
        while let file = enumerator?.nextObject() as? URL {
            if file.pathExtension.lowercased() == "safetensors" {
                foundWeights = true
                break
            }
        }
        guard foundWeights else {
            throw FlowLikeMLXError.invalidModelDirectory(
                "no .safetensors weights were found in \(directory.path)"
            )
        }
        return directory
    }

    #if os(iOS)
        /// Reject a checkpoint whose weight files alone exceed the process's
        /// current allocation headroom. This is deliberately a one-sided test:
        /// passing does not promise that transient/KV memory will fit, while
        /// failing means eager `eval(model)` cannot make the weights resident.
        private func validateIOSModelFitsAvailableMemory(_ directory: URL) throws {
            let availableBytes = UInt64(os_proc_available_memory())
            guard availableBytes > 0 else { return }

            let enumerator = FileManager.default.enumerator(
                at: directory,
                includingPropertiesForKeys: [.isRegularFileKey, .fileSizeKey],
                options: [.skipsHiddenFiles, .skipsPackageDescendants]
            )
            var weightBytes: UInt64 = 0
            while let file = enumerator?.nextObject() as? URL {
                guard file.pathExtension.lowercased() == "safetensors" else { continue }
                let values = try file.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
                guard values.isRegularFile == true, let fileSize = values.fileSize else { continue }
                let (sum, overflow) = weightBytes.addingReportingOverflow(UInt64(fileSize))
                weightBytes = overflow ? UInt64.max : sum
                if overflow { break }
            }

            guard weightBytes > availableBytes else { return }
            let formatter = ByteCountFormatter()
            formatter.countStyle = .memory
            throw FlowLikeMLXError.unsupported(
                "MLX model weights require at least \(formatter.string(fromByteCount: Int64(clamping: weightBytes))), "
                    + "but iOS currently reports only "
                    + "\(formatter.string(fromByteCount: Int64(clamping: availableBytes))) available; "
                    + "choose a smaller quantized model"
            )
        }
    #endif

    private func normalizedPath(_ path: String) -> String {
        URL(fileURLWithPath: path, isDirectory: true)
            .standardizedFileURL
            .resolvingSymlinksInPath()
            .path
    }

    private func performDeferredCleanupIfIdle() {
        guard active == nil else { return }

        if releaseAllModelsWhenIdle {
            models.removeAll(keepingCapacity: false)
            deferredModelUnloads.removeAll(keepingCapacity: false)
            releaseAllModelsWhenIdle = false
        } else if !deferredModelUnloads.isEmpty {
            models = models.filter {
                !deferredModelUnloads.contains($0.key.directory)
            }
            deferredModelUnloads.removeAll(keepingCapacity: false)
        }

        if clearCacheWhenIdle {
            MLX.Memory.clearCache()
            clearCacheWhenIdle = false
        }
        restoreCacheLimitAfterMemoryPressure()
    }

    private func reduceCacheForMemoryPressure() {
        MLX.Memory.cacheLimit = 0
        restoreCacheLimitWhenIdle = true
    }

    private func restoreCacheLimitAfterMemoryPressure() {
        guard restoreCacheLimitWhenIdle else { return }
        MLX.Memory.cacheLimit = flowLikeMLXDefaultCacheLimit
        restoreCacheLimitWhenIdle = false
    }

    private func cancelActive(message: String) {
        guard active != nil else { return }
        if active?.cancellationMessage == nil {
            active?.cancellationMessage = message
        }
        active?.task.cancel()
    }

    private func activeCancellationMessage(for requestID: String) -> String {
        guard active?.job.command.id == requestID else {
            return "MLX generation was cancelled"
        }
        return active?.cancellationMessage ?? "MLX generation was cancelled"
    }

    private func makeChunk(
        id: String,
        created: Int,
        model: String,
        delta: OpenAIChunkDelta
    ) -> OpenAIChatCompletionChunk {
        OpenAIChatCompletionChunk(
            id: id,
            object: "chat.completion.chunk",
            created: created,
            model: model,
            choices: [
                OpenAIChunkChoice(
                    index: 0,
                    delta: delta,
                    finishReason: nil
                )
            ],
            usage: nil
        )
    }

    private func makeToolCall(
        _ toolCall: ToolCall,
        requestID: String,
        index: Int
    ) throws -> OpenAIResponseToolCall {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        let argumentsData = try encoder.encode(toolCall.function.arguments)
        return OpenAIResponseToolCall(
            id: "call_\(requestID)_\(index)",
            type: "function",
            function: OpenAIResponseToolCall.Function(
                name: toolCall.function.name,
                arguments: String(decoding: argumentsData, as: UTF8.self)
            )
        )
    }

    private func makeFinishReason(
        info: GenerateCompletionInfo?,
        hasToolCalls: Bool,
        stoppedBySequence: Bool
    ) -> String {
        if hasToolCalls {
            return "tool_calls"
        }
        if stoppedBySequence {
            return "stop"
        }
        switch info?.stopReason {
        case .some(.length):
            return "length"
        case .some(.stop), .some(.cancelled), .none:
            return "stop"
        }
    }

    private func resumeIdleWaitersIfNeeded() {
        guard active == nil, pending.isEmpty, !idleWaiters.isEmpty else { return }
        let waiters = idleWaiters
        idleWaiters.removeAll(keepingCapacity: false)
        for waiter in waiters {
            waiter.resume()
        }
    }
}

/// Detects generations whose entire output disappeared. The tool-call parser
/// buffers anything that looks like a call and discards the buffer when it does
/// not parse, so a malformed call otherwise reaches the caller as a successful
/// but completely empty answer.
enum FlowLikeMLXOutputGuard {
    static func droppedEveryToken(
        content: String,
        toolCallCount: Int,
        completionTokens: Int
    ) -> Bool {
        content.isEmpty && toolCallCount == 0 && completionTokens > 0
    }
}

/// Token accounting for a finished generation. The runtime only reports counts
/// through a terminal info event, which never arrives when a stop sequence ends
/// the loop early; the fallback keeps usage reporting truthful in that case.
enum FlowLikeMLXUsageAccounting {
    static func make(
        info: GenerateCompletionInfo?,
        fallbackPromptTokens: Int,
        fallbackCompletionTokens: Int
    ) -> OpenAIUsage {
        let promptTokens = info?.promptTokenCount ?? max(0, fallbackPromptTokens)
        let completionTokens =
            info?.generationTokenCount ?? max(0, fallbackCompletionTokens)
        return OpenAIUsage(
            promptTokens: promptTokens,
            completionTokens: completionTokens,
            totalTokens: promptTokens + completionTokens
        )
    }
}

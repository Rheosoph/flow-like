import Foundation
import MLX
import MLXLLM
import MLXLMCommon
import MLXVLM

#if canImport(UIKit)
import UIKit
#endif

public typealias FlowLikeMLXEventHandler = @Sendable (String) -> Void

/// Public façade shared by the iOS C ABI and the macOS NDJSON executable.
///
/// The engine deliberately runs one generation at a time. This avoids
/// multiplying KV-cache and temporary-buffer pressure on iOS, while still
/// retaining loaded model weights in a small platform-specific LRU cache.
public final class FlowLikeMLXRuntime: @unchecked Sendable {
    public static let shared = FlowLikeMLXRuntime()

    private let engine = FlowLikeMLXEngine()
    private let lifecycleLock = NSLock()
    private var lifecyclePrepared = false
    private var lifecycleObservers: [NSObjectProtocol] = []

    private init() {}

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

        #if os(iOS)
            // The official MLX iOS guidance uses a 20 MiB recyclable-buffer
            // cache as a conservative starting point for jetsam-limited apps.
            MLX.Memory.cacheLimit = 20 * 1024 * 1024
        #else
            MLX.Memory.cacheLimit = 128 * 1024 * 1024
        #endif

        #if canImport(UIKit)
            lifecycleObservers.append(
                NotificationCenter.default.addObserver(
                    forName: UIApplication.didReceiveMemoryWarningNotification,
                    object: nil,
                    queue: .main
                ) { [engine] _ in
                    Task {
                        await engine.releaseForMemoryPressure(
                            reason: "iOS memory warning"
                        )
                    }
                }
            )
            lifecycleObservers.append(
                NotificationCenter.default.addObserver(
                    forName: UIApplication.didEnterBackgroundNotification,
                    object: nil,
                    queue: .main
                ) { [engine] _ in
                    Task {
                        await engine.releaseForMemoryPressure(
                            reason: "application entered the background"
                        )
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

private actor FlowLikeMLXEngine {
    private struct Job: Sendable {
        let command: FlowLikeMLXCommand
        let emitter: FlowLikeMLXEventEmitter
    }

    private struct ActiveJob: Sendable {
        let job: Job
        let task: Task<Void, Never>
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

    private static let maximumRememberedEarlyCancellations = 256

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
            active.task.cancel()
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
            active.task.cancel()
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

        models = models.filter { $0.key.directory != normalizedDirectory }
        MLX.Memory.clearCache()
        resumeIdleWaitersIfNeeded()
    }

    func clearCache() {
        MLX.Memory.clearCache()
    }

    func releaseForMemoryPressure(reason: String) {
        active?.task.cancel()
        let cancelled = pending
        pending.removeAll(keepingCapacity: false)
        for job in cancelled {
            job.emitter.error(
                id: job.command.id,
                message: "MLX generation cancelled: \(reason)"
            )
        }
        models.removeAll(keepingCapacity: false)
        MLX.Memory.clearCache()
        resumeIdleWaitersIfNeeded()
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
        guard active == nil, !pending.isEmpty else { return }
        let job = pending.removeFirst()
        let task = Task<Void, Never> { [weak self] in
            guard let self else { return }
            await self.run(job)
        }
        active = ActiveJob(job: job, task: task)
    }

    private func run(_ job: Job) async {
        defer {
            if active?.job.command.id == job.command.id {
                active = nil
            }
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
            MLX.Memory.clearCache()
            job.emitter.error(
                id: job.command.id,
                message: "MLX generation was cancelled"
            )
        } catch {
            MLX.Memory.clearCache()
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
        let generations = try await container.generate(
            input: prepared,
            parameters: parameters
        )

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
        var toolCalls: [OpenAIResponseToolCall] = []
        var completionInfo: GenerateCompletionInfo?
        var stoppedBySequence = false

        generationLoop: for await generation in generations {
            try Task.checkCancellation()
            switch generation {
            case .chunk(let text):
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
        let usage = OpenAIUsage(
            promptTokens: completionInfo?.promptTokenCount ?? 0,
            completionTokens: completionInfo?.generationTokenCount ?? 0,
            totalTokens:
                (completionInfo?.promptTokenCount ?? 0)
                + (completionInfo?.generationTokenCount ?? 0)
        )

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

    private func normalizedPath(_ path: String) -> String {
        URL(fileURLWithPath: path, isDirectory: true)
            .standardizedFileURL
            .resolvingSymlinksInPath()
            .path
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

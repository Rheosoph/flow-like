import CoreImage
import Foundation
import MLXLMCommon

public enum FlowLikeImageReference: Sendable, Equatable {
    case file(URL)
    case data(mimeType: String, bytes: Data)
    case remote(URL)
}

public struct FlowLikeMappedMessage: Sendable, Equatable {
    public let role: String
    public let text: String
    public let images: [FlowLikeImageReference]
}

public struct FlowLikeMappedRequest: Sendable, Equatable {
    public let messages: [FlowLikeMappedMessage]
    public let stopSequences: [String]
}

public enum FlowLikeMLXRequestMapper {
    public static func map(
        _ request: OpenAIChatCompletionRequest,
        modelKind: FlowLikeMLXModelKind
    ) throws -> FlowLikeMappedRequest {
        guard !request.messages.isEmpty else {
            throw FlowLikeMLXError.invalidRequest("messages must not be empty")
        }
        if let n = request.n, n != 1 {
            throw FlowLikeMLXError.unsupported("only n=1 is supported")
        }

        var mapped: [FlowLikeMappedMessage] = []
        mapped.reserveCapacity(request.messages.count)

        for message in request.messages {
            guard ["system", "user", "assistant", "tool"].contains(message.role) else {
                throw FlowLikeMLXError.unsupported(
                    "message role \"\(message.role)\" is not supported"
                )
            }
            if modelKind == .vlm,
                let toolCalls = message.toolCalls,
                !toolCalls.isEmpty
            {
                throw FlowLikeMLXError.unsupported(
                    "VLM assistant tool_calls in input history cannot be represented "
                        + "by mlx-swift-lm 3.31.3 structured multimodal Chat.Message; "
                        + "use an LLM MLX model for tool-loop turns"
                )
            }
            if modelKind == .vlm, message.toolCallID != nil {
                throw FlowLikeMLXError.unsupported(
                    "VLM tool_call_id history cannot be represented by mlx-swift-lm "
                        + "3.31.3 structured multimodal Chat.Message"
                )
            }

            var text = ""
            var images: [FlowLikeImageReference] = []

            switch message.content {
            case .none:
                break
            case .text(let value):
                text = value
            case .parts(let parts):
                for part in parts {
                    switch part.type {
                    case "text":
                        guard let partText = part.text else {
                            throw FlowLikeMLXError.invalidRequest(
                                "text content part is missing text"
                            )
                        }
                        text += partText
                    case "image_url":
                        guard let imageURL = part.imageURL else {
                            throw FlowLikeMLXError.invalidRequest(
                                "image_url content part is missing image_url"
                            )
                        }
                        images.append(try parseImageReference(imageURL.url))
                    default:
                        throw FlowLikeMLXError.unsupported(
                            "content part type \"\(part.type)\" is not supported"
                        )
                    }
                }
            }

            if modelKind == .llm, !images.isEmpty {
                throw FlowLikeMLXError.unsupported(
                    "image input requires model_kind \"vlm\""
                )
            }

            mapped.append(
                FlowLikeMappedMessage(role: message.role, text: text, images: images)
            )
        }

        let stopSequences = request.stop?.values ?? []
        if stopSequences.contains(where: \.isEmpty) {
            throw FlowLikeMLXError.invalidRequest("stop sequences must not be empty")
        }

        return FlowLikeMappedRequest(
            messages: mapped,
            stopSequences: stopSequences
        )
    }

    public static func parseImageReference(_ value: String) throws -> FlowLikeImageReference {
        if value.hasPrefix("data:") {
            return try parseDataImage(value)
        }

        if value.hasPrefix("/") {
            return .file(URL(fileURLWithPath: value, isDirectory: false).standardizedFileURL)
        }

        guard let url = URL(string: value), let scheme = url.scheme?.lowercased() else {
            throw FlowLikeMLXError.invalidImage(
                "only HTTP(S), file URLs, absolute file paths, and base64 data URLs are accepted"
            )
        }
        switch scheme {
        case "file":
            return .file(url.standardizedFileURL)
        case "http", "https":
            return .remote(url)
        default:
            throw FlowLikeMLXError.invalidImage(
                "URL scheme \"\(scheme)\" is not allowed; expected http, https, or file"
            )
        }
    }

    static func makeUserInput(
        mapped: FlowLikeMappedRequest,
        request: OpenAIChatCompletionRequest,
        modelKind: FlowLikeMLXModelKind
    ) async throws -> sending UserInput {
        let tools = try makeToolSpecs(request)
        if modelKind == .llm {
            return UserInput(
                messages: try makeRawLLMMessages(request.messages),
                tools: tools
            )
        }

        var chat: [Chat.Message] = []
        chat.reserveCapacity(mapped.messages.count)
        for message in mapped.messages {
            var images: [UserInput.Image] = []
            images.reserveCapacity(message.images.count)
            for image in message.images {
                images.append(try await makeMLXImage(image))
            }
            switch message.role {
            case "system":
                chat.append(.system(message.text, images: images))
            case "user":
                chat.append(.user(message.text, images: images))
            case "assistant":
                chat.append(.assistant(message.text, images: images))
            case "tool":
                guard images.isEmpty else {
                    throw FlowLikeMLXError.unsupported(
                        "tool messages cannot contain images"
                    )
                }
                chat.append(.tool(message.text))
            default:
                throw FlowLikeMLXError.unsupported(
                    "message role \"\(message.role)\" is not supported"
                )
            }
        }

        return UserInput(
            chat: chat,
            // ChatSession in mlx-swift-lm 3.31.3 uses the same conservative
            // 512x512 default. Individual VLM processors can further adapt it.
            processing: .init(resize: CGSize(width: 512, height: 512)),
            tools: tools
        )
    }

    static func makeGenerationParameters(
        _ request: OpenAIChatCompletionRequest
    ) throws -> GenerateParameters {
        var parameters = GenerateParameters()

        if let maxTokens = request.maxCompletionTokens ?? request.maxTokens {
            guard maxTokens > 0 else {
                throw FlowLikeMLXError.invalidRequest(
                    "max_completion_tokens/max_tokens must be greater than zero"
                )
            }
            parameters.maxTokens = maxTokens
        }
        if let temperature = request.temperature {
            guard temperature.isFinite, (0...2).contains(temperature) else {
                throw FlowLikeMLXError.invalidRequest(
                    "temperature must be finite and between 0 and 2"
                )
            }
            parameters.temperature = Float(temperature)
        }
        if let topP = request.topP {
            guard topP.isFinite, topP > 0, topP <= 1 else {
                throw FlowLikeMLXError.invalidRequest(
                    "top_p must be finite, greater than 0, and at most 1"
                )
            }
            parameters.topP = Float(topP)
        }
        if let topK = request.topK {
            guard topK >= 0 else {
                throw FlowLikeMLXError.invalidRequest("top_k must not be negative")
            }
            parameters.topK = topK
        }
        if let minP = request.minP {
            guard minP.isFinite, (0...1).contains(minP) else {
                throw FlowLikeMLXError.invalidRequest(
                    "min_p must be finite and between 0 and 1"
                )
            }
            parameters.minP = Float(minP)
        }
        if let maxKVSize = request.maxKVSize {
            guard maxKVSize > 0 else {
                throw FlowLikeMLXError.invalidRequest(
                    "max_kv_size must be greater than zero"
                )
            }
            parameters.maxKVSize = maxKVSize
        }
        if let kvBits = request.kvBits {
            guard kvBits == 4 || kvBits == 8 else {
                throw FlowLikeMLXError.invalidRequest("kv_bits must be 4 or 8")
            }
            parameters.kvBits = kvBits
        }
        if let kvGroupSize = request.kvGroupSize {
            guard kvGroupSize > 0 else {
                throw FlowLikeMLXError.invalidRequest(
                    "kv_group_size must be greater than zero"
                )
            }
            parameters.kvGroupSize = kvGroupSize
        }
        if let quantizedKVStart = request.quantizedKVStart {
            guard quantizedKVStart >= 0 else {
                throw FlowLikeMLXError.invalidRequest(
                    "quantized_kv_start must not be negative"
                )
            }
            parameters.quantizedKVStart = quantizedKVStart
        }
        if let repetitionPenalty = request.repetitionPenalty {
            guard repetitionPenalty.isFinite, repetitionPenalty > 0 else {
                throw FlowLikeMLXError.invalidRequest(
                    "repetition_penalty must be finite and greater than zero"
                )
            }
            parameters.repetitionPenalty = Float(repetitionPenalty)
        }
        if let repetitionContextSize = request.repetitionContextSize {
            guard repetitionContextSize >= 0 else {
                throw FlowLikeMLXError.invalidRequest(
                    "repetition_context_size must not be negative"
                )
            }
            parameters.repetitionContextSize = repetitionContextSize
        }
        if let presencePenalty = request.presencePenalty {
            guard presencePenalty.isFinite, (-2...2).contains(presencePenalty) else {
                throw FlowLikeMLXError.invalidRequest(
                    "presence_penalty must be finite and between -2 and 2"
                )
            }
            parameters.presencePenalty = Float(presencePenalty)
        }
        if let presenceContextSize = request.presenceContextSize {
            guard presenceContextSize >= 0 else {
                throw FlowLikeMLXError.invalidRequest(
                    "presence_context_size must not be negative"
                )
            }
            parameters.presenceContextSize = presenceContextSize
        }
        if let frequencyPenalty = request.frequencyPenalty {
            guard frequencyPenalty.isFinite, (-2...2).contains(frequencyPenalty) else {
                throw FlowLikeMLXError.invalidRequest(
                    "frequency_penalty must be finite and between -2 and 2"
                )
            }
            parameters.frequencyPenalty = Float(frequencyPenalty)
        }
        if let frequencyContextSize = request.frequencyContextSize {
            guard frequencyContextSize >= 0 else {
                throw FlowLikeMLXError.invalidRequest(
                    "frequency_context_size must not be negative"
                )
            }
            parameters.frequencyContextSize = frequencyContextSize
        }
        if let prefillStepSize = request.prefillStepSize {
            guard prefillStepSize > 0 else {
                throw FlowLikeMLXError.invalidRequest(
                    "prefill_step_size must be greater than zero"
                )
            }
            parameters.prefillStepSize = prefillStepSize
        }

        return parameters
    }

    private static func parseDataImage(_ value: String) throws -> FlowLikeImageReference {
        guard let comma = value.firstIndex(of: ",") else {
            throw FlowLikeMLXError.invalidImage("malformed data URL")
        }
        let metadata = String(value[value.index(value.startIndex, offsetBy: 5)..<comma])
        let payload = String(value[value.index(after: comma)...])
        let components = metadata.split(separator: ";", omittingEmptySubsequences: false)
        guard let mimeType = components.first.map(String.init),
            mimeType.hasPrefix("image/"),
            components.dropFirst().contains("base64")
        else {
            throw FlowLikeMLXError.invalidImage(
                "data URL must use an image MIME type and base64 encoding"
            )
        }
        guard let bytes = Data(base64Encoded: payload), !bytes.isEmpty else {
            throw FlowLikeMLXError.invalidImage("data URL contains invalid base64")
        }
        return .data(mimeType: mimeType, bytes: bytes)
    }

    private static func makeRawLLMMessages(
        _ messages: [OpenAIChatMessage]
    ) throws -> [Message] {
        try messages.map { message in
            var raw: Message = ["role": message.role]

            switch message.content {
            case .none:
                // Tool-calling assistant messages are allowed to have null content.
                // It has to reach the chat template as an empty string: the Jinja
                // bridge cannot convert NSNull and would fail the whole request.
                raw["content"] = ""
            case .text(let text):
                raw["content"] = text
            case .parts(let parts):
                var text = ""
                for part in parts {
                    guard part.type == "text", let partText = part.text else {
                        throw FlowLikeMLXError.unsupported(
                            "LLM content arrays may only contain text parts"
                        )
                    }
                    text += partText
                }
                raw["content"] = text
            }

            if let name = message.name {
                raw["name"] = name
            }
            if let toolCallID = message.toolCallID {
                raw["tool_call_id"] = toolCallID
            }
            if let toolCalls = message.toolCalls, !toolCalls.isEmpty {
                raw["tool_calls"] = toolCalls.map { toolCall -> any Sendable in
                    let function: [String: any Sendable] = [
                        "name": toolCall.function.name,
                        "arguments": toolCall.function.arguments,
                    ]
                    // Keep the concrete dictionary type explicit for Swift 6's
                    // `any Sendable` existential conversion.
                    let functionValue: [String: any Sendable] = function
                    var value: [String: any Sendable] = [
                        "type": toolCall.type ?? "function",
                        "function": functionValue,
                    ]
                    if let id = toolCall.id {
                        value["id"] = id
                    }
                    return value
                }
            }
            return raw
        }
    }

    private static func makeMLXImage(
        _ reference: FlowLikeImageReference
    ) async throws -> UserInput.Image {
        switch reference {
        case .file(let url):
            var isDirectory: ObjCBool = false
            guard FileManager.default.fileExists(
                atPath: url.path,
                isDirectory: &isDirectory
            ), !isDirectory.boolValue
            else {
                throw FlowLikeMLXError.invalidImage(
                    "file does not exist: \(url.path)"
                )
            }
            return .url(url)
        case .data(_, let bytes):
            guard let image = CIImage(data: bytes) else {
                throw FlowLikeMLXError.invalidImage(
                    "base64 payload is not a decodable image"
                )
            }
            return .ciImage(image)
        case .remote(let url):
            let bytes = try await downloadRemoteImage(url)
            guard let image = CIImage(data: bytes) else {
                throw FlowLikeMLXError.invalidImage(
                    "HTTP(S) response is not a decodable image"
                )
            }
            return .ciImage(image)
        }
    }

    static let maximumRemoteImageBytes = 20 * 1024 * 1024

    private static func downloadRemoteImage(_ url: URL) async throws -> Data {
        var request = URLRequest(url: url)
        request.timeoutInterval = 20
        request.cachePolicy = .reloadIgnoringLocalCacheData
        request.setValue("image/*", forHTTPHeaderField: "Accept")

        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 20
        configuration.timeoutIntervalForResource = 30
        configuration.urlCache = nil
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }

        let (bytes, response) = try await session.bytes(for: request)
        guard let http = response as? HTTPURLResponse,
            (200...299).contains(http.statusCode)
        else {
            let status = (response as? HTTPURLResponse)?.statusCode ?? 0
            throw FlowLikeMLXError.invalidImage(
                "HTTP(S) image request failed with status \(status)"
            )
        }
        guard let finalURL = http.url,
            ["http", "https"].contains(finalURL.scheme?.lowercased() ?? "")
        else {
            throw FlowLikeMLXError.invalidImage(
                "HTTP(S) image redirect resolved to a disallowed URL scheme"
            )
        }

        let contentType =
            http.value(forHTTPHeaderField: "Content-Type")?
            .split(separator: ";", maxSplits: 1)
            .first?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        guard let contentType, contentType.hasPrefix("image/") else {
            throw FlowLikeMLXError.invalidImage(
                "HTTP(S) response Content-Type must be image/*"
            )
        }
        if http.expectedContentLength > Int64(maximumRemoteImageBytes) {
            throw FlowLikeMLXError.invalidImage(
                "HTTP(S) image exceeds the \(maximumRemoteImageBytes)-byte limit"
            )
        }

        var data = Data()
        if http.expectedContentLength > 0 {
            data.reserveCapacity(
                min(Int(http.expectedContentLength), maximumRemoteImageBytes)
            )
        }
        for try await byte in bytes {
            guard data.count < maximumRemoteImageBytes else {
                throw FlowLikeMLXError.invalidImage(
                    "HTTP(S) image exceeds the \(maximumRemoteImageBytes)-byte limit"
                )
            }
            data.append(byte)
        }
        guard !data.isEmpty else {
            throw FlowLikeMLXError.invalidImage("HTTP(S) image response is empty")
        }
        return data
    }

    private static func makeToolSpecs(
        _ request: OpenAIChatCompletionRequest
    ) throws -> [ToolSpec]? {
        guard let tools = request.tools, !tools.isEmpty else {
            return nil
        }

        var selected = tools
        if let toolChoice = request.toolChoice {
            switch toolChoice {
            case .string("none"):
                return nil
            case .string("auto"), .string("required"):
                break
            case .object(let object):
                guard case .object(let function)? = object["function"],
                    case .string(let name)? = function["name"]
                else {
                    throw FlowLikeMLXError.invalidRequest(
                        "tool_choice object must contain function.name"
                    )
                }
                selected = tools.filter { $0.function.name == name }
                guard !selected.isEmpty else {
                    throw FlowLikeMLXError.invalidRequest(
                        "tool_choice references unknown function \"\(name)\""
                    )
                }
            default:
                throw FlowLikeMLXError.unsupported(
                    "tool_choice must be none, auto, required, or a named function"
                )
            }
        }

        return try selected.map { tool in
            guard tool.type == "function" else {
                throw FlowLikeMLXError.unsupported(
                    "only function tools are supported"
                )
            }
            guard !tool.function.name.isEmpty else {
                throw FlowLikeMLXError.invalidRequest(
                    "tool function name must not be empty"
                )
            }

            var function: [String: any Sendable] = [
                "name": tool.function.name
            ]
            if let description = tool.function.description {
                function["description"] = description
            }
            if let parameters = tool.function.parameters {
                function["parameters"] = parameters.sendableValue
            }

            return [
                "type": "function",
                "function": function,
            ] as ToolSpec
        }
    }
}

/// Keeps possible stop-sequence prefixes buffered so a stop sequence split
/// across model chunks is never emitted to the caller.
struct StopSequenceFilter {
    private let stops: [String]
    private var pending = ""
    private(set) var didStop = false

    init(stops: [String]) {
        self.stops = stops
    }

    mutating func consume(_ chunk: String) -> String {
        guard !didStop else { return "" }
        guard !stops.isEmpty else { return chunk }

        pending += chunk
        if let match = firstStopMatch(in: pending) {
            let output = String(pending[..<match.lowerBound])
            pending.removeAll(keepingCapacity: false)
            didStop = true
            return output
        }

        let maximumPrefix = max(0, (stops.map(\.count).max() ?? 1) - 1)
        let maximumCandidate = min(maximumPrefix, pending.count)
        var retainedCount = 0
        if maximumCandidate > 0 {
            for length in stride(from: maximumCandidate, through: 1, by: -1) {
                let suffix = String(pending.suffix(length))
                if stops.contains(where: { $0.hasPrefix(suffix) }) {
                    retainedCount = length
                    break
                }
            }
        }

        let outputCount = pending.count - retainedCount
        let output = String(pending.prefix(outputCount))
        pending = String(pending.suffix(retainedCount))
        return output
    }

    mutating func finish() -> String {
        guard !didStop else { return "" }
        defer { pending.removeAll(keepingCapacity: false) }
        return pending
    }

    private func firstStopMatch(in text: String) -> Range<String.Index>? {
        stops
            .compactMap { text.range(of: $0) }
            .min { lhs, rhs in lhs.lowerBound < rhs.lowerBound }
    }
}

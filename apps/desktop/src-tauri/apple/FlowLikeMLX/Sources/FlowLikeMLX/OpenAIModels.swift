import Foundation

public enum FlowLikeMLXError: LocalizedError, Sendable {
    case invalidRequest(String)
    case unsupported(String)
    case invalidModelDirectory(String)
    case invalidImage(String)
    case unparsableToolCall(String)

    public var errorDescription: String? {
        switch self {
        case .invalidRequest(let message):
            "Invalid request: \(message)"
        case .unsupported(let message):
            "Unsupported request: \(message)"
        case .invalidModelDirectory(let message):
            "Invalid model directory: \(message)"
        case .invalidImage(let message):
            "Invalid image: \(message)"
        case .unparsableToolCall(let message):
            "Unparsable tool call: \(message)"
        }
    }
}

public enum FlowLikeMLXModelKind: String, Codable, Sendable, Hashable {
    case llm
    case vlm
}

/// The command envelope shared by the C ABI and the macOS NDJSON executable.
///
/// `modelDirectory`, `modelKind`, and `request` are required for `generate`.
/// They remain optional at the decoding layer so lifecycle commands can use the
/// same envelope.
public struct FlowLikeMLXCommand: Codable, Sendable {
    public let id: String
    public let command: String
    public let modelDirectory: String?
    public let modelKind: FlowLikeMLXModelKind?
    public let request: OpenAIChatCompletionRequest?

    enum CodingKeys: String, CodingKey {
        case id
        case command
        case modelDirectory = "model_directory"
        case modelKind = "model_kind"
        case request
    }

    public func validatedGenerate() throws
        -> (modelDirectory: String, modelKind: FlowLikeMLXModelKind, request: OpenAIChatCompletionRequest)
    {
        guard command == "generate" else {
            throw FlowLikeMLXError.invalidRequest(
                "expected command \"generate\", got \"\(command)\""
            )
        }
        guard !id.isEmpty else {
            throw FlowLikeMLXError.invalidRequest("id must not be empty")
        }
        guard let modelDirectory, !modelDirectory.isEmpty else {
            throw FlowLikeMLXError.invalidRequest("model_directory is required")
        }
        guard let modelKind else {
            throw FlowLikeMLXError.invalidRequest("model_kind is required")
        }
        guard let request else {
            throw FlowLikeMLXError.invalidRequest("request is required")
        }
        return (modelDirectory, modelKind, request)
    }
}

/// A Codable, Sendable representation of arbitrary JSON used for tool schemas.
public enum FlowLikeJSONValue: Codable, Sendable, Equatable {
    case null
    case bool(Bool)
    case integer(Int)
    case number(Double)
    case string(String)
    case array([FlowLikeJSONValue])
    case object([String: FlowLikeJSONValue])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Int.self) {
            self = .integer(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([FlowLikeJSONValue].self) {
            self = .array(value)
        } else if let value = try? container.decode([String: FlowLikeJSONValue].self) {
            self = .object(value)
        } else {
            throw DecodingError.dataCorruptedError(
                in: container,
                debugDescription: "Unsupported JSON value"
            )
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null:
            try container.encodeNil()
        case .bool(let value):
            try container.encode(value)
        case .integer(let value):
            try container.encode(value)
        case .number(let value):
            try container.encode(value)
        case .string(let value):
            try container.encode(value)
        case .array(let value):
            try container.encode(value)
        case .object(let value):
            try container.encode(value)
        }
    }

    var sendableValue: any Sendable {
        switch self {
        case .null:
            return NSNull()
        case .bool(let value):
            return value
        case .integer(let value):
            return value
        case .number(let value):
            return value
        case .string(let value):
            return value
        case .array(let values):
            return values.map(\.sendableValue)
        case .object(let values):
            var result: [String: any Sendable] = [:]
            for (key, value) in values {
                result[key] = value.sendableValue
            }
            return result
        }
    }
}

public enum OpenAIMessageContent: Codable, Sendable, Equatable {
    case text(String)
    case parts([OpenAIContentPart])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let text = try? container.decode(String.self) {
            self = .text(text)
        } else {
            self = .parts(try container.decode([OpenAIContentPart].self))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .text(let text):
            try container.encode(text)
        case .parts(let parts):
            try container.encode(parts)
        }
    }
}

public enum OpenAIImageURLPayload: Codable, Sendable, Equatable {
    case url(String, detail: String?)

    private struct ObjectValue: Codable {
        let url: String
        let detail: String?
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let url = try? container.decode(String.self) {
            self = .url(url, detail: nil)
        } else {
            let object = try container.decode(ObjectValue.self)
            self = .url(object.url, detail: object.detail)
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .url(let url, let detail):
            try container.encode(ObjectValue(url: url, detail: detail))
        }
    }

    public var url: String {
        switch self {
        case .url(let url, _):
            url
        }
    }
}

public struct OpenAIContentPart: Codable, Sendable, Equatable {
    public let type: String
    public let text: String?
    public let imageURL: OpenAIImageURLPayload?

    enum CodingKeys: String, CodingKey {
        case type
        case text
        case imageURL = "image_url"
    }
}

public struct OpenAIInputToolCall: Codable, Sendable, Equatable {
    public struct Function: Codable, Sendable, Equatable {
        public let name: String
        public let arguments: String
    }

    public let id: String?
    public let type: String?
    public let function: Function
}

public struct OpenAIChatMessage: Codable, Sendable, Equatable {
    public let role: String
    public let content: OpenAIMessageContent?
    public let name: String?
    public let toolCallID: String?
    public let toolCalls: [OpenAIInputToolCall]?

    enum CodingKeys: String, CodingKey {
        case role
        case content
        case name
        case toolCallID = "tool_call_id"
        case toolCalls = "tool_calls"
    }
}

public struct OpenAITool: Codable, Sendable, Equatable {
    public struct Function: Codable, Sendable, Equatable {
        public let name: String
        public let description: String?
        public let parameters: FlowLikeJSONValue?
    }

    public let type: String
    public let function: Function
}

public enum OpenAIStop: Codable, Sendable, Equatable {
    case one(String)
    case many([String])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let value = try? container.decode(String.self) {
            self = .one(value)
        } else {
            self = .many(try container.decode([String].self))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .one(let value):
            try container.encode(value)
        case .many(let values):
            try container.encode(values)
        }
    }

    public var values: [String] {
        switch self {
        case .one(let value):
            [value]
        case .many(let values):
            values
        }
    }
}

public struct OpenAIStreamOptions: Codable, Sendable, Equatable {
    public let includeUsage: Bool?

    enum CodingKeys: String, CodingKey {
        case includeUsage = "include_usage"
    }
}

/// The supported Chat Completions request fields. Unknown JSON fields are
/// intentionally ignored by Codable for forward compatibility.
public struct OpenAIChatCompletionRequest: Codable, Sendable, Equatable {
    public let model: String?
    public let messages: [OpenAIChatMessage]
    public let temperature: Double?
    public let topP: Double?
    public let maxTokens: Int?
    public let maxCompletionTokens: Int?
    public let n: Int?
    public let stream: Bool?
    public let streamOptions: OpenAIStreamOptions?
    public let stop: OpenAIStop?
    public let tools: [OpenAITool]?
    public let toolChoice: FlowLikeJSONValue?
    public let presencePenalty: Double?
    public let frequencyPenalty: Double?

    // Optional local-provider extensions.
    public let topK: Int?
    public let minP: Double?
    public let maxKVSize: Int?
    public let kvBits: Int?
    public let kvGroupSize: Int?
    public let quantizedKVStart: Int?
    public let repetitionPenalty: Double?
    public let repetitionContextSize: Int?
    public let presenceContextSize: Int?
    public let frequencyContextSize: Int?
    public let prefillStepSize: Int?

    enum CodingKeys: String, CodingKey {
        case model
        case messages
        case temperature
        case topP = "top_p"
        case maxTokens = "max_tokens"
        case maxCompletionTokens = "max_completion_tokens"
        case n
        case stream
        case streamOptions = "stream_options"
        case stop
        case tools
        case toolChoice = "tool_choice"
        case presencePenalty = "presence_penalty"
        case frequencyPenalty = "frequency_penalty"
        case topK = "top_k"
        case minP = "min_p"
        case maxKVSize = "max_kv_size"
        case kvBits = "kv_bits"
        case kvGroupSize = "kv_group_size"
        case quantizedKVStart = "quantized_kv_start"
        case repetitionPenalty = "repetition_penalty"
        case repetitionContextSize = "repetition_context_size"
        case presenceContextSize = "presence_context_size"
        case frequencyContextSize = "frequency_context_size"
        case prefillStepSize = "prefill_step_size"
    }
}

public struct OpenAIUsage: Codable, Sendable, Equatable {
    public let promptTokens: Int
    public let completionTokens: Int
    public let totalTokens: Int

    enum CodingKeys: String, CodingKey {
        case promptTokens = "prompt_tokens"
        case completionTokens = "completion_tokens"
        case totalTokens = "total_tokens"
    }
}

public struct OpenAIResponseToolCall: Codable, Sendable, Equatable {
    public struct Function: Codable, Sendable, Equatable {
        public let name: String
        public let arguments: String
    }

    public let id: String
    public let type: String
    public let function: Function
}

public struct OpenAIAssistantMessage: Codable, Sendable, Equatable {
    public let role: String
    public let content: String?
    public let toolCalls: [OpenAIResponseToolCall]?

    enum CodingKeys: String, CodingKey {
        case role
        case content
        case toolCalls = "tool_calls"
    }
}

public struct OpenAICompletionChoice: Codable, Sendable, Equatable {
    public let index: Int
    public let message: OpenAIAssistantMessage
    public let finishReason: String

    enum CodingKeys: String, CodingKey {
        case index
        case message
        case finishReason = "finish_reason"
    }
}

public struct OpenAIChatCompletionResponse: Codable, Sendable, Equatable {
    public let id: String
    public let object: String
    public let created: Int
    public let model: String
    public let choices: [OpenAICompletionChoice]
    public let usage: OpenAIUsage?
}

public struct OpenAIChunkToolCall: Codable, Sendable, Equatable {
    public struct Function: Codable, Sendable, Equatable {
        public let name: String?
        public let arguments: String?
    }

    public let index: Int
    public let id: String?
    public let type: String?
    public let function: Function
}

public struct OpenAIChunkDelta: Codable, Sendable, Equatable {
    public let role: String?
    public let content: String?
    public let toolCalls: [OpenAIChunkToolCall]?

    enum CodingKeys: String, CodingKey {
        case role
        case content
        case toolCalls = "tool_calls"
    }
}

public struct OpenAIChunkChoice: Codable, Sendable, Equatable {
    public let index: Int
    public let delta: OpenAIChunkDelta
    public let finishReason: String?

    enum CodingKeys: String, CodingKey {
        case index
        case delta
        case finishReason = "finish_reason"
    }
}

public struct OpenAIChatCompletionChunk: Codable, Sendable, Equatable {
    public let id: String
    public let object: String
    public let created: Int
    public let model: String
    public let choices: [OpenAIChunkChoice]
    public let usage: OpenAIUsage?
}

private struct FlowLikeMLXEventEnvelope<Payload: Encodable>: Encodable {
    let id: String
    let event: String
    let data: Payload?
    let error: String?
}

private struct FlowLikeEmptyPayload: Codable {}

public enum FlowLikeMLXEventCodec {
    public static func encode<Payload: Encodable>(
        id: String,
        event: String,
        data: Payload
    ) -> String {
        encodeEnvelope(
            FlowLikeMLXEventEnvelope(id: id, event: event, data: data, error: nil)
        )
    }

    public static func error(id: String, message: String) -> String {
        encodeEnvelope(
            FlowLikeMLXEventEnvelope<FlowLikeEmptyPayload>(
                id: id,
                event: "error",
                data: nil,
                error: message
            )
        )
    }

    private static func encodeEnvelope<Payload: Encodable>(
        _ envelope: FlowLikeMLXEventEnvelope<Payload>
    ) -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.withoutEscapingSlashes]
        do {
            let data = try encoder.encode(envelope)
            return String(decoding: data, as: UTF8.self)
        } catch {
            // All event payloads are Codable value types. This is a last-resort
            // protocol-safe fallback should an encoder ever reject a value.
            return #"{"id":"unknown","event":"error","error":"event encoding failed"}"#
        }
    }
}

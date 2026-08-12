@testable import FlowLikeMLX
import MLXLMCommon
import XCTest

final class RequestMappingTests: XCTestCase {
    func testLLMToolHistoryUsesRawMessages() async throws {
        let request = try decodeRequest(
            """
            {
              "model": "local-mlx",
              "messages": [
                {"role": "user", "content": "weather?"},
                {
                  "role": "assistant",
                  "content": null,
                  "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                      "name": "weather",
                      "arguments": "{\\"city\\":\\"Berlin\\"}"
                    }
                  }]
                },
                {
                  "role": "tool",
                  "tool_call_id": "call_1",
                  "content": "sunny"
                }
              ],
              "tools": [{
                "type": "function",
                "function": {
                  "name": "weather",
                  "parameters": {
                    "type": "object",
                    "properties": {"city": {"type": "string"}}
                  }
                }
              }]
            }
            """
        )

        let mapped = try FlowLikeMLXRequestMapper.map(request, modelKind: .llm)
        let input = try await FlowLikeMLXRequestMapper.makeUserInput(
            mapped: mapped,
            request: request,
            modelKind: .llm
        )

        guard case .messages(let messages) = input.prompt else {
            return XCTFail("LLM history must use raw MLX messages")
        }
        XCTAssertEqual(messages.count, 3)
        XCTAssertEqual(messages[2]["tool_call_id"] as? String, "call_1")

        // Null assistant content has to survive as an empty string: the Jinja
        // bridge rejects NSNull and would fail every tool-loop turn.
        XCTAssertFalse(messages[1]["content"] is NSNull)
        XCTAssertEqual(messages[1]["content"] as? String, "")

        let calls = try XCTUnwrap(messages[1]["tool_calls"] as? [any Sendable])
        let call = try XCTUnwrap(calls.first as? [String: any Sendable])
        XCTAssertEqual(call["id"] as? String, "call_1")
        let function = try XCTUnwrap(
            call["function"] as? [String: any Sendable]
        )
        XCTAssertEqual(function["name"] as? String, "weather")
        XCTAssertEqual(
            function["arguments"] as? String,
            #"{"city":"Berlin"}"#
        )
    }

    func testVLMToolHistoryReturnsPreciseLimitation() throws {
        let request = try decodeRequest(
            """
            {
              "messages": [{
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                  "id": "call_1",
                  "type": "function",
                  "function": {"name": "weather", "arguments": "{}"}
                }]
              }]
            }
            """
        )

        XCTAssertThrowsError(
            try FlowLikeMLXRequestMapper.map(request, modelKind: .vlm)
        ) { error in
            XCTAssertTrue(error.localizedDescription.contains("VLM assistant tool_calls"))
        }
    }

    func testImageReferenceAcceptsOnlyDocumentedSchemes() throws {
        guard case .remote(let remote) =
            try FlowLikeMLXRequestMapper.parseImageReference(
                "https://example.com/image.png"
            )
        else {
            return XCTFail("expected remote image")
        }
        XCTAssertEqual(remote.scheme, "https")

        guard case .file(let file) =
            try FlowLikeMLXRequestMapper.parseImageReference("file:///tmp/image.png")
        else {
            return XCTFail("expected file image")
        }
        XCTAssertEqual(file.path, "/tmp/image.png")

        guard case .data(let mimeType, let bytes) =
            try FlowLikeMLXRequestMapper.parseImageReference(
                "data:image/png;base64,AQID"
            )
        else {
            return XCTFail("expected data image")
        }
        XCTAssertEqual(mimeType, "image/png")
        XCTAssertEqual(bytes, Data([1, 2, 3]))

        XCTAssertThrowsError(
            try FlowLikeMLXRequestMapper.parseImageReference(
                "ftp://example.com/image.png"
            )
        )
    }

    func testStopSequenceAcrossChunksIsNeverEmitted() {
        var filter = StopSequenceFilter(stops: ["</stop>"])
        XCTAssertEqual(filter.consume("answer</st"), "answer")
        XCTAssertEqual(filter.consume("op>ignored"), "")
        XCTAssertTrue(filter.didStop)
        XCTAssertEqual(filter.finish(), "")
    }

    func testGenerationParametersMapIndependentContextSizes() throws {
        let request = try decodeRequest(
            """
            {
              "messages": [{"role": "user", "content": "hello"}],
              "presence_context_size": 17,
              "frequency_context_size": 23,
              "max_kv_size": 1024
            }
            """
        )
        let parameters = try FlowLikeMLXRequestMapper.makeGenerationParameters(request)
        XCTAssertEqual(parameters.presenceContextSize, 17)
        XCTAssertEqual(parameters.frequencyContextSize, 23)
        XCTAssertEqual(parameters.maxKVSize, 1024)
    }

    func testGenerationParametersUseSafePlatformPrefillDefault() throws {
        let request = try decodeRequest(
            """
            {
              "messages": [{"role": "user", "content": "hello"}]
            }
            """
        )
        let parameters = try FlowLikeMLXRequestMapper.makeGenerationParameters(request)

        #if os(iOS)
            XCTAssertEqual(parameters.prefillStepSize, 128)
        #else
            XCTAssertNil(parameters.prefillStepSize)
        #endif
    }

    func testGenerationParametersHonorExplicitPrefillStepSize() throws {
        let request = try decodeRequest(
            """
            {
              "messages": [{"role": "user", "content": "hello"}],
              "prefill_step_size": 256
            }
            """
        )
        let parameters = try FlowLikeMLXRequestMapper.makeGenerationParameters(request)

        XCTAssertEqual(parameters.prefillStepSize, 256)
    }

    private func decodeRequest(_ json: String) throws -> OpenAIChatCompletionRequest {
        try JSONDecoder().decode(
            OpenAIChatCompletionRequest.self,
            from: Data(json.utf8)
        )
    }
}

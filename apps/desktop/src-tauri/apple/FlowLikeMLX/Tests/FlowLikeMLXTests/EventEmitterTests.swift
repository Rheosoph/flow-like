@testable import FlowLikeMLX
import Foundation
import XCTest

final class EventEmitterTests: XCTestCase {
    func testTerminalEventClosesEmitter() throws {
        let recorder = EventRecorder()
        let emitter = FlowLikeMLXEventEmitter { event in
            recorder.append(event)
        }

        emitter.chunk(id: "request", data: chunk(content: "one"))
        emitter.error(id: "request", message: "cancelled")
        emitter.chunk(id: "request", data: chunk(content: "too late"))
        emitter.error(id: "request", message: "also too late")

        let events = recorder.values
        XCTAssertEqual(events.count, 2)
        let terminal = try JSONSerialization.jsonObject(
            with: Data(events[1].utf8)
        ) as? [String: Any]
        XCTAssertEqual(terminal?["event"] as? String, "error")
        XCTAssertEqual(terminal?["id"] as? String, "request")
    }

    private func chunk(content: String) -> OpenAIChatCompletionChunk {
        OpenAIChatCompletionChunk(
            id: "chatcmpl-request",
            object: "chat.completion.chunk",
            created: 1,
            model: "test",
            choices: [
                OpenAIChunkChoice(
                    index: 0,
                    delta: OpenAIChunkDelta(
                        role: nil,
                        content: content,
                        toolCalls: nil
                    ),
                    finishReason: nil
                )
            ],
            usage: nil
        )
    }
}

private final class EventRecorder: @unchecked Sendable {
    private let lock = NSLock()
    private var storage: [String] = []

    var values: [String] {
        lock.lock()
        defer { lock.unlock() }
        return storage
    }

    func append(_ event: String) {
        lock.lock()
        storage.append(event)
        lock.unlock()
    }
}

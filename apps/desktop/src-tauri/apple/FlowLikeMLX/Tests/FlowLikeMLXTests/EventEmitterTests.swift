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

    func testJoinedProducerDoesNotReturnBeforeCancelledWorkFinishes() async {
        let started = expectation(description: "producer started")
        let probe = CompletionProbe()
        let producer = Task<Void, Never> {
            started.fulfill()
            while !Task.isCancelled {
                await Task.yield()
            }
            // Model generation performs its final GPU synchronization after it
            // observes cancellation. Simulate work in that settlement window.
            for _ in 0..<1_000 {
                await Task.yield()
            }
            probe.markFinished()
        }

        await fulfillment(of: [started], timeout: 1)
        await cancelAndJoinMLXProducer(producer)

        XCTAssertTrue(probe.isFinished)
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

private final class CompletionProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var finished = false

    var isFinished: Bool {
        lock.lock()
        defer { lock.unlock() }
        return finished
    }

    func markFinished() {
        lock.lock()
        finished = true
        lock.unlock()
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

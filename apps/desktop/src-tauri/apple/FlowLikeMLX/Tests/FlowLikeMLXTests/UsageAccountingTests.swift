@testable import FlowLikeMLX
import MLXLMCommon
import XCTest

final class UsageAccountingTests: XCTestCase {
    private func info(prompt: Int, generation: Int) -> GenerateCompletionInfo {
        GenerateCompletionInfo(
            promptTokenCount: prompt,
            generationTokenCount: generation,
            promptTime: 0.1,
            generationTime: 0.2,
            stopReason: .stop
        )
    }

    func testRuntimeInfoWinsOverLocalCounts() {
        let usage = FlowLikeMLXUsageAccounting.make(
            info: info(prompt: 43, generation: 40),
            fallbackPromptTokens: 7,
            fallbackCompletionTokens: 9
        )

        XCTAssertEqual(usage.promptTokens, 43)
        XCTAssertEqual(usage.completionTokens, 40)
        XCTAssertEqual(usage.totalTokens, 83)
    }

    func testStopSequenceFallbackReportsLocalCounts() {
        let usage = FlowLikeMLXUsageAccounting.make(
            info: nil,
            fallbackPromptTokens: 43,
            fallbackCompletionTokens: 4
        )

        XCTAssertEqual(usage.promptTokens, 43)
        XCTAssertEqual(usage.completionTokens, 4)
        XCTAssertEqual(usage.totalTokens, 47)
    }

    func testFallbackStillCountsPromptWhenNothingWasGenerated() {
        let usage = FlowLikeMLXUsageAccounting.make(
            info: nil,
            fallbackPromptTokens: 12,
            fallbackCompletionTokens: 0
        )

        XCTAssertEqual(usage.promptTokens, 12)
        XCTAssertEqual(usage.completionTokens, 0)
        XCTAssertEqual(usage.totalTokens, 12)
    }

    func testNegativeCountsNeverLeakIntoUsage() {
        let usage = FlowLikeMLXUsageAccounting.make(
            info: nil,
            fallbackPromptTokens: -3,
            fallbackCompletionTokens: -8
        )

        XCTAssertEqual(usage.promptTokens, 0)
        XCTAssertEqual(usage.completionTokens, 0)
        XCTAssertEqual(usage.totalTokens, 0)
    }
}

final class OutputGuardTests: XCTestCase {
    func testMalformedToolCallThatAteEveryTokenIsDetected() {
        XCTAssertTrue(
            FlowLikeMLXOutputGuard.droppedEveryToken(
                content: "",
                toolCallCount: 0,
                completionTokens: 21
            )
        )
    }

    func testTextOutputIsNeverTreatedAsDropped() {
        XCTAssertFalse(
            FlowLikeMLXOutputGuard.droppedEveryToken(
                content: "hello",
                toolCallCount: 0,
                completionTokens: 21
            )
        )
    }

    func testToolCallOutputIsNeverTreatedAsDropped() {
        XCTAssertFalse(
            FlowLikeMLXOutputGuard.droppedEveryToken(
                content: "",
                toolCallCount: 1,
                completionTokens: 21
            )
        )
    }

    func testModelThatGeneratedNothingIsNotAnError() {
        XCTAssertFalse(
            FlowLikeMLXOutputGuard.droppedEveryToken(
                content: "",
                toolCallCount: 0,
                completionTokens: 0
            )
        )
    }

    func testWhitespaceOnlyAnswerIsNotTreatedAsDropped() {
        XCTAssertFalse(
            FlowLikeMLXOutputGuard.droppedEveryToken(
                content: " ",
                toolCallCount: 0,
                completionTokens: 1
            )
        )
    }
}

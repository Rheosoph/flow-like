@testable import FlowLikeMLX
import XCTest

final class MemoryWarningPolicyTests: XCTestCase {
    func testFirstWarningDefersAndRepeatedWarningEscalates() {
        var policy = FlowLikeMLXMemoryWarningPolicy()

        XCTAssertEqual(
            policy.action(activeRequestID: "request-a"),
            .releaseWhenIdle
        )
        XCTAssertEqual(
            policy.action(activeRequestID: "request-a"),
            .cancelAndRelease
        )
    }

    func testCompletedRequestGetsNewFirstWarningGrace() {
        var policy = FlowLikeMLXMemoryWarningPolicy()
        _ = policy.action(activeRequestID: "request-a")

        policy.requestDidFinish("request-a")

        XCTAssertEqual(
            policy.action(activeRequestID: "request-a"),
            .releaseWhenIdle
        )
    }

    func testIdleWarningReleasesImmediatelyAndResetsEscalation() {
        var policy = FlowLikeMLXMemoryWarningPolicy()
        _ = policy.action(activeRequestID: "request-a")

        XCTAssertEqual(
            policy.action(activeRequestID: nil),
            .releaseImmediately
        )
        XCTAssertEqual(
            policy.action(activeRequestID: "request-a"),
            .releaseWhenIdle
        )
    }

    func testDifferentRequestGetsItsOwnFirstWarningGrace() {
        var policy = FlowLikeMLXMemoryWarningPolicy()
        _ = policy.action(activeRequestID: "request-a")

        XCTAssertEqual(
            policy.action(activeRequestID: "request-b"),
            .releaseWhenIdle
        )
    }
}

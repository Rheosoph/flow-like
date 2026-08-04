@testable import FlowLikeMLX
import XCTest

final class ForegroundGateTests: XCTestCase {
    func testDeactivationClosesAdmissionUntilActivation() {
        let gate = FlowLikeMLXForegroundGate()

        XCTAssertTrue(gate.allowsExecution)
        let deactivation = gate.deactivate()
        XCTAssertFalse(gate.allowsExecution)
        XCTAssertTrue(gate.isCurrentDeactivation(deactivation))

        gate.activate()
        XCTAssertTrue(gate.allowsExecution)
        XCTAssertFalse(gate.isCurrentDeactivation(deactivation))
    }

    func testNewDeactivationInvalidatesEarlierLifecycleTask() {
        let gate = FlowLikeMLXForegroundGate()
        let first = gate.deactivate()
        gate.activate()
        let second = gate.deactivate()

        XCTAssertFalse(gate.isCurrentDeactivation(first))
        XCTAssertTrue(gate.isCurrentDeactivation(second))
    }
}

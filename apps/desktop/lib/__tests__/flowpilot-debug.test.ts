import {
	flowPilotDebugLog,
	isFlowPilotDebugEnabled,
	stripFlowPilotDebugReport,
} from "@flow-like/flow-like-ui/lib/flowpilot-debug";
import { describe, expect, test, vi } from "vitest";

describe("FlowPilot development diagnostics", () => {
	test("enables diagnostics only for development builds", () => {
		expect(isFlowPilotDebugEnabled("development")).toBe(true);
		expect(isFlowPilotDebugEnabled("production")).toBe(false);
		expect(isFlowPilotDebugEnabled("test")).toBe(false);
		expect(isFlowPilotDebugEnabled(undefined)).toBe(false);
	});

	test("does not write internal console logs outside a development build", () => {
		const debug = vi.spyOn(console, "debug").mockImplementation(() => {});
		flowPilotDebugLog("sensitive FlowPilot diagnostic", { token: "secret" });
		expect(debug).not.toHaveBeenCalled();
		debug.mockRestore();
	});

	test("strips reports for production persistence without mutating the message", () => {
		const message = {
			id: "assistant-1",
			debug_report: { schema: "flowpilot.run-report/v1", events: [] },
		};
		const sanitized = stripFlowPilotDebugReport(message, false);
		expect(sanitized).toEqual({ id: "assistant-1" });
		expect(message.debug_report).toBeDefined();
		expect(stripFlowPilotDebugReport(message, true)).toBe(message);
	});
});

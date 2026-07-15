import {
	applyStreamEvent,
	createStreamAccumulator,
	orderedSteps,
} from "@flow-like/flow-like-ui/state/global-chat/copilot-stream-steps";
import { describe, expect, test } from "vitest";

describe("copilot stream steps", () => {
	test.each([
		["error", "failed"],
		["failed", "failed"],
		["timeout", "failed"],
		["timed_out", "failed"],
		["success", "done"],
		["ok", "done"],
		["completed", "done"],
		["queued", "done"],
		["done", "done"],
	] as const)(
		"normalizes a terminal tool status of %s to %s",
		(status, expectedStatus) => {
			const accumulator = createStreamAccumulator();
			applyStreamEvent(accumulator, {
				type: "tool_start",
				data: {
					tool_call_id: `tool-${status}`,
					tool_name: "flowpilot_board",
					arguments: { instruction: "Build and validate the board" },
				},
			});
			applyStreamEvent(accumulator, {
				type: "tool_end",
				data: {
					tool_call_id: `tool-${status}`,
					tool_name: "flowpilot_board",
					status,
				},
			});

			expect(orderedSteps(accumulator)).toEqual([
				expect.objectContaining({
					id: `tool-${status}`,
					title: "Using flowpilot_board",
					status: expectedStatus,
				}),
			]);
			expect(accumulator.currentStepId).toBeUndefined();
		},
	);
});

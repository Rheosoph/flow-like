import { describe, expect, test } from "bun:test";
import {
	applyStreamEvent,
	createStreamAccumulator,
	toolEndPlanStepStatus,
} from "./copilot-stream-steps";

describe("toolEndPlanStepStatus", () => {
	test("keeps accepted plans and host advisories out of the failure count", () => {
		for (const status of [
			"scope_plan_accepted",
			"scope_plan_required",
			"declaration_lookup_required",
			"predraft_inspection_budget_exhausted",
			"discovery_budget_exhausted",
			"time_budget_unavailable",
			"validation_errors",
			"draft_needs_repair",
			"provider_specific_advisory",
		]) {
			expect(toolEndPlanStepStatus({ status })).toBe("done");
		}
	});

	test("recognises terminal failures without rejecting future status names", () => {
		for (const status of [
			"scope_plan_rejected",
			"request_identity_mismatch",
			"edit_budget_exhausted",
		]) {
			expect(toolEndPlanStepStatus({ status })).toBe("failed");
		}
		expect(toolEndPlanStepStatus({ status: "new_provider_success" })).toBe(
			"done",
		);
	});

	test("trusts explicit provider error metadata", () => {
		expect(
			toolEndPlanStepStatus({
				status: "validation_errors",
				is_error: false,
			}),
		).toBe("done");
		expect(
			toolEndPlanStepStatus({
				status: "validation_errors",
				isError: true,
			}),
		).toBe("failed");
	});

	test("settles the matching streamed tool step with the shared classifier", () => {
		const acc = createStreamAccumulator();
		applyStreamEvent(acc, {
			type: "tool_start",
			data: {
				tool_call_id: "plan-1",
				tool: "plan_board_scope",
			},
			raw: "",
		});
		applyStreamEvent(acc, {
			type: "tool_end",
			data: {
				tool_call_id: "plan-1",
				status: "scope_plan_accepted",
			},
			raw: "",
		});
		expect(acc.steps.get("plan-1")?.status).toBe("done");
	});
});

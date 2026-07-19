import { afterEach, describe, expect, it } from "vitest";

import { FlowPilotGenerationMetricsRun } from "@flow-like/flow-like-ui/components/flowpilot/generation-metrics";
import {
	type IFlowPilotProductionMetrics,
	setFlowPilotProductionMetricsSink,
} from "@flow-like/flow-like-ui/state/global-chat/agent-debug-report";

let removeSink: (() => void) | undefined;

afterEach(() => {
	removeSink?.();
	removeSink = undefined;
});

describe("direct FlowPilot generation metrics", () => {
	it("holds a queued typed review open until Apply", () => {
		const published: IFlowPilotProductionMetrics[] = [];
		removeSink = setFlowPilotProductionMetricsSink((metrics) => {
			published.push(metrics);
		});
		const run = new FlowPilotGenerationMetricsRun("direct-typed", 100);
		run.push(
			`<tool_end>${JSON.stringify({
				tool: "commit_flow_ir_draft",
				tool_call_id: "commit-1",
				result_preview: {
					status: "queued",
					draft_id: "support",
					revision: 7,
					diagnostics: [],
					missing_modules: [],
					capability_plan: { feasible: true },
				},
			})}</tool_end>`,
		);
		run.finish("ok", true);
		expect(published).toHaveLength(0);

		run.disposeReview(
			"applied",
			{
				board_id: "board",
				draft_id: "support",
				revision: 7,
				base_fingerprint: "base",
				claim_id: "claim",
				requires_destructive_approval: false,
			},
			4,
		);

		expect(published).toHaveLength(1);
		expect(published[0]).toMatchObject({
			runs_started: 1,
			runs_succeeded: 1,
			attempts_total: 1,
			attempts_reconcile_valid: 1,
			attempts_applied: 1,
			queued_reviews: 1,
			apply_dispositions: 1,
			boards_inspected: 1,
			empty_boards_after_run: 0,
		});
	});

	it("publishes a zero-candidate failure", () => {
		const published: IFlowPilotProductionMetrics[] = [];
		removeSink = setFlowPilotProductionMetricsSink((metrics) => {
			published.push(metrics);
		});
		const run = new FlowPilotGenerationMetricsRun("direct-failure", 100);
		run.finish("timeout", false, 0);

		expect(published).toHaveLength(1);
		expect(published[0]).toMatchObject({
			runs_failed: 1,
			attempts_total: 0,
			attempts_applied: 0,
			boards_inspected: 1,
			empty_boards_after_run: 1,
		});
	});
});

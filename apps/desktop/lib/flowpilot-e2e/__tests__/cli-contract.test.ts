import { describe, expect, test } from "vitest";

import {
	type FlowPilotE2EArtifact,
	type FlowPilotE2ECaseId,
	type FlowPilotE2ECliEnvelope,
	flowPilotE2ECliExitCode,
	isFlowPilotE2ECliEnvelope,
	normalizeFlowPilotE2ECliEnvelope,
} from "../index";

const runId = "e2e_contract_fixture";
const caseIds: FlowPilotE2ECaseId[] = ["forum", "simple-agent"];

function artifact(
	caseId: FlowPilotE2ECaseId,
	passed: boolean,
): FlowPilotE2EArtifact {
	return {
		schema: "flowpilot.app-creation-e2e-artifact/v1",
		generatedAt: "2026-07-22T10:00:00.000Z",
		durationMs: 10,
		requestedModel: {
			provider: "codex",
			model: "gpt-5.6-terra",
			reasoningEffort: "high",
		},
		caseId,
		expectedAppName: `Fixture ${caseId}`,
		prompt: `Create ${caseId}`,
		runner: { suppressedNavigations: [], issues: [] },
		report: {
			schema: "flowpilot.app-creation-e2e-report/v1",
			caseId,
			caseTitle: caseId,
			appId: `app-${caseId}`,
			appName: `Fixture ${caseId}`,
			expectedAppName: `Fixture ${caseId}`,
			model: {
				provider: "codex",
				model: "gpt-5.6-terra",
				reasoningEffort: "high",
			},
			passed,
			summary: { checks: 1, passed: passed ? 1 : 0, failed: passed ? 0 : 1 },
			inventory: {
				boards: 1,
				totalNodes: 1,
				pages: 1,
				widgets: 1,
				tables: 1,
				events: 1,
			},
			flowScript: { canonical: [] },
			checks: [],
			failures: [],
		},
	};
}

function envelope(
	artifacts: FlowPilotE2EArtifact[],
	overrides: Partial<FlowPilotE2ECliEnvelope> = {},
): FlowPilotE2ECliEnvelope {
	return {
		schema: "flowpilot.app-creation-e2e-cli-result/v1",
		runId,
		startedAt: "2026-07-22T10:00:00.000Z",
		completedAt: "2026-07-22T10:00:01.000Z",
		durationMs: 1_000,
		selection: { caseIds, repeat: 1, failFast: false },
		artifacts,
		passed: true,
		summary: {
			requestedRuns: 999,
			completedRuns: 999,
			passed: 999,
			failed: 0,
			skipped: 0,
		},
		...overrides,
	};
}

const expectation = {
	runId,
	caseIds,
	repeat: 1,
	minFlowScriptNonWhitespaceChars: undefined,
	failFast: false,
} as const;

describe("FlowPilot E2E CLI callback contract", () => {
	test("recomputes a webview summary and refuses a false green", () => {
		const normalized = normalizeFlowPilotE2ECliEnvelope(
			envelope([artifact("forum", true), artifact("simple-agent", false)]),
			expectation,
		);

		expect(normalized.passed).toBe(false);
		expect(normalized.summary).toEqual({
			requestedRuns: 2,
			completedRuns: 2,
			passed: 1,
			failed: 1,
			skipped: 0,
		});
		expect(flowPilotE2ECliExitCode(normalized)).toBe(1);
	});

	test("accepts an intentional fail-fast prefix only after a failed artifact", () => {
		const failFastExpectation = { ...expectation, failFast: true };
		const normalized = normalizeFlowPilotE2ECliEnvelope(
			envelope([artifact("forum", false)], {
				selection: { caseIds, repeat: 1, failFast: true },
			}),
			failFastExpectation,
		);

		expect(normalized.summary.skipped).toBe(1);
		expect(flowPilotE2ECliExitCode(normalized)).toBe(1);
		expect(() =>
			normalizeFlowPilotE2ECliEnvelope(
				envelope([artifact("forum", true)], {
					selection: { caseIds, repeat: 1, failFast: true },
				}),
				failFastExpectation,
			),
		).toThrow("before all requested runs completed");
		expect(() =>
			normalizeFlowPilotE2ECliEnvelope(
				envelope([artifact("forum", false), artifact("simple-agent", true)], {
					selection: { caseIds, repeat: 1, failFast: true },
				}),
				failFastExpectation,
			),
		).toThrow("after a fail-fast failure");
	});

	test("preserves partial evidence while classifying runner errors as infrastructure", () => {
		const normalized = normalizeFlowPilotE2ECliEnvelope(
			envelope([artifact("forum", true)], { error: "collector crashed" }),
			expectation,
		);

		expect(normalized.artifacts).toHaveLength(1);
		expect(normalized.summary.skipped).toBe(1);
		expect(flowPilotE2ECliExitCode(normalized)).toBe(2);
	});

	test("rejects stale selections, wrong order, and malformed envelopes", () => {
		expect(() =>
			normalizeFlowPilotE2ECliEnvelope(
				envelope([artifact("forum", true), artifact("simple-agent", true)], {
					selection: { caseIds: ["forum"], repeat: 1, failFast: false },
				}),
				expectation,
			),
		).toThrow("selection does not match");
		expect(() =>
			normalizeFlowPilotE2ECliEnvelope(
				envelope([artifact("simple-agent", true), artifact("forum", true)]),
				expectation,
			),
		).toThrow("expected forum");
		expect(isFlowPilotE2ECliEnvelope({ runId }, runId)).toBe(false);
	});

	test("validates repeated case ordering", () => {
		const repeatedExpectation = { ...expectation, repeat: 2 };
		const normalized = normalizeFlowPilotE2ECliEnvelope(
			envelope(
				[
					artifact("forum", true),
					artifact("simple-agent", true),
					artifact("forum", true),
					artifact("simple-agent", true),
				],
				{ selection: { caseIds, repeat: 2, failFast: false } },
			),
			repeatedExpectation,
		);

		expect(normalized.passed).toBe(true);
		expect(normalized.summary.requestedRuns).toBe(4);
		expect(flowPilotE2ECliExitCode(normalized)).toBe(0);
	});
});

import { describe, expect, test } from "vitest";

import {
	FLOWPILOT_APP_CREATION_CASES,
	FLOWPILOT_APP_CREATION_SMOKE_CASES,
	FLOWPILOT_E2E_DEFAULT_MODEL,
	buildCasePrompt,
	getFlowPilotAppCreationCase,
	resolveFlowPilotE2ERunCases,
	selectFlowPilotAppCreationCases,
} from "../index";

describe("FlowPilot app-creation E2E cases", () => {
	test("defines six stable cases and the three requested smoke archetypes", () => {
		expect(FLOWPILOT_APP_CREATION_CASES.map(({ id }) => id)).toEqual([
			"simple-agent",
			"forum",
			"ops-dashboard",
			"expense-approval",
			"rss-digest",
			"incident-console",
		]);
		expect(FLOWPILOT_APP_CREATION_SMOKE_CASES.map(({ id }) => id)).toEqual([
			"simple-agent",
			"forum",
			"ops-dashboard",
		]);
		expect(
			FLOWPILOT_APP_CREATION_CASES.every(
				(caseDefinition) =>
					caseDefinition.requirements.requireAuthoritativeReconcile,
			),
		).toBe(true);
	});

	test("defaults to Codex Terra with high reasoning", () => {
		expect(FLOWPILOT_E2E_DEFAULT_MODEL).toEqual({
			provider: "codex",
			model: "gpt-5.6-terra",
			reasoningEffort: "high",
		});
	});

	test("builds an exact unique app name and a compact FlowScript contract", () => {
		const built = buildCasePrompt(
			getFlowPilotAppCreationCase("forum"),
			"  run-42\nnightly  ",
			{ minFlowScriptNonWhitespaceChars: 321 },
		);

		expect(built.expectedAppName).toBe("Pocket Forum run-42 nightly");
		expect(built.caseDefinition.expectedAppName).toBe(built.expectedAppName);
		expect(
			built.caseDefinition.requirements.minFlowScriptNonWhitespaceChars,
		).toBe(321);
		expect(built.prompt).toContain(
			'Create the app named exactly "Pocket Forum run-42 nightly"',
		);
		expect(built.prompt).toContain(
			"Keep working FlowScript as short as practical",
		);
		expect(built.prompt).toContain("at least 321 non-whitespace characters");
		expect(built.prompt).toContain('table named exactly "Forum Threads"');
	});

	test("rejects invalid character thresholds", () => {
		expect(() =>
			buildCasePrompt(getFlowPilotAppCreationCase("forum"), "", {
				minFlowScriptNonWhitespaceChars: 0,
			}),
		).toThrow("positive safe integer");
	});

	test("selects smoke and explicit case subsets without reordering", () => {
		expect(
			selectFlowPilotAppCreationCases({
				ids: ["incident-console", "forum"],
			}).map(({ id }) => id),
		).toEqual(["forum", "incident-console"]);
		expect(
			selectFlowPilotAppCreationCases({
				smoke: true,
				ids: ["incident-console", "ops-dashboard"],
			}).map(({ id }) => id),
		).toEqual(["ops-dashboard"]);
	});

	test("resolves ordered CLI subsets and suites through one selector", () => {
		expect(
			resolveFlowPilotE2ERunCases({
				caseIds: ["incident-console", "forum", "incident-console"],
			}).map(({ id }) => id),
		).toEqual(["incident-console", "forum"]);
		expect(
			resolveFlowPilotE2ERunCases({ suite: "smoke" }).map(({ id }) => id),
		).toEqual(["simple-agent", "forum", "ops-dashboard"]);
		expect(resolveFlowPilotE2ERunCases({ suite: "full" })).toHaveLength(6);
	});

	test("rejects ambiguous runner selections", () => {
		expect(() =>
			resolveFlowPilotE2ERunCases({ caseId: "forum", suite: "smoke" }),
		).toThrow("either explicit cases or a suite");
		expect(() =>
			resolveFlowPilotE2ERunCases({
				caseId: "forum",
				caseIds: ["simple-agent"],
			}),
		).toThrow("either caseId or caseIds");
	});
});

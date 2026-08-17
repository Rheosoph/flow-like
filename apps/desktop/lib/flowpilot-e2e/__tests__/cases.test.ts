import { describe, expect, test } from "vitest";

import {
	DEFAULT_CASE_RUN_TIMEOUT_MS,
	FLOWPILOT_APP_CREATION_CASES,
	FLOWPILOT_APP_CREATION_SMOKE_CASES,
	FLOWPILOT_E2E_DEFAULT_MODEL,
	FLOWPILOT_E2E_DEFAULT_MODEL_KEY,
	FLOWPILOT_E2E_MODELS,
	FLOWPILOT_E2E_MODEL_KEYS,
	buildCasePrompt,
	flowPilotE2ECaseRunTimeoutMs,
	flowPilotE2EModel,
	getFlowPilotAppCreationCase,
	resolveFlowPilotE2EModelKey,
	resolveFlowPilotE2ERunCases,
	selectFlowPilotAppCreationCases,
} from "../index";

describe("FlowPilot app-creation E2E cases", () => {
	test("defines twelve stable cases and the three requested smoke archetypes", () => {
		expect(FLOWPILOT_APP_CREATION_CASES.map(({ id }) => id)).toEqual([
			"simple-agent",
			"forum",
			"ops-dashboard",
			"expense-approval",
			"rss-digest",
			"incident-console",
			"mail-approval",
			"doc-compliance",
			"webhook-enrichment",
			"agent-tools",
			"multi-board-pages",
			"ai-adventure",
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

	test("pins a real multi-board page-ownership regression", () => {
		const built = buildCasePrompt(
			getFlowPilotAppCreationCase("multi-board-pages"),
			"run-boards",
		);
		expect(built.prompt).toContain("board ids and globally unique page ids");
		expect(built.prompt).toContain(
			"Choose globally unique page ids and caller-selected new-board ids up front",
		);
		expect(built.prompt).toContain("create_new_board=true");
		expect(built.caseDefinition.requirements.minBoards).toBe(2);
		expect(built.caseDefinition.requirements.minPages).toBe(2);
		expect(built.caseDefinition.requirements.requiredPageBoardBindings).toEqual(
			[
				{
					page: "Operations",
					board: "Operations Workflow",
					requireOnLoadEvent: true,
				},
				{
					page: "Analytics",
					board: "Analytics Workflow",
					requireOnLoadEvent: true,
				},
			],
		);
	});

	test("defaults to Codex Terra with high reasoning", () => {
		expect(FLOWPILOT_E2E_DEFAULT_MODEL_KEY).toBe("terra");
		expect(FLOWPILOT_E2E_DEFAULT_MODEL).toEqual({
			provider: "codex",
			model: "gpt-5.6-terra",
			reasoningEffort: "high",
		});
	});

	test("offers Terra and Sol as comparable benchmark models", () => {
		expect(FLOWPILOT_E2E_MODEL_KEYS).toEqual(["terra", "sol"]);
		expect(FLOWPILOT_E2E_MODELS.sol).toEqual({
			provider: "codex",
			model: "gpt-5.6-sol",
			reasoningEffort: "high",
		});
		expect(flowPilotE2EModel("sol").model).toBe("gpt-5.6-sol");
		expect(resolveFlowPilotE2EModelKey(undefined)).toBe("terra");
		expect(resolveFlowPilotE2EModelKey("  SOL ")).toBe("sol");
		expect(resolveFlowPilotE2EModelKey("gpt-5.6-sol")).toBe("sol");
		expect(() => resolveFlowPilotE2EModelKey("gpt-4")).toThrow(
			"Unknown FlowPilot E2E model",
		);
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

	test("pins the AI adventure case to its agent-directed campaign contract", () => {
		const built = buildCasePrompt(
			getFlowPilotAppCreationCase("ai-adventure"),
			"run-7",
		);
		const requirements = built.caseDefinition.requirements;

		expect(built.expectedAppName).toBe("AI - Adventure run-7");
		expect(built.prompt).toContain(
			'pages named exactly "Adventure Menu", "Save Games", and "Play Scene"',
		);
		expect(built.prompt).toContain("Restore last savepoint");
		// A first live run burned its whole budget re-patching an over-budget root layer.
		expect(built.prompt).toContain("at most 100 nodes");
		expect(built.prompt).toContain("rejected whole");
		expect(flowPilotE2ECaseRunTimeoutMs(built.caseDefinition)).toBe(
			60 * 60_000,
		);
		expect(
			flowPilotE2ECaseRunTimeoutMs(getFlowPilotAppCreationCase("forum")),
		).toBe(DEFAULT_CASE_RUN_TIMEOUT_MS);
		expect(requirements.minPages).toBe(3);
		expect(requirements.minWidgets).toBe(2);
		expect(requirements.minTables).toBe(5);
		expect(requirements.requiredSemanticTableAliases).not.toContain(
			"adventure_memory",
		);
		expect(requirements.requiredLazyDatabaseAliases).toEqual([
			"adventure_memory",
		]);
		expect(built.prompt).toContain(
			"do not pre-create it with a guessed vector size",
		);
		expect(
			requirements.requiredNodeCapabilities.map(({ alias }) => alias),
		).toEqual(
			expect.arrayContaining([
				"agent_core",
				"agent_tools",
				"agent_invoke",
				"structured_scene",
				"memory_embedding",
				"memory_semantic_search",
			]),
		);
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
		expect(resolveFlowPilotE2ERunCases({ suite: "full" })).toHaveLength(12);
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

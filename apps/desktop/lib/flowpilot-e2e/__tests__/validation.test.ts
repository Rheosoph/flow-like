import { describe, expect, test } from "vitest";

import {
	type FlowPilotAppCreationSnapshot,
	type FlowPilotE2ECaseDefinition,
	appCreationFailureFingerprint,
	assertAppCreationCasePassed,
	evaluateAppCreationCase,
	flowScriptSizeMetrics,
	formatAppCreationReport,
	normalizeSemanticAlias,
} from "../index";

const source = `const REQUESTS = "table-expenses"
const AUDIT = "table-audit"
const PAGE = "page-queue"
const ROW = "widget-row"
const APPROVE = "action-approve"
const REJECT = "action-reject"
function loadQueue() { logInfo({ message: REQUESTS + PAGE + ROW }) }
function decide() { logInfo({ message: AUDIT + APPROVE + REJECT }) }
eventsSimple load() { loadQueue() }
eventsWidgetAction approve() { decide() }`;

function caseDefinition(
	overrides: Partial<FlowPilotE2ECaseDefinition["requirements"]> = {},
): FlowPilotE2ECaseDefinition {
	return {
		id: "expense-approval",
		title: "Fixture",
		description: "Fixture",
		appName: "Expense Desk",
		prompt: "Fixture",
		smoke: false,
		requirements: {
			minFlowScriptNonWhitespaceChars: 100,
			maxFlowScriptNonWhitespaceChars: 2_000,
			minBoards: 1,
			minTotalNodes: 3,
			minPages: 1,
			minWidgets: 1,
			minTables: 2,
			minEvents: 2,
			requireAuthoredFlowScript: true,
			requireAuthoredLintDiagnostics: true,
			requireCanonicalFlowScript: true,
			requireLintDiagnostics: true,
			requireAuthoritativeReconcile: true,
			requireSuccessfulCompilerReceipt: true,
			validateReferenceIntegrity: true,
			requiredSemanticTableAliases: ["expense_requests", "expense_audit"],
			requiredLazyDatabaseAliases: [],
			requiredIdReferences: [
				{
					entity: "table",
					alias: "expense_requests",
					source: "both",
				},
				{ entity: "widget", alias: "expense_row", source: "canonical" },
				{ entity: "widget_action", alias: "approve", source: "canonical" },
				{ entity: "page", alias: "expense_queue", source: "canonical" },
			],
			requiredNodeCapabilities: [
				{
					alias: "database_write",
					anyOf: ["insert_local_db", "upsert_local_db"],
				},
			],
			...overrides,
		},
	};
}

function snapshot(): FlowPilotAppCreationSnapshot {
	return {
		appId: "app-expenses",
		appName: "Expense Desk",
		model: {
			provider: "codex",
			model: "gpt-5.6-terra",
			reasoningEffort: "high",
		},
		authoredFlowScript: source,
		authoredFlowScriptStatus: "queued",
		authoredLintDiagnostics: [],
		flowScriptGenerationRuns: [
			{
				schema: "flowpilot.flowscript-generation-run/v1",
				conversationId: "conversation-fixture",
				requestId: "request-fixture:agent",
				parentRequestId: "request-fixture",
				appId: "app-expenses",
				boardId: "board-main",
				provider: "codex",
				modelId: "codex:gpt-5.6-terra",
				reasoningEffort: "high",
				startedAtMs: 1,
				endedAtMs: 2,
				outcome: "ok",
				finalWorkspaceStatus: "queued",
				appliedCommands: 4,
				persistedReadbackVerified: true,
				candidates: [{ source, status: "queued", capturedAtMs: 1 }],
				compilerReceipts: [
					{
						toolName: "check_flowscript",
						status: "valid",
						draftId: "draft-fixture",
						revision: 2,
						baseFingerprint: "base-fixture",
						source,
						diagnostics: [],
						reviewNotes: [],
						corrections: [],
						derivedCommandCount: 4,
						queuedCount: 0,
						success: true,
						payload: {
							status: "valid",
							draft_id: "draft-fixture",
							revision: 2,
							base_fingerprint: "base-fixture",
							derived_command_count: 4,
							queued_count: 0,
						},
						capturedAtMs: 1,
					},
					{
						toolName: "commit_flowscript",
						status: "queued",
						draftId: "draft-fixture",
						revision: 2,
						baseFingerprint: "base-fixture",
						source,
						diagnostics: [],
						reviewNotes: [],
						corrections: [],
						derivedCommandCount: 4,
						queuedCount: 4,
						success: true,
						payload: {
							status: "queued",
							draft_id: "draft-fixture",
							revision: 2,
							base_fingerprint: "base-fixture",
							derived_command_count: 4,
							queued_count: 4,
						},
						capturedAtMs: 2,
					},
				],
			},
		],
		boards: [
			{
				id: "board-main",
				name: "Expense Workflow",
				nodeCount: 4,
				nodeIds: ["node-load", "node-approve", "node-reject", "node-helper"],
				nodeTypes: ["events_simple", "insert_local_db", "log_info"],
				flowScript: source,
				lintDiagnostics: [],
				reconcile: {
					parseValid: true,
					reconcileValid: true,
					idempotent: true,
					commandCount: 0,
					corrections: [],
					diagnostics: [],
				},
			},
		],
		pages: [
			{
				id: "page-queue",
				name: "Expense Queue",
				boardId: "board-main",
				onLoadEventId: "node-load",
				content: [
					{ Widget: { widgetId: "widget-row", instanceId: "row-1" } },
					{ Component: { targetPageId: "page-queue" } },
				],
				widgetRefs: {
					"row-1": { id: "widget-row", name: "Expense Row" },
				},
			},
		],
		widgets: [
			{
				id: "widget-row",
				name: "Expense Row",
				actions: [
					{ id: "action-approve", label: "Approve" },
					{ id: "action-reject", label: "Reject" },
				],
			},
		],
		tables: [
			{
				id: "table-expenses",
				name: "Expense Requests",
				semanticAlias: "expense_requests",
			},
			{
				id: "table-audit",
				name: "Expense Audit",
				semanticAlias: "expense_audit",
			},
		],
		events: [
			{
				id: "event-load",
				name: "Load",
				boardId: "board-main",
				nodeId: "node-load",
			},
			{
				id: "event-approve",
				name: "Approve",
				boardId: "board-main",
				nodeId: "node-approve",
			},
		],
	};
}

describe("FlowPilot app-creation artifact validation", () => {
	test("accepts a complete deterministic artifact snapshot", () => {
		const definition = caseDefinition();
		const artifacts = snapshot();
		const report = evaluateAppCreationCase(definition, artifacts);

		expect(report.passed).toBe(true);
		expect(report.summary.failed).toBe(0);
		expect(report.inventory).toEqual({
			boards: 1,
			totalNodes: 4,
			pages: 1,
			widgets: 1,
			tables: 2,
			events: 2,
		});
		expect(report.model).toEqual({
			provider: "codex",
			model: "gpt-5.6-terra",
			reasoningEffort: "high",
		});
		expect(evaluateAppCreationCase(definition, artifacts)).toEqual(report);
		expect(() => assertAppCreationCasePassed(report)).not.toThrow();
	});

	test("uses exact names and honors a captured model config", () => {
		const artifacts = snapshot();
		artifacts.appName = "Expense desk";
		artifacts.model = {
			provider: "codex",
			model: "gpt-5.6-terra",
			reasoningEffort: "medium",
		};
		const report = evaluateAppCreationCase(caseDefinition(), artifacts);

		expect(report.passed).toBe(false);
		expect(report.model.reasoningEffort).toBe("medium");
		expect(report.failures.map(({ code }) => code)).toContain("app.name");
	});

	test("evaluates a run against the benchmark model it requested", () => {
		const artifacts = snapshot();
		const sol = {
			provider: "codex",
			model: "gpt-5.6-sol",
			reasoningEffort: "high",
		} as const;

		const wrongModel = evaluateAppCreationCase(
			caseDefinition(),
			artifacts,
			sol,
		);
		expect(wrongModel.failures.map(({ code }) => code)).toContain("model.id");
		expect(wrongModel.failures.map(({ code }) => code)).toContain(
			"flowscript.compiler_receipt.nested_model",
		);

		const solRun = snapshot();
		solRun.model = { ...sol };
		const run = solRun.flowScriptGenerationRuns?.[0];
		if (!run) throw new Error("expected fixture compiler run");
		solRun.flowScriptGenerationRuns = [
			{ ...run, modelId: "codex:gpt-5.6-sol" },
		];
		expect(evaluateAppCreationCase(caseDefinition(), solRun, sol).passed).toBe(
			true,
		);
	});

	test("requires a successful exact-revision compiler receipt", () => {
		const missing = snapshot();
		missing.flowScriptGenerationRuns = [];
		const missingCodes = evaluateAppCreationCase(
			caseDefinition(),
			missing,
		).failures.map(({ code }) => code);
		expect(missingCodes).toContain("flowscript.compiler_receipt.present");
		expect(missingCodes).toContain("flowscript.compiler_receipt.check_success");
		expect(missingCodes).toContain(
			"flowscript.compiler_receipt.commit_success",
		);

		const mismatched = snapshot();
		const run = mismatched.flowScriptGenerationRuns?.[0];
		if (!run) throw new Error("expected fixture compiler run");
		mismatched.flowScriptGenerationRuns = [
			{
				...run,
				compilerReceipts: run.compilerReceipts.map((receipt) =>
					receipt.toolName === "commit_flowscript"
						? { ...receipt, revision: 3 }
						: receipt,
				),
			},
		];
		const mismatchCodes = evaluateAppCreationCase(
			caseDefinition(),
			mismatched,
		).failures.map(({ code }) => code);
		expect(mismatchCodes).toContain(
			"flowscript.compiler_receipt.exact_revision",
		);

		const unbound = snapshot();
		const unboundRun = unbound.flowScriptGenerationRuns?.[0];
		if (!unboundRun) throw new Error("expected fixture compiler run");
		unbound.flowScriptGenerationRuns = [
			{
				...unboundRun,
				compilerReceipts: unboundRun.compilerReceipts.map((receipt) => ({
					...receipt,
					draftId: undefined,
				})),
			},
		];
		const unboundCodes = evaluateAppCreationCase(
			caseDefinition(),
			unbound,
		).failures.map(({ code }) => code);
		expect(unboundCodes).toContain(
			"flowscript.compiler_receipt.exact_revision",
		);
	});

	test("fails a missing model and an invalid authored candidate", () => {
		const artifacts = snapshot();
		artifacts.model = undefined;
		artifacts.authoredFlowScriptStatus = "validation_errors";
		artifacts.authoredFlowScriptCompletion = "partial_working_slice";
		artifacts.authoredLintDiagnostics = [
			{ severity: "error", message: "authored source does not parse" },
		];
		const report = evaluateAppCreationCase(caseDefinition(), artifacts);
		const codes = report.failures.map(({ code }) => code);

		expect(codes).toContain("model.present");
		expect(codes).toContain("model.provider");
		expect(codes).toContain("model.id");
		expect(codes).toContain("model.reasoning_effort");
		expect(codes).toContain("flowscript.authored.status");
		expect(codes).toContain("flowscript.authored.completion");
		expect(codes).toContain("flowscript.authored.lint_errors");
	});

	test("reports an authored source truncated by the stream capture cap", () => {
		const artifacts = snapshot();
		artifacts.authoredFlowScript = `${source}${"x".repeat(16_384 - source.length)}...`;
		const report = evaluateAppCreationCase(caseDefinition(), artifacts);

		expect(report.failures.map(({ code }) => code)).toContain(
			"flowscript.authored.capture_complete",
		);
		expect(
			evaluateAppCreationCase(caseDefinition(), snapshot()).failures.map(
				({ code }) => code,
			),
		).not.toContain("flowscript.authored.capture_complete");
	});

	test("rejects Markdown wrappers and prose padding in authored FlowScript", () => {
		const artifacts = snapshot();
		artifacts.authoredFlowScript = `${source}\n// padding\n\`\`\`flowscript`;
		const report = evaluateAppCreationCase(caseDefinition(), artifacts);

		expect(report.failures.map(({ code }) => code)).toContain(
			"flowscript.authored.compact_source",
		);
	});

	test("reports short, invalid and non-idempotent canonical FlowScript", () => {
		const artifacts = snapshot();
		artifacts.authoredFlowScript = "eventsSimple x(){}";
		artifacts.boards = [
			{
				...artifacts.boards[0],
				flowScript: "eventsSimple x(){}",
				lintDiagnostics: [
					{ severity: "error", message: "unknown declaration" },
				],
				reconcile: {
					parseValid: false,
					reconcileValid: false,
					idempotent: false,
					commandCount: 2,
					diagnostics: ["cannot reconcile"],
				},
			},
		];
		const report = evaluateAppCreationCase(caseDefinition(), artifacts);
		const codes = report.failures.map(({ code }) => code);

		expect(codes).toContain("flowscript.authored.min_non_whitespace");
		expect(codes).toContain("flowscript.canonical.min_non_whitespace");
		expect(codes).toContain("flowscript.lint.errors.board-main");
		expect(codes).toContain("flowscript.reconcile.parse_valid.board-main");
		expect(codes).toContain("flowscript.reconcile.reconcile_valid.board-main");
		expect(codes).toContain("flowscript.reconcile.idempotent.board-main");
		expect(codes).toContain("flowscript.reconcile.commands.board-main");
	});

	test("detects unresolved aliases and missing concrete FlowScript ids", () => {
		const artifacts = snapshot();
		artifacts.tables = [artifacts.tables[0]];
		artifacts.boards = [
			{
				...artifacts.boards[0],
				flowScript: source.replace("widget-row", "not-the-widget"),
			},
		];
		const report = evaluateAppCreationCase(caseDefinition(), artifacts);
		const codes = report.failures.map(({ code }) => code);

		expect(codes).toContain("tables.count");
		expect(codes).toContain("tables.semantic_alias.expense_audit");
		expect(codes).toContain("flowscript.id_reference.widget.expense_row");
	});

	test("requires an exact canonical database call for lazy aliases", () => {
		const definition = caseDefinition({
			requiredLazyDatabaseAliases: ["adventure_memory"],
		});
		const exact = snapshot();
		exact.boards = [
			{
				...exact.boards[0],
				flowScript: `${source}
function memory() { return database({ name: "adventure_memory" }) }`,
			},
		];
		expect(
			evaluateAppCreationCase(definition, exact).failures.map(
				({ code }) => code,
			),
		).not.toContain("flowscript.lazy_database_alias.adventure_memory");

		const unrelatedLiteral = snapshot();
		unrelatedLiteral.boards = [
			{
				...unrelatedLiteral.boards[0],
				flowScript: `${source}
const memoryName = "adventure_memory"
function memory() { return database({ name: "prefix-adventure_memory" }) }`,
			},
		];
		expect(
			evaluateAppCreationCase(definition, unrelatedLiteral).failures.map(
				({ code }) => code,
			),
		).toContain("flowscript.lazy_database_alias.adventure_memory");
	});

	test("requires scenario capabilities in the persisted workflow graph", () => {
		const artifacts = snapshot();
		artifacts.boards = [
			{ ...artifacts.boards[0], nodeTypes: ["events_simple", "log_info"] },
		];
		const report = evaluateAppCreationCase(caseDefinition(), artifacts);

		expect(report.failures.map(({ code }) => code)).toContain(
			"flowscript.capability.database_write",
		);
	});

	test("does not count an id embedded in an identifier as a wired value", () => {
		const artifacts = snapshot();
		artifacts.authoredFlowScript = source.replace(
			'"table-expenses"',
			'"prefix-table-expenses-suffix"',
		);
		artifacts.boards = [
			{
				...artifacts.boards[0],
				flowScript: source.replace(
					'"table-expenses"',
					'"prefix-table-expenses-suffix"',
				),
			},
		];
		const report = evaluateAppCreationCase(caseDefinition(), artifacts);

		expect(report.failures.map(({ code }) => code)).toContain(
			"flowscript.id_reference.table.expense_requests",
		);
	});

	test("accepts a page id in an A2UI page/element target", () => {
		const artifacts = snapshot();
		const compoundTarget = source.replace('"page-queue"', '"page-queue/kpi"');
		artifacts.boards = [{ ...artifacts.boards[0], flowScript: compoundTarget }];
		const report = evaluateAppCreationCase(caseDefinition(), artifacts);

		expect(report.failures.map(({ code }) => code)).not.toContain(
			"flowscript.id_reference.page.expense_queue",
		);
	});

	test("rejects an authored script above the compactness ceiling", () => {
		const artifacts = snapshot();
		artifacts.authoredFlowScript = `${source}\n${"x".repeat(2_001)}`;
		const report = evaluateAppCreationCase(caseDefinition(), artifacts);

		expect(report.failures.map(({ code }) => code)).toContain(
			"flowscript.authored.max_non_whitespace",
		);
	});

	test("checks page/widget, page/node and app-event/node reference integrity", () => {
		const artifacts = snapshot();
		artifacts.pages = [
			{
				...artifacts.pages[0],
				onLoadEventId: "event-load",
				content: {
					widgetId: "missing-widget",
					targetPageId: "missing-page",
				},
				widgetRefs: {},
			},
		];
		artifacts.events = [
			{
				...artifacts.events[0],
				nodeId: "missing-node",
			},
			...artifacts.events.slice(1),
		];
		const report = evaluateAppCreationCase(caseDefinition(), artifacts);
		const codes = report.failures.map(({ code }) => code);

		expect(codes).toContain("integrity.page_load_node.page-queue");
		expect(codes).toContain("integrity.page_widget.page-queue.missing-widget");
		expect(codes).toContain("integrity.page_reference.page-queue.missing-page");
		expect(codes).toContain("integrity.event_node.event-load");
	});

	test("requires authoritative results only when the case requests them", () => {
		const artifacts = snapshot();
		artifacts.boards = [{ ...artifacts.boards[0], reconcile: undefined }];

		const required = evaluateAppCreationCase(caseDefinition(), artifacts);
		expect(required.failures.map(({ code }) => code)).toContain(
			"flowscript.reconcile.available.board-main",
		);

		const optional = evaluateAppCreationCase(
			caseDefinition({ requireAuthoritativeReconcile: false }),
			artifacts,
		);
		expect(optional.failures.map(({ code }) => code)).not.toContain(
			"flowscript.reconcile.available.board-main",
		);
	});

	test("requires canonical source, lint and reconcile for every board", () => {
		const artifacts = snapshot();
		artifacts.boards = [
			...artifacts.boards,
			{
				id: "board-empty",
				name: "Unexpected scaffold",
				nodeCount: 0,
				nodeIds: [],
			},
		];
		const report = evaluateAppCreationCase(caseDefinition(), artifacts);
		const codes = report.failures.map(({ code }) => code);

		expect(codes).toContain("flowscript.canonical.board_present.board-empty");
		expect(codes).toContain("flowscript.lint.available.board-empty");
		expect(codes).toContain("flowscript.reconcile.available.board-empty");
		expect(codes).toContain("boards.nonempty.board-empty");
	});

	test("formats compact reports and stable failure fingerprints", () => {
		const artifacts = snapshot();
		artifacts.appName = "Wrong";
		const report = evaluateAppCreationCase(caseDefinition(), artifacts);

		expect(appCreationFailureFingerprint(report)).toBe("app.name");
		expect(formatAppCreationReport(report)).toContain(
			"FAIL expense-approval: Wrong",
		);
		expect(() => assertAppCreationCasePassed(report)).toThrow(
			"FAIL [app.name]",
		);

		const boardFailure = {
			...report,
			failures: [
				{ ...report.failures[0], code: "flowscript.lint.errors.random-a" },
				{ ...report.failures[0], code: "flowscript.lint.errors.random-b" },
			],
		};
		expect(appCreationFailureFingerprint(boardFailure)).toBe(
			"flowscript.lint.errors",
		);
	});

	test("normalizes aliases and reports deterministic size metrics", () => {
		expect(normalizeSemanticAlias("  Métric & Snapshots ")).toBe(
			"metric_and_snapshots",
		);
		expect(flowScriptSizeMetrics("a b\ncd")).toEqual({
			characters: 6,
			nonWhitespaceCharacters: 4,
			lines: 2,
			estimatedTokens: 2,
		});
	});
});

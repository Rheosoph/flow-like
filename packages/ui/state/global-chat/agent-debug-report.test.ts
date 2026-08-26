import { describe, expect, test } from "bun:test";
import {
	agentDebugPreview,
	agentDebugReportAsMarkdown,
	agentDebugRunSummaries,
	createAgentDebugReport,
	debugEventFromCopilotStream,
	finalizeAgentDebugReport,
	markAgentDebugReportInterrupted,
	recordAgentDebugEvent,
	runSummaryDiagnosticTrends,
} from "./agent-debug-report";
import type { IMessage } from "./global-chat-db";
import {
	markRestoredMessageDebugReportStale,
	useGlobalChatStore,
} from "./global-chat-store";

describe("interaction diagnostic privacy", () => {
	test("redacts values entered into app pages while retaining the target", () => {
		const preview = agentDebugPreview({
			app_id: "checkout",
			actions: [
				{
					action: "set_value",
					component_id: "password",
					value: "correct horse battery staple",
				},
				{
					action: "trigger",
					component_id: "submit",
					event: "click",
				},
			],
		});

		expect(preview).toContain('"component_id":"password"');
		expect(preview).toContain('"value":"[REDACTED]"');
		expect(preview).not.toContain("correct horse battery staple");
	});
});

function workspaceArtifact(
	source: string,
	status: string,
	nowMs: number,
	sequence: number,
) {
	const event = debugEventFromCopilotStream(
		{ type: "flowscript_workspace", data: { status, source } },
		{
			scope: "nested",
			requestId: "board-run",
			parentRequestId: "parent-run",
			nowMs,
			sequence,
		},
	);
	if (!event) throw new Error("Expected a flowscript_workspace artifact.");
	return event;
}

describe("flowscript artifact source dedup", () => {
	test("keeps the first copy of a source and stores repeats as a reference line", () => {
		const source = `function buildSupportBoard() {\n${"x".repeat(20 * 1024)}\n}`;
		let report = createAgentDebugReport("dedupe-run", { startedAtMs: 1_000 });
		report = recordAgentDebugEvent(
			report,
			workspaceArtifact(source, "validation_errors", 1_100, 1),
		);
		report = recordAgentDebugEvent(
			report,
			workspaceArtifact(source, "queued", 1_200, 2),
		);

		const artifacts = report.events.filter(
			(event) => event.stage === "artifact",
		);
		expect(artifacts).toHaveLength(2);
		const [first, second] = artifacts;
		expect(first?.result_preview?.length ?? 0).toBeGreaterThan(20 * 1024);
		expect(first?.result_preview).toContain("validation_errors");
		expect(second?.result_preview?.length ?? 0).toBeLessThan(1_024);
		expect(second?.result_preview).toContain("FlowScript source unchanged");
		expect(second?.result_preview).toContain("hash");
		expect(second?.result_preview).toContain("queued");
	});

	test("a changed source is embedded in full again", () => {
		const source = `function first() {\n${"a".repeat(4 * 1024)}\n}`;
		const changed = `function second() {\n${"b".repeat(4 * 1024)}\n}`;
		let report = createAgentDebugReport("dedupe-changed", {
			startedAtMs: 1_000,
		});
		report = recordAgentDebugEvent(
			report,
			workspaceArtifact(source, "validation_errors", 1_100, 1),
		);
		report = recordAgentDebugEvent(
			report,
			workspaceArtifact(changed, "queued", 1_200, 2),
		);

		const artifacts = report.events.filter(
			(event) => event.stage === "artifact",
		);
		expect(artifacts[1]?.result_preview).toContain("function second");
		expect(artifacts[1]?.result_preview).not.toContain(
			"FlowScript source unchanged",
		);
	});
});

describe("compaction retention order", () => {
	test("terminal tool_end diagnostics outlive artifact snapshots under pressure", () => {
		let report = createAgentDebugReport("pressure-run", { startedAtMs: 1_000 });
		for (let index = 0; index < 30; index += 1) {
			report = recordAgentDebugEvent(
				report,
				workspaceArtifact(
					`function candidate${index}() {\n${"f".repeat(24 * 1024)}\n}`,
					"validation_errors",
					1_100 + index,
					index + 1,
				),
			);
		}
		for (let index = 0; index < 220; index += 1) {
			report = recordAgentDebugEvent(report, {
				id: `terminal-${index}`,
				kind: "tool",
				stage: "tool_end",
				status: "error",
				name: "check_flowscript",
				timestamp_ms: 2_000 + index,
				ended_at_ms: 2_000 + index,
				error: `FS_TYPE_MISMATCH terminal diagnostic ${index}`,
				arguments_preview: "p".repeat(8 * 1024),
				result_preview: "q".repeat(8 * 1024),
				reasoning: "r".repeat(8 * 1024),
			});
		}

		const bytes = new TextEncoder().encode(JSON.stringify(report)).byteLength;
		expect(bytes).toBeLessThanOrEqual(512 * 1024);
		expect(report.truncation?.bytes_dropped ?? 0).toBeGreaterThan(0);
		const artifacts = report.events.filter(
			(event) => event.stage === "artifact",
		);
		const terminals = report.events.filter(
			(event) => event.stage === "tool_end",
		);
		// Artifact snapshots are shed before terminal evidence; the diagnostics that explain the
		// failed run must survive the longest.
		expect(terminals.length).toBeGreaterThan(0);
		expect(artifacts.length).toBeLessThan(terminals.length);
		expect(
			terminals.every((event) =>
				event.error?.includes("FS_TYPE_MISMATCH terminal diagnostic"),
			),
		).toBe(true);
	});
});

function runSummaryStreamEvent(
	overrides: Record<string, unknown>,
	nowMs: number,
	sequence: number,
) {
	const event = debugEventFromCopilotStream(
		{
			type: "tool_end",
			data: {
				kind: "run_summary",
				outcome: "committed",
				provider: "codex",
				model: "gpt-5",
				duration_ms: 1234,
				phases: 2,
				budget: {
					checks: { used: 5, limit: 12 },
					source_ops: { used: 9, limit: 24 },
					commits: { used: 2, limit: 3 },
					stalled: { used: 1, limit: 3 },
					continuations: { used: 1, limit: 2 },
				},
				diagnostics_by_code: { FS_TYPE_MISMATCH: 12 },
				retained_draft: { id: "draft-1", revision: 7 },
				review_notes: 3,
				applied_commands: 6,
				...overrides,
			},
		},
		{
			scope: "nested",
			requestId: "board-run",
			parentRequestId: "parent-run",
			nowMs,
			sequence,
		},
	);
	if (!event) throw new Error("Expected a run_summary debug event.");
	return event;
}

describe("run summaries", () => {
	test("a run summary survives event-count and byte compaction pressure", () => {
		let report = createAgentDebugReport("summary-pressure", {
			startedAtMs: 1_000,
		});
		report = recordAgentDebugEvent(report, runSummaryStreamEvent({}, 1_050, 1));
		for (let index = 0; index < 30; index += 1) {
			report = recordAgentDebugEvent(
				report,
				workspaceArtifact(
					`function candidate${index}() {\n${"f".repeat(24 * 1024)}\n}`,
					"validation_errors",
					1_100 + index,
					index + 2,
				),
			);
		}
		for (let index = 0; index < 260; index += 1) {
			report = recordAgentDebugEvent(report, {
				id: `terminal-${index}`,
				kind: "tool",
				stage: "tool_end",
				status: "error",
				name: "check_flowscript",
				timestamp_ms: 2_000 + index,
				ended_at_ms: 2_000 + index,
				error: `FS_TYPE_MISMATCH terminal diagnostic ${index}`,
				arguments_preview: "p".repeat(8 * 1024),
				result_preview: "q".repeat(8 * 1024),
				reasoning: "r".repeat(8 * 1024),
			});
		}

		const bytes = new TextEncoder().encode(JSON.stringify(report)).byteLength;
		expect(bytes).toBeLessThanOrEqual(512 * 1024);
		expect(report.truncation?.events_dropped ?? 0).toBeGreaterThan(0);
		const summaries = agentDebugRunSummaries(report);
		expect(summaries).toHaveLength(1);
		expect(summaries[0]?.outcome).toBe("committed");
		expect(summaries[0]?.diagnostics_by_code?.FS_TYPE_MISMATCH).toBe(12);
		expect(summaries[0]?.budget?.checks).toEqual({ used: 5, limit: 12 });
		expect(summaries[0]?.retained_draft).toEqual({
			id: "draft-1",
			revision: 7,
		});
	});

	test("run summaries render as a compact table at the top of the markdown export", () => {
		let report = createAgentDebugReport("summary-markdown", {
			startedAtMs: 1_000,
		});
		report = recordAgentDebugEvent(report, runSummaryStreamEvent({}, 1_500, 1));
		report = finalizeAgentDebugReport(report, {
			outcome: "ok",
			terminalStage: "completed",
			endedAtMs: 3_000,
		});

		const markdown = agentDebugReportAsMarkdown(report);
		expect(markdown).toContain("## Run summaries");
		expect(markdown).toContain(
			"| 1 | **committed** | codex / gpt-5 | 1234 ms | 2 | 5/12 | 9/24 | 2/3 | 1/3 | 1/2 | 12 | `draft-1@7` | 3 | 6 |",
		);
		expect(markdown.indexOf("## Run summaries")).toBeLessThan(
			markdown.indexOf("## Timeline"),
		);
		// A single run renders no trend section.
		expect(markdown).not.toContain("### Diagnostic trend");
	});

	test("multiple run summaries aggregate a per-code diagnostic trend", () => {
		let report = createAgentDebugReport("summary-trend", {
			startedAtMs: 1_000,
		});
		report = recordAgentDebugEvent(
			report,
			runSummaryStreamEvent(
				{
					outcome: "incomplete",
					diagnostics_by_code: { FS_TYPE_MISMATCH: 12 },
				},
				1_100,
				1,
			),
		);
		report = recordAgentDebugEvent(
			report,
			runSummaryStreamEvent(
				{
					outcome: "incomplete",
					diagnostics_by_code: {
						FS_TYPE_MISMATCH: 3,
						FS_UNKNOWN_DECLARATION: 1,
					},
				},
				1_200,
				2,
			),
		);
		report = recordAgentDebugEvent(
			report,
			runSummaryStreamEvent({ diagnostics_by_code: {} }, 1_300, 3),
		);

		const summaries = agentDebugRunSummaries(report);
		expect(summaries).toHaveLength(3);
		expect(runSummaryDiagnosticTrends(summaries)).toEqual([
			"FS_TYPE_MISMATCH: 12 -> 3 -> 0 across runs",
			"FS_UNKNOWN_DECLARATION: 0 -> 1 -> 0 across runs",
		]);
		const markdown = agentDebugReportAsMarkdown(report);
		expect(markdown).toContain("### Diagnostic trend");
		expect(markdown).toContain("FS_TYPE_MISMATCH: 12 -> 3 -> 0 across runs");
	});
});

describe("stale run reports", () => {
	test("marks a running report interrupted at its last event timestamp", () => {
		let report = createAgentDebugReport("stale-run", { startedAtMs: 1_000 });
		report = recordAgentDebugEvent(report, {
			id: "tool-1",
			kind: "tool",
			stage: "tool_end",
			status: "done",
			timestamp_ms: 5_000,
			ended_at_ms: 5_000,
		});

		const marked = markAgentDebugReportInterrupted(report);
		expect(marked.outcome).toBe("interrupted");
		expect(marked.ended_at_ms).toBe(5_000);
		expect(marked.duration_ms).toBe(4_000);
		expect(marked.terminal_code).toBe("RUN_INTERRUPTED");
		expect(marked.generation_evaluation).toBeUndefined();
		expect(agentDebugReportAsMarkdown(marked)).toContain("**interrupted**");
	});

	test("finalized reports pass through unchanged", () => {
		const report = finalizeAgentDebugReport(
			createAgentDebugReport("done-run", { startedAtMs: 1_000 }),
			{ outcome: "ok", terminalStage: "completed", endedAtMs: 2_000 },
		);
		expect(markAgentDebugReportInterrupted(report)).toBe(report);
	});

	test("loadConversation marks restored running reports as interrupted", () => {
		let report = createAgentDebugReport("dead-run", { startedAtMs: 1_000 });
		report = recordAgentDebugEvent(report, {
			id: "tool-1",
			kind: "tool",
			stage: "tool_end",
			status: "done",
			timestamp_ms: 3_000,
			ended_at_ms: 3_000,
		});
		const message: IMessage = {
			id: "dead-run",
			appId: "global-chat",
			sessionId: "conversation-1",
			inner: {
				role: "assistant",
				content: "partial reply",
			} as IMessage["inner"],
			files: [],
			tools: [],
			actions: [],
			timestamp: 1_000,
			debug_report: report,
		};

		useGlobalChatStore.getState().loadConversation("conversation-1", [message]);
		const restored = useGlobalChatStore.getState().messages[0];
		expect(restored?.debug_report?.outcome).toBe("interrupted");
		expect(restored?.debug_report?.ended_at_ms).toBe(3_000);

		const finalizedMessage: IMessage = {
			...message,
			id: "finished-run",
			debug_report: finalizeAgentDebugReport(
				createAgentDebugReport("finished-run", { startedAtMs: 1_000 }),
				{ outcome: "ok", terminalStage: "completed", endedAtMs: 2_000 },
			),
		};
		expect(
			markRestoredMessageDebugReportStale(finalizedMessage).debug_report
				?.outcome,
		).toBe("ok");
		expect(
			markRestoredMessageDebugReportStale({
				...message,
				debug_report: undefined,
			}),
		).toMatchObject({ id: "dead-run" });
	});
});

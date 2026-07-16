import { describe, expect, test } from "bun:test";
import {
	agentDebugReportAsMarkdown,
	createAgentDebugReport,
	debugEventFromCopilotStream,
	finalizeAgentDebugReport,
	markAgentDebugReportInterrupted,
	recordAgentDebugEvent,
} from "./agent-debug-report";
import type { IMessage } from "./global-chat-db";
import {
	markRestoredMessageDebugReportStale,
	useGlobalChatStore,
} from "./global-chat-store";

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

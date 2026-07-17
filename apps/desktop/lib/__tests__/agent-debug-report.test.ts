import {
	AGENT_DEBUG_REPORT_SCHEMA,
	agentDebugPreview,
	agentDebugReportAsMarkdown,
	agentGenerationReviewDispositionEvent,
	beginAgentGenerationMetrics,
	createAgentDebugReport,
	createAgentDebugStreamRecorder,
	debugEventFromCopilotStream,
	finalizeAgentDebugReport,
	finalizeAgentGenerationMetrics,
	nestedAgentRunEvent,
	recordAgentDebugEvent,
	recordAgentGenerationMetricEvent,
	setFlowPilotProductionMetricsSink,
	summarizeAgentDebugRootOutcomes,
} from "@flow-like/flow-like-ui/state/global-chat/agent-debug-report";
import { useGlobalChatStore } from "@flow-like/flow-like-ui/state/global-chat/global-chat-store";
import { describe, expect, test, vi } from "vitest";

describe("agent debug report", () => {
	test("renders merged terminal events in stable chronological order", () => {
		const report = createAgentDebugReport("run-order", { startedAtMs: 900 });
		report.events = [
			{
				id: "run",
				kind: "lifecycle",
				stage: "run_started",
				timestamp_ms: 1_000,
			},
			{
				id: "long-tool",
				kind: "tool",
				stage: "tool_end",
				name: "flowpilot_board",
				timestamp_ms: 3_000,
				started_at_ms: 1_100,
				ended_at_ms: 3_000,
			},
			{
				id: "short-tool",
				kind: "tool",
				stage: "tool_end",
				name: "list_apps",
				timestamp_ms: 2_000,
				started_at_ms: 1_500,
				ended_at_ms: 2_000,
			},
		];

		const markdown = agentDebugReportAsMarkdown(report);
		expect(markdown.indexOf("run_started")).toBeLessThan(
			markdown.indexOf("list_apps"),
		);
		expect(markdown.indexOf("list_apps")).toBeLessThan(
			markdown.indexOf("flowpilot_board"),
		);
	});

	test("unwraps provider PlanStep envelopes", () => {
		const event = debugEventFromCopilotStream(
			{
				type: "plan_step",
				data: {
					PlanStep: {
						id: "plan-1",
						title: "Inspect board",
						description: "Read the existing nodes and connections.",
						status: "InProgress",
						reasoning: "The board must exist before an Event is attached.",
					},
				},
			},
			{
				scope: "main",
				requestId: "run-1",
				nowMs: 1_000,
			},
		);

		expect(event).toMatchObject({
			id: "main:run-1:plan:plan-1",
			kind: "plan",
			stage: "plan",
			status: "InProgress",
			name: "Inspect board",
			summary: "Read the existing nodes and connections.",
			reasoning: "The board must exist before an Event is attached.",
			request_id: "run-1",
			timestamp_ms: 1_000,
		});
	});

	test("keeps start metadata when progress and terminal tool frames are merged", () => {
		let report = createAgentDebugReport("run-1", { startedAtMs: 900 });
		const start = debugEventFromCopilotStream(
			{
				type: "tool_start",
				data: {
					tool_call_id: "tool-1",
					tool_name: "flowpilot_board",
					summary: "Build the board",
					arguments: JSON.stringify({ app_id: "app-1", password: "hidden" }),
				},
			},
			{ scope: "main", requestId: "run-1", nowMs: 1_000 },
		);
		expect(start).not.toBeNull();
		if (!start) throw new Error("Expected a tool_start debug event.");
		report = recordAgentDebugEvent(report, start);

		const progress = debugEventFromCopilotStream(
			{
				type: "tool_progress",
				data: {
					tool_call_id: "tool-1",
					message: "Nested agent is editing the FlowScript.",
				},
			},
			{ scope: "main", requestId: "run-1", nowMs: 1_100 },
		);
		if (!progress) throw new Error("Expected a tool_progress debug event.");
		report = recordAgentDebugEvent(report, progress);

		const end = debugEventFromCopilotStream(
			{
				type: "tool_end",
				data: {
					tool_call_id: "tool-1",
					status: "timeout",
					result_summary: "Timed out after 600 seconds",
					result_preview: JSON.stringify({ status: "timeout" }),
					error: "Frontend response deadline exceeded",
				},
			},
			{ scope: "main", requestId: "run-1", nowMs: 1_250 },
		);
		if (!end) throw new Error("Expected a tool_end debug event.");
		report = recordAgentDebugEvent(report, end);

		expect(report.events).toHaveLength(1);
		expect(report.events[0]).toMatchObject({
			id: "main:run-1:tool:tool-1",
			stage: "tool_end",
			status: "timeout",
			name: "flowpilot_board",
			started_at_ms: 1_000,
			ended_at_ms: 1_250,
			duration_ms: 250,
			result_summary: "Timed out after 600 seconds",
			error: "Frontend response deadline exceeded",
		});
		expect(report.events[0]?.arguments_preview).toContain("app-1");
		expect(report.events[0]?.arguments_preview).not.toContain("hidden");
		expect(report.events[0]?.result_preview).toContain("timeout");
	});

	test("renders an undeployed database capability as partial instead of failed", () => {
		const event = debugEventFromCopilotStream(
			{
				type: "tool_end",
				data: {
					tool_call_id: "db-create-1",
					tool_name: "database_tool",
					status: "partial",
					terminal_status: "partial",
					result_preview: JSON.stringify({
						code: "explicit_schema_create_not_deployed",
					}),
				},
			},
			{
				scope: "nested",
				requestId: "board-request",
				parentRequestId: "parent-request",
				nowMs: 1_000,
			},
		);

		expect(event).toMatchObject({
			stage: "tool_end",
			status: "partial",
			terminal_status: "partial",
			name: "database_tool",
		});
		expect(event?.error).toBeUndefined();
	});

	test("preserves nested request correlation", () => {
		const event = debugEventFromCopilotStream(
			{
				type: "tool_start",
				data: {
					tool_call_id: "db-1",
					tool_name: "database_tool",
				},
			},
			{
				scope: "nested",
				requestId: "nested-flowpilot-1",
				parentRequestId: "outer-flowpilot-1",
				nowMs: 2_000,
			},
		);

		expect(event).toMatchObject({
			kind: "nested",
			request_id: "nested-flowpilot-1",
			parent_request_id: "outer-flowpilot-1",
			name: "database_tool",
		});
	});

	test("keeps nested board input, surfaced output, tool arguments/results, and successful final status in the parent report", () => {
		let report = createAgentDebugReport("parent-message", {
			startedAtMs: 900,
			inputPreview: "Build the support automation",
		});
		report = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "board-request",
				parentRequestId: "parent-message",
				toolName: "flowpilot_board",
				stage: "started",
				status: "running",
				input: {
					instruction: "Build the complete board and add the event last",
					app_id: "app-1",
					board_id: "board-1",
				},
				nowMs: 1_000,
			}),
		);

		for (const event of [
			debugEventFromCopilotStream(
				{
					type: "tool_start",
					data: {
						tool_call_id: "edit-1",
						tool_name: "edit_flowscript",
						arguments: JSON.stringify({
							flowscript: "eventsSimple() { buildSupportReply() }",
						}),
					},
				},
				{
					scope: "nested",
					requestId: "board-request",
					parentRequestId: "parent-message",
					nowMs: 1_100,
				},
			),
			debugEventFromCopilotStream(
				{
					type: "tool_end",
					data: {
						tool_call_id: "edit-1",
						tool_name: "edit_flowscript",
						status: "done",
						terminal_status: "queued",
						result_summary: "FlowScript queued",
						result_preview: JSON.stringify({
							status: "queued",
							commands: 14,
						}),
					},
				},
				{
					scope: "nested",
					requestId: "board-request",
					parentRequestId: "parent-message",
					nowMs: 1_250,
				},
			),
		]) {
			if (!event) throw new Error("Expected a nested tool event.");
			report = recordAgentDebugEvent(report, event);
		}

		report = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "board-request",
				parentRequestId: "parent-message",
				toolName: "flowpilot_board",
				stage: "finished",
				status: "ok",
				output: {
					message: "The support workflow was built and validated.",
					status: "ok",
					applied_commands: 14,
				},
				nowMs: 1_400,
			}),
		);

		const nestedRun = report.events.find(
			(event) => event.id === "nested:board-request:run",
		);
		expect(nestedRun).toMatchObject({
			kind: "nested",
			stage: "nested_run_finished",
			status: "done",
			terminal_status: "ok",
			name: "flowpilot_board",
			request_id: "board-request",
			parent_request_id: "parent-message",
			started_at_ms: 1_000,
			ended_at_ms: 1_400,
			duration_ms: 400,
		});
		expect(nestedRun?.arguments_preview).toContain("Build the complete board");
		expect(nestedRun?.result_preview).toContain("support workflow was built");

		const nestedTool = report.events.find(
			(event) => event.id === "nested:board-request:tool:edit-1",
		);
		expect(nestedTool).toMatchObject({
			stage: "tool_end",
			status: "done",
			terminal_status: "queued",
			name: "edit_flowscript",
		});
		expect(nestedTool?.arguments_preview).toContain("eventsSimple");
		expect(nestedTool?.result_preview).toContain("commands");
		expect(report.input_preview).toContain("support automation");
		expect(summarizeAgentDebugRootOutcomes(report.events)).toEqual({
			recordedTimeout: false,
			recordedPartial: false,
			recordedError: false,
		});
	});

	test("records widget nested tool activity and normalizes successful final status", () => {
		let report = createAgentDebugReport("parent-widget", {
			startedAtMs: 1_000,
		});
		report = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "widget-request",
				parentRequestId: "parent-widget",
				toolName: "flowpilot_widget",
				stage: "started",
				input: {
					instruction: "Create the ticket review page",
					page_name: "Ticket Review",
				},
				nowMs: 1_100,
			}),
		);

		const toolStart = debugEventFromCopilotStream(
			{
				type: "tool_start",
				data: {
					tool_call_id: "schema-1",
					tool: "get_component_schema",
					arguments_preview: JSON.stringify({ component: "button" }),
				},
			},
			{
				scope: "nested",
				requestId: "widget-request",
				parentRequestId: "parent-widget",
				nowMs: 1_200,
			},
		);
		const toolEnd = debugEventFromCopilotStream(
			{
				type: "tool_end",
				data: {
					tool_call_id: "schema-1",
					tool: "get_component_schema",
					status: "success",
					result_preview: JSON.stringify({ component: "button", valid: true }),
				},
			},
			{
				scope: "nested",
				requestId: "widget-request",
				parentRequestId: "parent-widget",
				nowMs: 1_300,
			},
		);
		if (!toolStart || !toolEnd) throw new Error("Expected widget tool events.");
		report = recordAgentDebugEvent(report, toolStart);
		report = recordAgentDebugEvent(report, toolEnd);
		report = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "widget-request",
				parentRequestId: "parent-widget",
				toolName: "flowpilot_widget",
				stage: "finished",
				status: "completed",
				output: {
					message: "Ticket review page generated.",
					component_count: 8,
					staged: true,
				},
				nowMs: 1_500,
			}),
		);

		expect(
			report.events.find(
				(event) => event.id === "nested:widget-request:tool:schema-1",
			),
		).toMatchObject({
			kind: "nested",
			status: "done",
			terminal_status: "success",
			name: "get_component_schema",
		});
		const nestedRun = report.events.find(
			(event) => event.id === "nested:widget-request:run",
		);
		expect(nestedRun).toMatchObject({
			stage: "nested_run_finished",
			status: "done",
			terminal_status: "completed",
		});
		expect(nestedRun?.arguments_preview).toContain("ticket review page");
		expect(nestedRun?.result_preview).toContain("component_count");
		expect(summarizeAgentDebugRootOutcomes(report.events).recordedError).toBe(
			false,
		);
	});

	test("records split nested widget stream frames, tool I/O, and generated artifacts", () => {
		let report = createAgentDebugReport("parent-widget-stream", {
			startedAtMs: 1_000,
		});
		let nowMs = 1_100;
		const recorder = createAgentDebugStreamRecorder({
			scope: "nested",
			requestId: "widget-stream-request",
			parentRequestId: "parent-widget-stream",
			enabled: true,
			record: (event) => {
				report = recordAgentDebugEvent(report, event);
			},
			nowMs: () => {
				nowMs += 25;
				return nowMs;
			},
		});

		const toolStart = `<tool_start>${JSON.stringify({
			tool_call_id: "widget-schema-1",
			tool: "get_component_schema",
			arguments: { component: "button", include_actions: true },
		})}</tool_start>`;
		const components = `<components>${JSON.stringify([
			{ id: "ticket-root", component: "box" },
			{ id: "approve-ticket", component: "button" },
		])}</components>`;
		const toolEnd = `<tool_end>${JSON.stringify({
			tool_call_id: "widget-schema-1",
			tool: "get_component_schema",
			status: "success",
			result: { component: "button", valid: true },
		})}</tool_end>`;

		// Exercise a real stream boundary inside an opening control tag. The shared parser must hold
		// the fragment and the recorder must still correlate the resulting nested events.
		recorder.push(toolStart.slice(0, 7));
		recorder.push(`${toolStart.slice(7)}${components}${toolEnd}`);
		recorder.flush();

		const tool = report.events.find(
			(event) =>
				event.id === "nested:widget-stream-request:tool:widget-schema-1",
		);
		expect(tool).toMatchObject({
			kind: "nested",
			stage: "tool_end",
			status: "done",
			name: "get_component_schema",
			request_id: "widget-stream-request",
			parent_request_id: "parent-widget-stream",
		});
		expect(tool?.arguments_preview).toContain("include_actions");
		expect(tool?.result_preview).toContain("valid");
		expect(
			report.events.find(
				(event) =>
					event.kind === "nested" &&
					event.stage === "artifact" &&
					event.name === "components",
			),
		).toMatchObject({
			status: "done",
			request_id: "widget-stream-request",
			parent_request_id: "parent-widget-stream",
		});
		expect(
			report.events.find((event) => event.name === "components")
				?.result_preview,
		).toContain("approve-ticket");
	});

	test("converts nested widget and board artifacts into bounded parent-report events", () => {
		const components = debugEventFromCopilotStream(
			{
				type: "components",
				data: [
					{ id: "root", component: "box" },
					{ id: "approve", component: "button" },
				],
			},
			{
				scope: "nested",
				requestId: "widget-artifacts",
				parentRequestId: "parent-artifacts",
				nowMs: 1_000,
			},
		);
		const workspace = debugEventFromCopilotStream(
			{
				type: "flowscript_workspace",
				data: {
					status: "queued",
					source: "eventsSimple() { processTickets() }",
				},
			},
			{
				scope: "nested",
				requestId: "board-artifacts",
				parentRequestId: "parent-artifacts",
				nowMs: 1_100,
			},
		);

		expect(components).toMatchObject({
			kind: "nested",
			stage: "artifact",
			status: "done",
			name: "components",
			request_id: "widget-artifacts",
			parent_request_id: "parent-artifacts",
		});
		expect(components?.result_summary).toContain("2");
		expect(components?.result_preview).toContain("approve");
		expect(workspace).toMatchObject({
			kind: "nested",
			stage: "artifact",
			status: "done",
			terminal_status: "queued",
			name: "flowscript_workspace",
			request_id: "board-artifacts",
		});
		expect(workspace?.result_preview).toContain("processTickets");
	});

	test("keeps live FlowScript drafting snapshots out of durable debug artifacts", () => {
		const drafting = debugEventFromCopilotStream(
			{
				type: "flowscript_workspace",
				data: {
					status: "drafting",
					source: "function buildReply() {\n  emailSmtpSend({",
				},
			},
			{
				scope: "nested",
				requestId: "live-draft",
				parentRequestId: "parent-live-draft",
				nowMs: 1_050,
			},
		);
		const submitted = debugEventFromCopilotStream(
			{
				type: "flowscript_workspace",
				data: {
					status: "submitted",
					source:
						'function buildReply() {\n  emailSmtpSend({ to: "reviewer@example.com" })\n}',
				},
			},
			{
				scope: "nested",
				requestId: "live-draft",
				parentRequestId: "parent-live-draft",
				nowMs: 1_100,
			},
		);

		expect(drafting).toBeNull();
		expect(submitted).toMatchObject({
			stage: "artifact",
			name: "flowscript_workspace",
			terminal_status: "submitted",
		});
	});

	test("retains large redacted FlowScript and component artifacts up to the evidence limit", () => {
		const largeFlowScript = [
			"function buildSupportBoard() {",
			"x".repeat(20 * 1024),
			'const SMTP_PASSWORD: string = "mail-secret";',
			"// FLOWSCRIPT_EVIDENCE_TAIL",
			"}",
		].join("\n");
		const largeComponentDescription = `${"c".repeat(20 * 1024)}COMPONENT_EVIDENCE_TAIL`;
		const workspace = debugEventFromCopilotStream(
			{
				type: "flowscript_workspace",
				data: { status: "queued", source: largeFlowScript },
			},
			{
				scope: "nested",
				requestId: "large-board-artifact",
				parentRequestId: "large-parent",
				nowMs: 1_000,
			},
		);
		const components = debugEventFromCopilotStream(
			{
				type: "components",
				data: [
					{
						id: "ticket-review",
						component: "box",
						props: {
							description: largeComponentDescription,
							apiKey: "component-secret",
						},
					},
				],
			},
			{
				scope: "nested",
				requestId: "large-widget-artifact",
				parentRequestId: "large-parent",
				nowMs: 1_100,
			},
		);

		expect(workspace?.result_preview?.length).toBeGreaterThan(8 * 1024);
		expect(workspace?.result_preview?.length).toBeLessThanOrEqual(32 * 1024);
		expect(workspace?.result_preview).toContain("FLOWSCRIPT_EVIDENCE_TAIL");
		expect(workspace?.result_preview).not.toContain("mail-secret");
		expect(components?.result_preview?.length).toBeGreaterThan(8 * 1024);
		expect(components?.result_preview?.length).toBeLessThanOrEqual(32 * 1024);
		expect(components?.result_preview).toContain("COMPONENT_EVIDENCE_TAIL");
		expect(components?.result_preview).not.toContain("component-secret");
	});

	test("uses 8 KiB ordinary previews and 32 KiB nested run boundaries", () => {
		const ordinaryPayload = `${"o".repeat(12 * 1024)}ORDINARY_TAIL`;
		let report = createAgentDebugReport("preview-limits", {
			startedAtMs: 1_000,
		});
		report = recordAgentDebugEvent(report, {
			id: "ordinary-tool",
			kind: "tool",
			stage: "tool_end",
			status: "done",
			timestamp_ms: 1_100,
			arguments_preview: ordinaryPayload,
			result_preview: ordinaryPayload,
		});
		report = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "large-boundary",
				parentRequestId: "preview-limits",
				toolName: "flowpilot_board",
				stage: "started",
				input: {
					instruction: `${"i".repeat(20 * 1024)}NESTED_INPUT_TAIL`,
					password: "nested-input-secret",
				},
				nowMs: 1_200,
			}),
		);
		report = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "large-boundary",
				parentRequestId: "preview-limits",
				toolName: "flowpilot_board",
				stage: "finished",
				status: "ok",
				output: {
					message: `${"r".repeat(20 * 1024)}NESTED_OUTPUT_TAIL`,
					apiKey: "nested-output-secret",
				},
				nowMs: 1_300,
			}),
		);

		const ordinary = report.events.find(
			(event) => event.id === "ordinary-tool",
		);
		expect(ordinary?.arguments_preview?.length).toBeLessThanOrEqual(8 * 1024);
		expect(ordinary?.arguments_preview).toContain("… [truncated]");
		expect(ordinary?.arguments_preview).not.toContain("ORDINARY_TAIL");
		const boundary = report.events.find(
			(event) => event.id === "nested:large-boundary:run",
		);
		expect(boundary?.arguments_preview?.length).toBeGreaterThan(8 * 1024);
		expect(boundary?.arguments_preview).toContain("NESTED_INPUT_TAIL");
		expect(boundary?.result_preview?.length).toBeGreaterThan(8 * 1024);
		expect(boundary?.result_preview).toContain("NESTED_OUTPUT_TAIL");
		expect(boundary?.arguments_preview).not.toContain("nested-input-secret");
		expect(boundary?.result_preview).not.toContain("nested-output-secret");
	});

	test("retains recovered nested failures without mislabeling the successful parent run", () => {
		let report = createAgentDebugReport("parent-retry", { startedAtMs: 1_000 });
		report = recordAgentDebugEvent(report, {
			id: "nested:retry-request:tool:first-edit",
			kind: "nested",
			stage: "tool_end",
			status: "error",
			request_id: "retry-request",
			parent_request_id: "parent-retry",
			timestamp_ms: 1_200,
			ended_at_ms: 1_200,
			error: "The first FlowScript candidate did not validate.",
		});
		report = recordAgentDebugEvent(report, {
			id: "nested:retry-request:tool:second-edit",
			kind: "nested",
			stage: "tool_end",
			status: "done",
			terminal_status: "queued",
			request_id: "retry-request",
			parent_request_id: "parent-retry",
			timestamp_ms: 1_300,
			ended_at_ms: 1_300,
			result_preview: JSON.stringify({ status: "queued" }),
		});
		report = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "retry-request",
				parentRequestId: "parent-retry",
				toolName: "flowpilot_board",
				stage: "finished",
				status: "ok",
				output: { status: "ok", applied_commands: 9 },
				nowMs: 1_400,
			}),
		);

		expect(
			report.events.find((event) => event.id.endsWith("first-edit"))?.error,
		).toContain("did not validate");
		expect(summarizeAgentDebugRootOutcomes(report.events)).toEqual({
			recordedTimeout: false,
			recordedPartial: false,
			recordedError: false,
		});

		const failedFinal = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "failed-request",
				parentRequestId: "parent-retry",
				toolName: "flowpilot_board",
				stage: "finished",
				status: "validation_errors",
				output: { status: "validation_errors" },
				nowMs: 1_500,
			}),
		);
		expect(
			summarizeAgentDebugRootOutcomes(failedFinal.events).recordedError,
		).toBe(true);
	});

	test("renders nested correlation, input, output, and terminal status in markdown", () => {
		let report = createAgentDebugReport("parent-markdown", {
			startedAtMs: 1_000,
		});
		report = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "nested-markdown",
				parentRequestId: "parent-markdown",
				toolName: "flowpilot_widget",
				stage: "started",
				input: { instruction: "MARKDOWN_NESTED_INPUT" },
				nowMs: 1_100,
			}),
		);
		report = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "nested-markdown",
				parentRequestId: "parent-markdown",
				toolName: "flowpilot_widget",
				stage: "finished",
				status: "success",
				output: { message: "MARKDOWN_NESTED_OUTPUT", staged: true },
				nowMs: 1_300,
			}),
		);
		const markdown = agentDebugReportAsMarkdown(report);

		expect(markdown).toContain("nested-markdown");
		expect(markdown).toContain("parent-markdown");
		expect(markdown).toContain("MARKDOWN_NESTED_INPUT");
		expect(markdown).toContain("MARKDOWN_NESTED_OUTPUT");
		expect(markdown).toContain("success");
	});

	test("preserves nested run input and output while bounding surrounding noise", () => {
		let report = createAgentDebugReport("parent-bounded", {
			startedAtMs: 1_000,
		});
		report = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "bounded-request",
				parentRequestId: "parent-bounded",
				toolName: "flowpilot_widget",
				stage: "started",
				input: { instruction: "PRESERVE_NESTED_INPUT", app_id: "app-1" },
				nowMs: 1_050,
			}),
		);
		for (let index = 0; index < 300; index += 1) {
			report = recordAgentDebugEvent(report, {
				id: `noise-${index}`,
				kind: "tool",
				stage: "tool_end",
				status: "done",
				timestamp_ms: 1_100 + index,
				arguments_preview: JSON.stringify({ value: "🙂".repeat(1_000) }),
				result_preview: JSON.stringify({ value: "🚀".repeat(1_000) }),
			});
		}
		report = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "bounded-request",
				parentRequestId: "parent-bounded",
				toolName: "flowpilot_widget",
				stage: "finished",
				status: "ok",
				output: { message: "PRESERVE_NESTED_OUTPUT", component_count: 12 },
				nowMs: 2_000,
			}),
		);
		report = finalizeAgentDebugReport(report, {
			outcome: "ok",
			terminalStage: "completed",
			endedAtMs: 2_100,
		});

		const nestedRun = report.events.find(
			(event) => event.id === "nested:bounded-request:run",
		);
		expect(nestedRun?.arguments_preview).toContain("PRESERVE_NESTED_INPUT");
		expect(nestedRun?.result_preview).toContain("PRESERVE_NESTED_OUTPUT");
		expect(
			new TextEncoder().encode(JSON.stringify(report)).byteLength,
		).toBeLessThanOrEqual(512 * 1024);
	});

	test("prunes plan and progress payloads before artifact and nested boundary evidence", () => {
		let report = createAgentDebugReport("priority-parent", {
			startedAtMs: 1_000,
		});
		report = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "priority-boundary",
				parentRequestId: "priority-parent",
				toolName: "flowpilot_board",
				stage: "started",
				input: {
					instruction: `${"i".repeat(24 * 1024)}PRIORITY_INPUT_TAIL`,
				},
				nowMs: 1_010,
			}),
		);
		const artifact = debugEventFromCopilotStream(
			{
				type: "flowscript_workspace",
				data: {
					status: "queued",
					source: `${"f".repeat(24 * 1024)}PRIORITY_FLOWSCRIPT_TAIL`,
				},
			},
			{
				scope: "nested",
				requestId: "priority-boundary",
				parentRequestId: "priority-parent",
				nowMs: 1_020,
			},
		);
		if (!artifact) throw new Error("Expected FlowScript artifact evidence.");
		report = recordAgentDebugEvent(report, artifact);

		for (let index = 0; index < 120; index += 1) {
			report = recordAgentDebugEvent(report, {
				id: `noisy-plan-${index}`,
				kind: "plan",
				stage: "plan",
				status: "progress",
				timestamp_ms: 1_100 + index,
				arguments_preview: `PLAN_INPUT_${index}_${"a".repeat(8 * 1024)}`,
				result_preview: `PLAN_OUTPUT_${index}_${"b".repeat(8 * 1024)}`,
				reasoning: `PLAN_REASONING_${index}_${"c".repeat(8 * 1024)}`,
			});
		}
		report = recordAgentDebugEvent(
			report,
			nestedAgentRunEvent({
				requestId: "priority-boundary",
				parentRequestId: "priority-parent",
				toolName: "flowpilot_board",
				stage: "finished",
				status: "ok",
				output: {
					message: `${"o".repeat(24 * 1024)}PRIORITY_OUTPUT_TAIL`,
				},
				nowMs: 2_000,
			}),
		);

		const boundary = report.events.find(
			(event) => event.id === "nested:priority-boundary:run",
		);
		const retainedArtifact = report.events.find(
			(event) => event.stage === "artifact",
		);
		expect(boundary?.arguments_preview).toContain("PRIORITY_INPUT_TAIL");
		expect(boundary?.result_preview).toContain("PRIORITY_OUTPUT_TAIL");
		expect(retainedArtifact?.result_preview).toContain(
			"PRIORITY_FLOWSCRIPT_TAIL",
		);
		expect(
			report.events.some(
				(event) =>
					event.stage === "plan" &&
					(event.arguments_preview?.length ?? Number.POSITIVE_INFINITY) <= 512,
			),
		).toBe(true);
		expect(retainedArtifact?.result_preview?.length).toBeGreaterThan(8 * 1024);
		expect(boundary?.arguments_preview?.length).toBeGreaterThan(8 * 1024);
		expect(report.truncation?.bytes_dropped).toBeGreaterThan(0);
		expect(
			new TextEncoder().encode(JSON.stringify(report)).byteLength,
		).toBeLessThanOrEqual(512 * 1024);
	});

	test("redacts secrets in objects, JSON strings, bearer headers, and signed URLs", () => {
		const objectPreview = agentDebugPreview({
			accessToken: "access-secret",
			apiKey: "api-secret",
			clientSecret: "client-secret",
			password: "password-secret",
			safe: "visible",
		});
		const stringPreview = agentDebugPreview(
			JSON.stringify({ token: "json-secret", safe: "json-visible" }),
		);
		const textPreview = agentDebugPreview(
			"Authorization: Bearer bearer-secret clientSecret=client-text-secret accessToken=access-text-secret https://example.test/file?X-Amz-Signature=signed-secret&X-Goog-Credential=google-secret&token=query-secret",
		);
		const typedFlowScriptPreview = agentDebugPreview(
			'@secret\nconst IMAP_PASSWORD: string = "mailbox-secret";\nconst API_KEY: string = "provider-secret";\n@secret const IMAP_HOST: string = "private-mail.example";\n@secret\nconst USERNAME: string = "private-user";',
		);

		expect(objectPreview).toContain("visible");
		expect(objectPreview).not.toMatch(
			/access-secret|api-secret|client-secret|password-secret/,
		);
		expect(stringPreview).toContain("json-visible");
		expect(stringPreview).not.toContain("json-secret");
		expect(textPreview).not.toMatch(
			/bearer-secret|client-text-secret|access-text-secret|signed-secret|google-secret|query-secret/,
		);
		expect(textPreview).toContain("[REDACTED]");
		expect(typedFlowScriptPreview).toContain("[REDACTED]");
		expect(typedFlowScriptPreview).not.toMatch(
			/mailbox-secret|provider-secret|private-mail\.example|private-user/,
		);
	});

	test("keeps diagnostics beside redacted FlowScript in nested MCP text results", () => {
		const preview = agentDebugPreview(
			JSON.stringify([
				{
					type: "text",
					text: JSON.stringify({
						status: "validation_errors",
						source: 'prefix @secret const innocuous: string = "nested-secret"',
						structured_diagnostics: [
							{
								code: "IR_REQUEST_APPROVAL_UI_ACTIONS_MISSING",
								phase: "acceptance",
							},
						],
					}),
				},
			]),
		);

		expect(preview).toContain("validation_errors");
		expect(preview).toContain("IR_REQUEST_APPROVAL_UI_ACTIONS_MISSING");
		expect(preview).not.toContain("nested-secret");
	});

	test("keeps a redacted debug preview when @secret precedes another variable decorator", () => {
		const preview = agentDebugPreview(`@secret
@category("IMAP")
const imap_password: string = "mailbox-secret"

eventsSimple() {
  logInfo({ message: "safe" })
}`);

		expect(preview).toContain('@category("IMAP")');
		expect(preview).toContain('const imap_password: string = ""');
		expect(preview).not.toContain("FlowScript source omitted");
		expect(preview).not.toContain("mailbox-secret");
	});

	test("labels fail-closed debug omission as persistence-only", () => {
		const preview = agentDebugPreview(
			'prefix @secret const innocuous: string = "must-not-leak"',
		);

		expect(preview).toContain("Persisted FlowScript copy omitted");
		expect(preview).toContain("not a parser/reconcile error");
		expect(preview).not.toContain("must-not-leak");
	});

	test("keeps long FlowScript object previews valid and safe across repeated normalization", () => {
		const source = `@secret
const innocuous: string = "opaque-secret"
${"// safe filler\n".repeat(900)}`;

		const first = agentDebugPreview({ source });
		const second = agentDebugPreview(first);

		expect(() => JSON.parse(first ?? "")).not.toThrow();
		expect(() => JSON.parse(second ?? "")).not.toThrow();
		expect(first).not.toContain("opaque-secret");
		expect(second).not.toContain("opaque-secret");
		expect(second).not.toContain("FlowScript source omitted");
		expect(second).toContain("const innocuous: string");
	});

	test("redacts FlowScript inside workspace envelopes without misreporting line one", () => {
		const source = `@secret
@category("Runtime")
const imap_password: string = "workspace-secret"
${"// safe filler\n".repeat(900)}`;
		const envelope = `<flowscript_workspace>${JSON.stringify({
			source,
			status: "validation_errors",
		})}</flowscript_workspace>`;

		const first = agentDebugPreview(envelope);
		const second = agentDebugPreview(first);

		expect(first).toContain("<flowscript_workspace>");
		expect(first).toContain("</flowscript_workspace>");
		expect(second).toContain("<flowscript_workspace>");
		expect(second).not.toContain("workspace-secret");
		expect(second).not.toContain("Persisted FlowScript copy omitted");
		expect(second).not.toContain("Unsupported @secret annotation at line 1");
	});

	test("redacts values whose sibling marks a command variable as secret", () => {
		const preview = agentDebugPreview({
			category: "visible-category",
			command: "CreateVariable",
			default_value: "opaque-default",
			name: "innocuous",
			secret: true,
			value: "opaque-value",
		});

		expect(preview).toContain("visible-category");
		expect(preview).toContain("[REDACTED]");
		expect(preview).not.toMatch(/opaque-default|opaque-value/);
	});

	test("does not let response delivery hide an authoritative failed request", () => {
		const events = [
			{
				id: "frontend:outer:request",
				kind: "bridge" as const,
				stage: "request_failed",
				status: "error",
				request_id: "outer",
				timestamp_ms: 1_000,
			},
			{
				id: "frontend:outer:delivery",
				kind: "bridge" as const,
				stage: "response_delivered",
				status: "done",
				request_id: "outer",
				timestamp_ms: 1_100,
			},
			nestedAgentRunEvent({
				requestId: "outer:agent",
				parentRequestId: "outer",
				toolName: "flowpilot_board",
				stage: "finished",
				status: "error",
				nowMs: 1_050,
			}),
		];

		expect(summarizeAgentDebugRootOutcomes(events)).toMatchObject({
			recordedError: true,
		});
	});

	test("retains recovered main tool and plan failures without marking a successful run red", () => {
		const events = [
			{
				id: "main:run:plan:first-attempt",
				kind: "plan" as const,
				stage: "plan",
				status: "error",
				request_id: "run",
				timestamp_ms: 1_000,
				error: "The first approach could not resolve a declaration.",
			},
			{
				id: "main:run:tool:first-edit",
				kind: "tool" as const,
				stage: "tool_end",
				status: "error",
				request_id: "run",
				timestamp_ms: 1_100,
				error: "The first FlowScript edit failed validation.",
			},
			{
				id: "main:run:tool:second-edit",
				kind: "tool" as const,
				stage: "tool_end",
				status: "done",
				request_id: "run",
				timestamp_ms: 1_200,
			},
			{
				id: "main:run:lifecycle:finish",
				kind: "lifecycle" as const,
				stage: "run_finished",
				status: "done",
				request_id: "run",
				timestamp_ms: 1_300,
				ended_at_ms: 1_300,
			},
		];

		expect(summarizeAgentDebugRootOutcomes(events)).toEqual({
			recordedTimeout: false,
			recordedPartial: false,
			recordedError: false,
		});
	});

	test("bounds noisy reports by event count and UTF-8 byte size", () => {
		let report = createAgentDebugReport("run-large", { startedAtMs: 1_000 });
		for (let index = 0; index < 300; index += 1) {
			report = recordAgentDebugEvent(report, {
				id: `tool-${index}`,
				kind: "tool",
				stage: "tool_end",
				status: "done",
				timestamp_ms: 1_000 + index,
				summary: `Tool ${index} ${"summary".repeat(100)}`,
				arguments_preview: JSON.stringify({ payload: "🙂".repeat(1_500) }),
				result_preview: JSON.stringify({ result: "🚀".repeat(1_500) }),
			});
		}
		report = finalizeAgentDebugReport(report, {
			outcome: "partial",
			terminalStage: "bounded",
			summary: "The run produced more diagnostics than the persistence limit.",
			endedAtMs: 2_000,
		});

		const bytes = new TextEncoder().encode(JSON.stringify(report)).byteLength;
		expect(report.schema).toBe(AGENT_DEBUG_REPORT_SCHEMA);
		expect(report.events.length).toBeLessThanOrEqual(256);
		expect(report.truncation?.events_dropped).toBeGreaterThan(0);
		expect(bytes).toBeLessThanOrEqual(512 * 1024);
		expect(report).toMatchObject({
			outcome: "partial",
			terminal_stage: "bounded",
			duration_ms: 1_000,
		});
	});

	test("records bounded generation evaluation evidence from planning and validation tools", () => {
		let report = createAgentDebugReport("generation-run", {
			startedAtMs: 1_000,
		});
		const recordToolEnd = (
			id: string,
			name: string,
			result: unknown,
			nowMs: number,
		) => {
			const event = debugEventFromCopilotStream(
				{
					type: "tool_end",
					data: {
						tool_call_id: id,
						tool_name: name,
						status: "done",
						result,
					},
				},
				{ scope: "main", requestId: "generation-run", nowMs },
			);
			if (!event) throw new Error("Expected a tool_end debug event.");
			report = recordAgentDebugEvent(report, event);
		};

		recordToolEnd("plan", "plan_flow_ir", { feasible: true }, 1_100);
		recordToolEnd(
			"validate",
			"validate_flow_ir_draft",
			{
				status: "draft_valid",
				draft_id: "mail-flow",
				revision: 4,
				diagnostics: [],
				missing_modules: [],
				capability_plan: { feasible: true },
				flowscript: "function run() { /* token=very-secret */ }",
			},
			1_500,
		);
		recordToolEnd(
			"commit",
			"commit_flow_ir_draft",
			{
				status: "queued",
				draft_id: "mail-flow",
				revision: 4,
				selected_revision: 4,
				diagnostics: [],
				queued_count: 8,
			},
			1_700,
		);
		recordToolEnd(
			"raw-invalid",
			"edit_flowscript",
			'<flowscript_workspace>{"source":"run() {}","status":"validation_errors"}</flowscript_workspace>\n' +
				'<structured_diagnostics>[{"code":"FS_TYPE_MISMATCH","phase":"type_check","message":"token=very-secret"}]</structured_diagnostics>',
			1_900,
		);
		recordToolEnd(
			"board-result",
			"flowpilot_board",
			{ status: "success", final_board_node_count: 3 },
			1_950,
		);

		report = finalizeAgentDebugReport(report, {
			outcome: "error",
			terminalStage: "validation_failed",
			endedAtMs: 2_000,
		});

		expect(report.generation_evaluation).toEqual({
			version: "flowpilot.generation-evaluation/v1",
			run_id: "generation-run",
			status: "failed",
			plan_outcome: "feasible",
			final_board_node_count: 3,
			attempts: [
				{
					attempt_index: 1,
					elapsed_ms: 500,
					parse_valid: true,
					typed_valid: true,
					reconcile_valid: true,
					accepted: false,
					diagnostic_keys: undefined,
				},
				{
					attempt_index: 2,
					elapsed_ms: 900,
					parse_valid: true,
					typed_valid: false,
					reconcile_valid: false,
					accepted: false,
					diagnostic_keys: ["FS_TYPE_MISMATCH"],
				},
			],
		});
		expect(JSON.stringify(report.generation_evaluation)).not.toContain(
			"very-secret",
		);
		expect(
			new TextEncoder().encode(JSON.stringify(report)).byteLength,
		).toBeLessThanOrEqual(512 * 1024);
	});

	test("records source-lifecycle diagnostics from nested MCP text results", () => {
		let report = createAgentDebugReport("source-lifecycle", {
			startedAtMs: 1_000,
		});
		const event = debugEventFromCopilotStream(
			{
				type: "tool_end",
				data: {
					tool_call_id: "check-1",
					tool_name: "check_flowscript",
					status: "error",
					result: JSON.stringify([
						{
							type: "text",
							text: JSON.stringify({
								status: "validation_errors",
								structured_diagnostics: [
									{
										code: "FS_TYPE_MISMATCH",
										phase: "type_check",
									},
								],
							}),
						},
					]),
				},
			},
			{ scope: "nested", requestId: "source-lifecycle", nowMs: 1_200 },
		);
		if (!event) throw new Error("Expected a source lifecycle tool event.");
		report = recordAgentDebugEvent(report, event);

		expect(report.generation_evaluation?.attempts).toEqual([
			{
				attempt_index: 1,
				elapsed_ms: 200,
				parse_valid: true,
				typed_valid: false,
				reconcile_valid: false,
				accepted: false,
				diagnostic_keys: ["FS_TYPE_MISMATCH"],
			},
		]);
	});

	test("records every typed draft mutation schema failure as a distinct attempt", () => {
		let report = createAgentDebugReport("typed-schema-failures", {
			startedAtMs: 1_000,
		});
		const failures = [
			["begin", "begin_flow_ir_draft", "IR_DRAFT_INVALID", undefined],
			[
				"update",
				"update_flow_ir_draft",
				"IR_DRAFT_UPDATE_INVALID",
				[{ code: "IR_ROOT_CAUSE", phase: "draft" }],
			],
			["upsert", "upsert_flow_ir_module", "IR_MODULE_INVALID", undefined],
			[
				"validate",
				"validate_flow_ir_draft",
				"IR_DRAFT_VALIDATION_INVALID",
				undefined,
			],
			["commit", "commit_flow_ir_draft", "IR_COMMIT_INVALID", undefined],
		] as const;

		failures.forEach(([id, name, code, diagnostics], index) => {
			const event = debugEventFromCopilotStream(
				{
					type: "tool_end",
					data: {
						tool_call_id: id,
						tool_name: name,
						status: "done",
						result: {
							status: "validation_errors",
							code,
							diagnostics,
						},
					},
				},
				{
					scope: "main",
					requestId: "typed-schema-failures",
					nowMs: 1_100 + index * 100,
				},
			);
			if (!event) throw new Error("Expected typed tool evidence.");
			report = recordAgentDebugEvent(report, event);
		});

		expect(report.generation_evaluation?.attempts).toHaveLength(5);
		expect(
			report.generation_evaluation?.attempts.map((attempt) => ({
				parse: attempt.parse_valid,
				typed: attempt.typed_valid,
				reconcile: attempt.reconcile_valid,
				accepted: attempt.accepted,
				keys: attempt.diagnostic_keys,
			})),
		).toEqual([
			{
				parse: false,
				typed: false,
				reconcile: false,
				accepted: false,
				keys: ["IR_DRAFT_INVALID"],
			},
			{
				parse: false,
				typed: false,
				reconcile: false,
				accepted: false,
				// Structured root identifiers supersede the wrapper code.
				keys: ["IR_ROOT_CAUSE"],
			},
			{
				parse: false,
				typed: false,
				reconcile: false,
				accepted: false,
				keys: ["IR_MODULE_INVALID"],
			},
			{
				parse: false,
				typed: false,
				reconcile: false,
				accepted: false,
				keys: ["IR_DRAFT_VALIDATION_INVALID"],
			},
			{
				parse: false,
				typed: false,
				reconcile: false,
				accepted: false,
				keys: ["IR_COMMIT_INVALID"],
			},
		]);
	});

	test("treats typed commit envelopes as queued reviews, not accepted applies", () => {
		let report = createAgentDebugReport("typed-commit-tags", {
			startedAtMs: 1_000,
		});
		const workspace =
			'<flowscript_workspace>{"source":"function run() {}","status":"queued"}</flowscript_workspace>';
		const commands = '<commands>[{"CreateNode":{"id":"node-1"}}]</commands>';
		const results = [
			`${workspace}\n${commands}\n<typed_commit_result>{"status":"queued","draft_id":"draft-a","revision":2,"selected_revision":2,"diagnostics":[],"queued_count":1}</typed_commit_result>`,
			`${workspace}\n${commands}`,
		];

		results.forEach((result, index) => {
			const event = debugEventFromCopilotStream(
				{
					type: "tool_end",
					data: {
						tool_call_id: `commit-${index}`,
						tool_name: "commit_flow_ir_draft",
						status: "done",
						result,
					},
				},
				{
					scope: "main",
					requestId: "typed-commit-tags",
					nowMs: 1_200 + index * 100,
				},
			);
			if (!event) throw new Error("Expected commit tool evidence.");
			report = recordAgentDebugEvent(report, event);
		});

		expect(report.generation_evaluation?.attempts).toHaveLength(2);
		expect(report.generation_evaluation?.attempts).toEqual([
			{
				attempt_index: 1,
				elapsed_ms: 200,
				parse_valid: true,
				typed_valid: true,
				reconcile_valid: true,
				accepted: false,
				diagnostic_keys: undefined,
			},
			{
				attempt_index: 2,
				elapsed_ms: 300,
				parse_valid: true,
				typed_valid: true,
				reconcile_valid: true,
				accepted: false,
				diagnostic_keys: undefined,
			},
		]);

		report = recordAgentDebugEvent(
			report,
			agentGenerationReviewDispositionEvent({
				requestId: "typed-commit-tags",
				disposition: "applied",
				draftId: "draft-a",
				revision: 2,
				claimId: "claim-a",
				nowMs: 1_400,
			}),
		);
		expect(
			report.generation_evaluation?.attempts.map((attempt) => attempt.accepted),
		).toEqual([true, false]);
	});

	test("emits aggregate-only production metrics after an actual apply", () => {
		const sink = vi.fn();
		const clearSink = setFlowPilotProductionMetricsSink(sink);
		beginAgentGenerationMetrics("private-message-id", 1_000);
		const queued = debugEventFromCopilotStream(
			{
				type: "tool_end",
				data: {
					tool_call_id: "commit",
					tool_name: "commit_flow_ir_draft",
					status: "done",
					result: {
						status: "queued",
						draft_id: "private-draft-id",
						revision: 7,
						selected_revision: 7,
						diagnostics: [],
						flowscript: "secret authored workflow",
					},
				},
			},
			{ scope: "nested", requestId: "private-request-id", nowMs: 1_100 },
		);
		if (!queued) throw new Error("Expected queued generation evidence.");
		recordAgentGenerationMetricEvent("private-message-id", queued);
		recordAgentGenerationMetricEvent(
			"private-message-id",
			agentGenerationReviewDispositionEvent({
				requestId: "private-request-id",
				disposition: "applied",
				draftId: "private-draft-id",
				revision: 7,
				claimId: "private-claim-id",
				nowMs: 1_200,
			}),
		);

		const metrics = finalizeAgentGenerationMetrics("private-message-id", "ok");
		clearSink();

		expect(metrics).toMatchObject({
			runs_started: 1,
			runs_succeeded: 1,
			attempts_total: 1,
			attempts_parse_valid: 1,
			attempts_typed_valid: 1,
			attempts_reconcile_valid: 1,
			attempts_applied: 1,
			queued_reviews: 1,
			apply_dispositions: 1,
			dismissed_dispositions: 0,
		});
		expect(sink).toHaveBeenCalledWith(metrics);
		expect(JSON.stringify(metrics)).not.toMatch(
			/private-message-id|private-request-id|private-draft-id|private-claim-id|secret authored workflow/,
		);
		for (const value of Object.values(metrics ?? {})) {
			expect(["number", "string"]).toContain(typeof value);
		}
	});

	test("publishes failed runs that produced no workflow candidate", () => {
		const sink = vi.fn();
		const clearSink = setFlowPilotProductionMetricsSink(sink);
		beginAgentGenerationMetrics("no-candidate-private-run", 1_000);

		const metrics = finalizeAgentGenerationMetrics(
			"no-candidate-private-run",
			"error",
		);
		clearSink();

		expect(metrics).toMatchObject({
			runs_started: 1,
			runs_succeeded: 0,
			runs_failed: 1,
			attempts_total: 0,
			attempts_parse_valid: 0,
			attempts_applied: 0,
		});
		expect(sink).toHaveBeenCalledWith(metrics);
		expect(JSON.stringify(metrics)).not.toContain("no-candidate-private-run");
	});

	test("production stream recording keeps verbose tool payloads out of events", () => {
		const metricEvents: Parameters<typeof recordAgentDebugEvent>[1][] = [];
		const recorder = createAgentDebugStreamRecorder({
			scope: "nested",
			requestId: "run",
			enabled: false,
			record: (event) => metricEvents.push(event),
			nowMs: () => 1_000,
		});
		recorder.push(
			`<tool_end>${JSON.stringify({
				tool_call_id: "commit",
				tool_name: "commit_flow_ir_draft",
				status: "done",
				result: {
					status: "queued",
					draft_id: "sensitive-draft",
					revision: 1,
					flowscript: "do not persist this source",
				},
			})}</tool_end>`,
		);
		recorder.flush();

		expect(metricEvents).toHaveLength(1);
		expect(metricEvents[0]).toMatchObject({
			stage: "tool_end",
			name: "commit_flow_ir_draft",
		});
		expect(JSON.stringify(metricEvents[0])).not.toMatch(
			/sensitive-draft|do not persist this source|result_preview|arguments_preview/,
		);
	});

	test("the production global-chat store publishes metrics without creating a debug report", () => {
		const sink = vi.fn();
		const clearSink = setFlowPilotProductionMetricsSink(sink);
		const store = useGlobalChatStore.getState();
		store.beginDebugReport("store-production-run", { startedAtMs: 1_000 });
		const queued = debugEventFromCopilotStream(
			{
				type: "tool_end",
				data: {
					tool_call_id: "commit",
					tool_name: "commit_flow_ir_draft",
					result: {
						status: "queued",
						draft_id: "store-draft",
						revision: 1,
						diagnostics: [],
					},
				},
			},
			{ scope: "main", requestId: "store-production-run", nowMs: 1_100 },
		);
		if (!queued) throw new Error("Expected queued generation evidence.");
		store.recordDebugEvent("store-production-run", queued);
		store.finalizeDebugReport("store-production-run", {
			outcome: "ok",
			terminalStage: "completed",
		});
		clearSink();

		expect(useGlobalChatStore.getState().debugReport).toBeNull();
		expect(sink).toHaveBeenCalledTimes(1);
		expect(sink.mock.calls[0]?.[0]).toMatchObject({
			queued_reviews: 1,
			attempts_applied: 0,
		});
	});
});

import { describe, expect, test } from "bun:test";
import { createCopilotStreamParser } from "../../components/flowpilot/copilot-stream-parser";
import {
	applyStreamEvent,
	createStreamAccumulator,
	readReplyWidget,
	toolEndPlanStepStatus,
} from "./copilot-stream-steps";

test("split native model identity frames remain metadata and never enter assistant prose", () => {
	const parser = createCopilotStreamParser();
	const acc = createStreamAccumulator();
	const identity = {
		bit_id: "claude-bit",
		provider_name: "custom:claude-code",
		configured_model_id: "claude-sonnet",
		model_id: "claude-sonnet",
	};
	const events = [
		...parser.push("The board is ready.<model_res"),
		...parser.push(`olved>${JSON.stringify(identity)}</model_`),
		...parser.push("resolved> Review the changes."),
		...parser.flush(),
	];
	for (const event of events) applyStreamEvent(acc, event);
	expect(events.find((event) => event.type === "model_resolved")?.data).toEqual(
		identity,
	);
	expect(acc.content).toBe("The board is ready. Review the changes.");
	expect(acc.steps.size).toBe(0);
});

test("cumulative usage snapshots replace the previous total for the same stream step", () => {
	const acc = createStreamAccumulator();
	for (const tokens of [100, 200, 200])
		applyStreamEvent(acc, {
			type: "usage_stat",
			raw: "",
			data: {
				step_name: "Assistant",
				stats: {
					usage: {
						prompt_tokens: tokens,
						completion_tokens: 10,
						total_tokens: tokens + 10,
					},
				},
			},
		});
	expect(acc.usageStats).toHaveLength(1);
	expect(acc.usageStats[0].stats.usage.total_tokens).toBe(210);
});

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

	test("keeps the tool name when progress arrives without a start event", () => {
		const acc = createStreamAccumulator();
		applyStreamEvent(acc, {
			type: "tool_progress",
			data: {
				tool_call_id: "edit-1",
				tool: "edit_flowscript",
				message: "Applying changes",
			},
			raw: "",
		});

		expect(acc.steps.get("edit-1")).toMatchObject({
			status: "progress",
			toolName: "edit_flowscript",
		});
	});
});

describe("tool activity lifecycle", () => {
	test("keeps native research delivered only as a completed item", () => {
		const acc = createStreamAccumulator();
		applyStreamEvent(acc, { type: "text", text: "Checking sources. " });
		applyStreamEvent(acc, {
			type: "tool_end",
			data: {
				tool_call_id: "native-search",
				tool: "web_search",
				status: "done",
				arguments_preview: JSON.stringify({
					queries: ["Panamera comfort", "BMW 7 Series comfort"],
				}),
			},
		});
		expect(acc.stepOrder).toEqual(["native-search"]);
		expect(acc.steps.get("native-search")).toMatchObject({
			toolName: "web_search",
			status: "done",
			content_offset: "Checking sources. ".length,
			detail: {
				category: "search",
				queries: ["Panamera comfort", "BMW 7 Series comfort"],
			},
		});
		expect(acc.steps.get("native-search")?.startTime).toBeUndefined();
	});

	test("uses a native page action's final tool name without duplicating its initial search item", () => {
		const acc = createStreamAccumulator();
		applyStreamEvent(acc, {
			type: "tool_start",
			data: { tool_call_id: "native-page", tool: "web_search" },
		});
		applyStreamEvent(acc, {
			type: "tool_progress",
			data: {
				tool_call_id: "native-page",
				tool: "open_url",
				arguments_preview: JSON.stringify({
					url: "https://reviews.example/panamera",
				}),
			},
		});
		expect(acc.steps.get("native-page")).toMatchObject({
			toolName: "open_url",
			status: "progress",
			detail: { label: "Reading a webpage", category: "web" },
		});
		applyStreamEvent(acc, {
			type: "tool_end",
			data: { tool_call_id: "native-page", status: "done" },
		});
		expect(acc.stepOrder).toEqual(["native-page"]);
		expect(acc.steps.get("native-page")).toMatchObject({
			title: "Using open_url",
			toolName: "open_url",
			status: "done",
			detail: {
				category: "web",
				label: "Reading a webpage",
				sources: [{ url: "https://reviews.example/panamera" }],
			},
		});
	});

	test("keeps thinking steps in the reasoning viewer", () => {
		const acc = createStreamAccumulator();
		applyStreamEvent(acc, {
			type: "plan_step",
			data: {
				id: "thinking-1",
				tool_name: "think",
				description: "Review the available context.",
				status: "progress",
			},
		});
		expect(acc.steps.get("thinking-1")).toMatchObject({
			title: "Thinking",
			reasoning: "Review the available context.",
		});
		expect(acc.steps.get("thinking-1")?.detail).toBeUndefined();
	});

	test("shows context checkpoints as progress, not as thinking", () => {
		const acc = createStreamAccumulator();
		applyStreamEvent(acc, {
			type: "plan_step",
			data: {
				id: "checkpoint_3",
				tool_name: "context_checkpoint",
				description: "Summarizing earlier steps so the work can continue",
				status: "progress",
			},
		});
		const step = acc.steps.get("checkpoint_3");
		expect(step?.title).not.toBe("Thinking");
		expect(step?.reasoning).toBeUndefined();
		expect(step?.detail).toMatchObject({ label: "Saving progress" });
	});

	test("retains request context and timing through progress, completion and plan updates", () => {
		const acc = createStreamAccumulator();
		applyStreamEvent(acc, { type: "text", text: "Checking the license. " });
		applyStreamEvent(acc, {
			type: "tool_start",
			data: {
				tool_call_id: "request-1",
				tool: "http_request",
				arguments_preview: JSON.stringify({
					url: "https://example.com/LICENSE?token=secret",
				}),
			},
		});
		const initial = acc.steps.get("request-1");
		applyStreamEvent(acc, { type: "text", text: "More context. " });
		applyStreamEvent(acc, {
			type: "tool_progress",
			data: { tool_call_id: "request-1", message: "Waiting for the response" },
		});
		applyStreamEvent(acc, {
			type: "tool_end",
			data: {
				tool_call_id: "request-1",
				status: "completed",
				result_preview: JSON.stringify({
					http_status: 200,
					body: "private response",
				}),
			},
		});
		applyStreamEvent(acc, {
			type: "plan_step",
			data: {
				id: "request-1",
				tool_name: "http_request",
				title: "Using http_request",
				status: "completed",
			},
		});
		expect(acc.steps.get("request-1")).toMatchObject({
			title: "Using http_request",
			status: "done",
			content_offset: initial?.content_offset,
			startTime: initial?.startTime,
			endTime: expect.any(Number),
			detail: {
				kind: "tool_activity",
				label: "Fetching a URL",
				method: "GET",
				sources: [
					{
						url: "https://example.com/LICENSE",
						hostname: "example.com",
						path: "/LICENSE",
						linkable: false,
					},
				],
				outcome: "HTTP 200",
			},
		});
		expect(acc.stepOrder).toEqual(["request-1"]);
		expect(acc.currentStepId).toBeUndefined();
		expect(JSON.stringify(acc.steps.get("request-1"))).not.toContain("secret");
		expect(JSON.stringify(acc.steps.get("request-1"))).not.toContain(
			"private response",
		);
	});

	test("learns context from progress after a start without arguments", () => {
		const acc = createStreamAccumulator();
		applyStreamEvent(acc, {
			type: "tool_start",
			data: { tool_call_id: "search-1", tool: "internet_search" },
		});
		applyStreamEvent(acc, {
			type: "tool_progress",
			data: {
				tool_call_id: "search-1",
				arguments: { query: "license terms" },
			},
		});
		expect(acc.steps.get("search-1")?.detail).toMatchObject({
			query: "license terms",
			category: "search",
		});
	});

	test("reads activity context from a wrapped plan step", () => {
		const acc = createStreamAccumulator();
		applyStreamEvent(acc, {
			type: "plan_step",
			data: {
				PlanStep: {
					id: "plan-1",
					tool_name: "open_url",
					status: "progress",
					arguments: { url: "https://example.com/license" },
				},
			},
		});
		expect(acc.steps.get("plan-1")?.detail).toMatchObject({
			sources: [
				{
					url: "https://example.com/license",
					hostname: "example.com",
					path: "/license",
				},
			],
		});
	});

	test("does not keep JSON argument dumps as step descriptions", () => {
		const acc = createStreamAccumulator();
		applyStreamEvent(acc, {
			type: "tool_start",
			data: {
				tool_call_id: "request-1",
				tool: "http_request",
				summary: '{"headers":{"Authorization":"Bearer private"}}',
			},
		});
		expect(acc.steps.get("request-1")?.description).toBeUndefined();
	});

	test("preserves richer build lane details", () => {
		const acc = createStreamAccumulator();
		acc.steps.set("build-1", {
			id: "build-1",
			title: "Building a workflow",
			status: "progress",
			toolName: "flowpilot_board",
			detail: {
				kind: "build_lane",
				lane: "workflow",
				target: "Order processing",
			},
		});
		applyStreamEvent(acc, {
			type: "tool_end",
			data: { tool_call_id: "build-1", status: "completed" },
		});
		expect(acc.steps.get("build-1")?.detail).toEqual({
			kind: "build_lane",
			lane: "workflow",
			target: "Order processing",
		});
	});
});

describe("reply widget frames", () => {
	const frame = {
		origin: { app_id: "app1", board_id: "b1", event_id: "e1" },
		widget: {
			instance_id: "w1",
			widget_id: "chart",
			surface_id: "s1",
			component: { widgetInstance: { widgetId: "chart" } },
			updates: [{ kind: "set" }],
		},
	};

	test("a valid frame becomes a widget bound to its app run", () => {
		expect(readReplyWidget(frame)).toEqual({
			instance_id: "w1",
			widget_id: "chart",
			surface_id: "s1",
			component: { widgetInstance: { widgetId: "chart" } },
			updates: [{ kind: "set" }],
			origin: { appId: "app1", boardId: "b1", eventId: "e1" },
		});
		expect(
			readReplyWidget({
				origin: { app_id: "app1" },
				widget: { ...frame.widget, updates: undefined },
			}),
		).toEqual({
			instance_id: "w1",
			widget_id: "chart",
			surface_id: "s1",
			component: { widgetInstance: { widgetId: "chart" } },
			origin: { appId: "app1" },
		});
	});

	test("junk is rejected", () => {
		expect(readReplyWidget(undefined)).toBeNull();
		expect(readReplyWidget("widget")).toBeNull();
		expect(readReplyWidget({ widget: frame.widget })).toBeNull();
		expect(readReplyWidget({ ...frame, origin: { app_id: "" } })).toBeNull();
		expect(
			readReplyWidget({
				...frame,
				widget: { ...frame.widget, instance_id: "" },
			}),
		).toBeNull();
		expect(
			readReplyWidget({ ...frame, widget: { ...frame.widget, component: [] } }),
		).toBeNull();
		expect(
			readReplyWidget({ ...frame, widget: { ...frame.widget, updates: {} } }),
		).toBeNull();
	});

	test("the parser emits a frame split across chunks and the accumulator ignores it", () => {
		const parser = createCopilotStreamParser();
		const acc = createStreamAccumulator();
		const raw = `Here it is.<reply_widget>${JSON.stringify(frame)}</reply_widget> Done.`;
		const events = [
			...parser.push(raw.slice(0, 16)),
			...parser.push(raw.slice(16, 60)),
			...parser.push(raw.slice(60)),
			...parser.flush(),
		];
		for (const event of events) applyStreamEvent(acc, event);
		const widgets = events.filter((event) => event.type === "reply_widget");
		expect(widgets).toHaveLength(1);
		expect(readReplyWidget(widgets[0].data)?.instance_id).toBe("w1");
		expect(acc.content).toBe("Here it is. Done.");
		expect(acc.steps.size).toBe(0);
	});
});

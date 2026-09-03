import { describe, expect, test } from "bun:test";
import {
	type IMessage,
	type IMessageRow,
	decodeMessageRow,
	encodeMessageRow,
} from "./chat-db";
import { finalizePlanSteps } from "./event-processor";

const richMessage = (): IMessage => ({
	id: "m1",
	appId: "app",
	sessionId: "s1",
	files: ["https://example.com/a.png", { url: "u", size: null, page: 2 }],
	inner: {
		role: "assistant",
		content: [
			{ type: "text", text: "hello" },
			{ type: "image_url", image_url: { url: "data:image/png;base64,AAA" } },
		],
	} as IMessage["inner"],
	timestamp: 1_700_000_000_000,
	rev: 4,
	tools: ["search"],
	actions: [],
	explicit_name: "Chat",
	plan_steps: [
		{ id: "p1", title: "Plan", status: "done", startTime: 1, endTime: 2 },
		{
			id: "p2",
			title: "Build",
			status: "progress",
			detail: {
				kind: "build_lane",
				lane: "data",
				segments: [{ id: "seg", title: "tables", applied: true }],
			},
		},
	],
	current_step_id: "p2",
	usage_stats: [
		{
			step_name: "llm",
			stats: {
				usage: { completion_tokens: 1, prompt_tokens: 2, total_tokens: 3 },
				calls: [
					{
						model: "m",
						usage: { completion_tokens: 1, prompt_tokens: 2, total_tokens: 3 },
					},
				],
			},
		},
	],
	widgets: [
		{
			instance_id: "w1",
			widget_id: "wid",
			surface_id: "surf",
			component: {
				type: "widgetInstance",
				props: { nested: [1, 2, { a: 1 }] },
			},
			updates: [{ type: "dataModelUpdate", path: "x", value: 1 }],
			origin: { appId: "other" },
		},
	],
	app_refs: ["other"],
	run_context: { schema: "1", provider: "openai" },
});

describe("encodeMessageRow / decodeMessageRow", () => {
	test("round-trips a deeply nested message through the payload column", () => {
		const message = richMessage();
		const row = encodeMessageRow(message);

		expect(row.id).toBe("m1");
		expect(row.sessionId).toBe("s1");
		expect(row.timestamp).toBe(message.timestamp);
		expect(row.rev).toBe(4);
		expect(typeof row.payload).toBe("string");
		expect(Object.keys(row).sort()).toEqual([
			"id",
			"payload",
			"rev",
			"sessionId",
			"timestamp",
		]);

		const decoded = decodeMessageRow(row);
		expect(decoded).toEqual(message);
		expect(decoded).not.toBe(message);
		expect(decoded.widgets?.[0].updates).toEqual(message.widgets?.[0].updates);
	});

	test("the payload is plain JSON, so a fresh parse of it is the message", () => {
		const row = encodeMessageRow(richMessage());
		expect(JSON.parse(row.payload)).toEqual(richMessage());
	});

	test("a legacy nested row without a payload is returned untouched", () => {
		const legacy = richMessage();
		const decoded = decodeMessageRow(legacy as IMessage | IMessageRow);
		expect(decoded).toBe(legacy);
	});

	test("a row whose payload is not a string is treated as legacy", () => {
		const legacy = { ...richMessage(), payload: { unexpected: true } };
		expect(decodeMessageRow(legacy as unknown as IMessageRow)).toBe(legacy);
	});

	test("rev stays optional on rows for messages that were never revised", () => {
		const message = richMessage();
		message.rev = undefined;
		const row = encodeMessageRow(message);
		expect(row.rev).toBeUndefined();
		expect(decodeMessageRow(row).rev).toBeUndefined();
	});
});

describe("finalizePlanSteps on a promoted draft", () => {
	test("closes steps still in progress and clears the current step", () => {
		const message = richMessage();
		const before = Date.now();
		finalizePlanSteps(message);

		expect(message.current_step_id).toBeUndefined();
		const [done, interrupted] = message.plan_steps ?? [];
		expect(done).toEqual({
			id: "p1",
			title: "Plan",
			status: "done",
			startTime: 1,
			endTime: 2,
		});
		expect(interrupted.status).toBe("done");
		expect(interrupted.endTime).toBeGreaterThanOrEqual(before);
		expect(interrupted.detail).toEqual(richMessage().plan_steps?.[1].detail);
	});

	test("leaves planned and failed steps alone", () => {
		const message = richMessage();
		message.plan_steps = [
			{ id: "a", title: "a", status: "planned" },
			{ id: "b", title: "b", status: "failed", endTime: 9 },
		];
		finalizePlanSteps(message);
		expect(message.plan_steps.map((step) => step.status)).toEqual([
			"planned",
			"failed",
		]);
		expect(message.plan_steps[1].endTime).toBe(9);
	});

	test("tolerates a message without plan steps", () => {
		const message = richMessage();
		message.plan_steps = undefined;
		message.current_step_id = "stale";
		finalizePlanSteps(message);
		expect(message.current_step_id).toBeUndefined();
	});
});

/**
 * Tests for the chat streaming reducer, exercising the wire shapes the Rust
 * chat nodes actually emit (Push Response → "chat_stream", Push Widget →
 * "chat_stream_partial", element nodes → "a2ui").
 */
import { describe, expect, test } from "bun:test";
import { Response } from "../../../lib/llm/response";
import type { IMessage } from "./chat-db";
import { processChatEvents } from "./event-processor";
import { joinContentText } from "./inline-segments";

const baseMessage = (): IMessage => ({
	id: "m1",
	appId: "app",
	sessionId: "s1",
	files: [],
	inner: { role: "assistant", content: "" } as IMessage["inner"],
	timestamp: 0,
	tools: [],
	actions: [],
});

const baseState = (responseMessage: IMessage) => ({
	intermediateResponse: Response.default(),
	responseMessage,
	attachments: new Map(),
	tmpLocalState: null,
	tmpGlobalState: null,
	done: false,
	appId: "app",
	eventId: "event",
	sessionId: "s1",
});

/** ChatResponse as push_response.rs serializes it (snake_case serde). */
const chatStreamEvent = (content: string) => ({
	event_type: "chat_stream",
	payload: {
		response: {
			choices: [
				{
					finish_reason: "artificial",
					index: 0,
					logprobs: null,
					message: { role: "assistant", content },
				},
			],
			usage: { completion_tokens: 0, prompt_tokens: 0, total_tokens: 0 },
		},
		local_session: {},
		global_session: {},
		actions: [],
		attachments: [],
		widgets: [],
		model_id: null,
	},
});

const mixedResponseEvent = (eventType: "chat_stream" | "chat_out") => ({
	event_type: eventType,
	payload: {
		response: {
			choices: [
				{
					finish_reason: "stop",
					index: 0,
					message: {
						role: "assistant",
						content: "caption",
						content_parts: [
							{
								type: "image_url",
								image_url: {
									url: "https://example.com/generated",
									media_type: "image/png",
								},
							},
						],
					},
				},
			],
			usage: { completion_tokens: 0, prompt_tokens: 0, total_tokens: 0 },
		},
		attachments: [],
		widgets: [],
	},
});

const widgetPushEvent = (instanceId: string, updates: unknown[] = []) => ({
	event_type: "chat_stream_partial",
	payload: {
		chunk: null,
		actions: [],
		attachments: [],
		plan: null,
		widgets: [
			{
				instance_id: instanceId,
				widget_id: "w1",
				surface_id: instanceId,
				component: {
					type: "widgetInstance",
					instanceId,
					widgetId: "w1",
					inlineWidgetDef: {
						name: "Map",
						rootComponentId: "root",
						components: [{ id: "map-1", component: { type: "geoMap" } }],
					},
				},
				updates,
			},
		],
	},
});

const upsertEvent = (elementId: string) => ({
	event_type: "a2ui",
	payload: {
		type: "upsertElement",
		element_id: elementId,
		value: { type: "setGeoMapViewport", viewport: { latitude: 1 } },
	},
});

describe("processChatEvents", () => {
	test("chat_stream from Push Response sets assistant content", () => {
		const result = processChatEvents(
			[chatStreamEvent("Marker added ✓")],
			baseState(baseMessage()),
		);

		expect(result.shouldUpdate).toBe(true);
		expect(result.responseMessage.inner.content).toBe("Marker added ✓");
	});

	test("mixed responses keep legacy text beside media-only parts", () => {
		for (const eventType of ["chat_stream", "chat_out"] as const) {
			const result = processChatEvents(
				[mixedResponseEvent(eventType)],
				baseState(baseMessage()),
			);
			const content = result.responseMessage.inner.content;
			expect(Array.isArray(content)).toBe(true);
			if (!Array.isArray(content)) throw new Error("expected content parts");
			expect(content).toHaveLength(2);
			expect(content[0]?.text).toBe("caption");
			expect(content[1]?.image_url?.url).toBe("https://example.com/generated");
		}
	});

	test("text arriving after streamed media remains visible and ordered", () => {
		const result = processChatEvents(
			[
				{
					event_type: "chat_stream_partial",
					payload: {
						chunk: {
							id: "chunk-1",
							choices: [
								{
									index: 0,
									delta: {
										role: "assistant",
										content_parts: [
											{
												type: "image_url",
												image_url: {
													url: "https://example.com/generated.png",
												},
											},
										],
									},
								},
							],
						},
					},
				},
				{
					event_type: "chat_stream_partial",
					payload: {
						chunk: {
							id: "chunk-2",
							choices: [
								{
									index: 0,
									delta: { content: "caption" },
								},
							],
						},
					},
				},
			],
			baseState(baseMessage()),
		);

		const content = result.responseMessage.inner.content;
		if (!Array.isArray(content)) throw new Error("expected content parts");
		expect(content[0]?.image_url?.url).toBe(
			"https://example.com/generated.png",
		);
		expect(content[1]?.text).toBe("caption");
	});

	test("chat_stream_partial widgets attach to the message", () => {
		const result = processChatEvents(
			[widgetPushEvent("inst-1")],
			baseState(baseMessage()),
		);

		expect(result.shouldUpdate).toBe(true);
		expect(result.responseMessage.widgets).toHaveLength(1);
		expect(result.responseMessage.widgets?.[0]?.instance_id).toBe("inst-1");
	});

	test("post-push a2ui updates append to the matching widget", () => {
		const result = processChatEvents(
			[widgetPushEvent("inst-1"), upsertEvent("inst-1/map-1")],
			baseState(baseMessage()),
		);

		expect(result.responseMessage.widgets?.[0]?.updates).toHaveLength(1);
	});

	test("instance prefixes isolate widgets that share the same child id", () => {
		const widgetA = widgetPushEvent("inst-a").payload.widgets[0];
		const widgetB = widgetPushEvent("inst-b").payload.widgets[0];
		const pushBoth = {
			...widgetPushEvent("inst-a"),
			payload: {
				...widgetPushEvent("inst-a").payload,
				widgets: [widgetA, widgetB],
			},
		};
		const result = processChatEvents(
			[pushBoth, upsertEvent("inst-b/map-1")],
			baseState(baseMessage()),
		);

		expect(result.responseMessage.widgets?.[0]?.updates).toHaveLength(0);
		expect(result.responseMessage.widgets?.[1]?.updates).toHaveLength(1);
	});

	test("chat_out re-send never regresses live-appended updates", () => {
		const message = baseMessage();
		processChatEvents(
			[widgetPushEvent("inst-1"), upsertEvent("inst-1/map-1")],
			baseState(message),
		);

		// chat_out replays the widget as snapshotted at push time (no updates)
		const result = processChatEvents(
			[
				{
					event_type: "chat_out",
					payload: {
						response: { choices: [] },
						widgets: widgetPushEvent("inst-1").payload.widgets,
					},
				},
			],
			baseState(message),
		);

		expect(result.responseMessage.widgets?.[0]?.updates).toHaveLength(1);
	});
});

/** ChatStreamingResponse as push_chunk.rs emits it: one delta per chunk, no plan. */
const chunkEvent = (delta: Record<string, unknown>) => ({
	event_type: "chat_stream_partial",
	payload: {
		chunk: {
			id: "chunk",
			choices: [{ index: 0, delta: { role: "assistant", ...delta } }],
		},
		actions: [],
		attachments: [],
		plan: null,
		widgets: [],
	},
});

/** ChatStreamingResponse as push_step.rs / push_reasoning.rs emit it: the whole plan, every time. */
const planEvent = (
	plan: [number, string][],
	currentStep: number,
	currentMessage = "",
) => ({
	event_type: "chat_stream_partial",
	payload: {
		chunk: null,
		actions: [],
		attachments: [],
		plan: { plan, current_step: currentStep, current_message: currentMessage },
		widgets: [],
	},
});

const chatOutEvent = (content: string) => ({
	...chatStreamEvent(content),
	event_type: "chat_out",
});

describe("processChatEvents step anchors", () => {
	const stepsOf = (message: IMessage) => message.plan_steps ?? [];

	test("Push Step anchors at the text streamed so far; later snapshots keep it", () => {
		const result = processChatEvents(
			[
				chunkEvent({ content: "Looking that up. " }),
				planEvent([[1, "Search: web"]], 1),
				chunkEvent({ content: "Found it." }),
				planEvent([[1, "Search: web"]], 1, "query sent"),
				planEvent(
					[
						[1, "Search: web"],
						[2, "Summarize: results"],
					],
					2,
				),
			],
			baseState(baseMessage()),
		);

		const steps = stepsOf(result.responseMessage);
		expect(steps.map((step) => step.content_offset)).toEqual([
			"Looking that up. ".length,
			"Looking that up. Found it.".length,
		]);
		expect(steps.map((step) => step.status)).toEqual(["done", "progress"]);
		expect(result.responseMessage.current_step_id).toBe("step-2");
	});

	test("Push Response detaches every anchor; later steps anchor against the replaced text", () => {
		const result = processChatEvents(
			[
				chunkEvent({ content: "draft" }),
				planEvent([[1, "Search: web"]], 1),
				chatStreamEvent("The final answer"),
				planEvent(
					[
						[1, "Search: web"],
						[2, "Verify: sources"],
					],
					2,
				),
			],
			baseState(baseMessage()),
		);

		const [first, second] = stepsOf(result.responseMessage);
		expect(first?.content_offset).toBeUndefined();
		expect(second?.content_offset).toBe("The final answer".length);
	});

	test("the final flush keeps anchors when it extends the streamed text, drops them otherwise", () => {
		const streamed = () => [
			chunkEvent({ content: "Hello" }),
			planEvent([[1, "Search: web"]], 1),
		];

		const kept = processChatEvents(
			[...streamed(), chatOutEvent("Hello world")],
			baseState(baseMessage()),
		);
		expect(stepsOf(kept.responseMessage)[0]?.content_offset).toBe(5);
		expect(stepsOf(kept.responseMessage)[0]?.status).toBe("done");

		const rewritten = processChatEvents(
			[...streamed(), chatOutEvent("Something else")],
			baseState(baseMessage()),
		);
		expect(
			stepsOf(rewritten.responseMessage)[0]?.content_offset,
		).toBeUndefined();
	});

	test("anchors index the joined text when content arrives as parts", () => {
		const result = processChatEvents(
			[
				chunkEvent({
					content_parts: [
						{
							type: "image_url",
							image_url: { url: "https://example.com/a.png" },
						},
					],
				}),
				chunkEvent({ content: "caption" }),
				planEvent([[1, "Search: web"]], 1),
			],
			baseState(baseMessage()),
		);

		const content = result.responseMessage.inner.content;
		expect(Array.isArray(content)).toBe(true);
		expect(joinContentText(content)).toBe("caption");
		expect(stepsOf(result.responseMessage)[0]?.content_offset).toBe(
			"caption".length,
		);
	});

	test("the synthesized Thinking step anchors where the reasoning started", () => {
		const result = processChatEvents(
			[
				chunkEvent({ content: "Intro " }),
				chunkEvent({ reasoning: "hmm" }),
				chunkEvent({ content: "more" }),
			],
			baseState(baseMessage()),
		);

		expect(stepsOf(result.responseMessage)[0]).toMatchObject({
			id: "step-0",
			title: "Thinking",
			content_offset: "Intro ".length,
		});
	});
});

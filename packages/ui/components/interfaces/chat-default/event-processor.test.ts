/**
 * Tests for the chat streaming reducer, exercising the wire shapes the Rust
 * chat nodes actually emit (Push Response → "chat_stream", Push Widget →
 * "chat_stream_partial", element nodes → "a2ui").
 */
import { describe, expect, test } from "bun:test";
import { Response } from "../../../lib/llm/response";
import type { IChatMainWidget, IChatWidget, IMessage } from "./chat-db";
import { mergeChatWidgets, processChatEvents } from "./event-processor";
import { joinContentText } from "./inline-segments";
import { refreshMainChatWidget, resolveMainChatWidget } from "./main-widget-state";

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

const a2uiEvent = (payload: Record<string, unknown>) => ({
	event_type: "a2ui",
	payload,
});

describe("main chat widget", () => {
	const main = (instanceId: string): IChatMainWidget => ({
		widget: widgetPushEvent(instanceId).payload.widgets[0],
		title: "Browser",
		display: "open",
	});
	const command = (
		value: IChatMainWidget | null,
		event_type = "chat_stream_partial",
	) => ({ event_type, payload: { main_widget: value } });

	test("sets and replaces one main widget without adding a message widget", () => {
		const message = baseMessage();
		const result = processChatEvents(
			[command(main("first")), command(main("second"))],
			baseState(message),
		);
		expect(result.shouldUpdate).toBe(true);
		expect(message.main_widget?.widget?.instance_id).toBe("second");
		expect(message.widgets).toBeUndefined();
	});

	test("keeps streamed element updates when the final response repeats the snapshot", () => {
		const message = baseMessage();
		const initial = main("browser");
		processChatEvents(
			[
				command(initial),
				upsertEvent("browser/map-1"),
				command(initial, "chat_out"),
			],
			baseState(message),
		);
		expect(message.main_widget?.widget?.updates).toEqual([
			upsertEvent("browser/map-1").payload,
		]);
		expect(initial.widget?.updates).toEqual([]);
	});

	test("a clear command survives later responses that omit the main widget", () => {
		const message = baseMessage();
		processChatEvents(
			[
				command(main("browser")),
				command({ widget: null }),
				command(null, "chat_out"),
			],
			baseState(message),
		);
		expect(message.main_widget).toMatchObject({ widget: null });
	});

	test("handles a main widget received only in a complete response", () => {
		for (const eventType of ["chat_stream", "chat_out"]) {
			const message = baseMessage();
			processChatEvents(
				[command(main("browser"), eventType)],
				baseState(message),
			);
			expect(message.main_widget).toMatchObject(main("browser"));
		}
	});

	test("a later turn updates inherited state without mutating stored history", () => {
		const previous = { ...baseMessage(), main_widget: main("browser") };
		const message = {
			...baseMessage(),
			id: "m2",
			main_widget: previous.main_widget,
		};
		processChatEvents(
			[upsertEvent("browser/map-1"), upsertEvent("unrelated/map-1")],
			baseState(message),
		);
		expect(message.main_widget.widget?.updates).toHaveLength(1);
		expect(previous.main_widget.widget?.updates).toEqual([]);
		expect(message.main_widget).not.toBe(previous.main_widget);
	});

	test("a new message keeps live main widget updates over the backend snapshot", () => {
		const initial = main("browser");
		const response = chatStreamEvent("Second answer");
		const result = processChatEvents(
			[
				{
					event_type: "chat_stream_partial",
					payload: { message_id: "first", main_widget: initial },
				},
				upsertEvent("browser/map-1"),
				{
					...response,
					payload: {
						...response.payload,
						message_id: "second",
						main_widget: initial,
					},
				},
			],
			baseState(baseMessage()),
		);
		expect(result.responseMessages).toHaveLength(2);
		expect(result.responseMessage.main_widget?.widget?.updates).toEqual([
			upsertEvent("browser/map-1").payload,
		]);
	});

	test("an explicit set can replace the component of the same instance", () => {
		const message = baseMessage();
		const initial = main("browser");
		const updated = main("browser");
		if (updated.widget)
			updated.widget.component = { type: "text", text: "New" };
		processChatEvents(
			[command(initial), upsertEvent("browser/map-1"), command(updated)],
			baseState(message),
		);
		expect(message.main_widget).toMatchObject(updated);
	});

	test("does not revive a replaced widget when its old elements update", () => {
		const message = baseMessage();
		processChatEvents(
			[
				command(main("browser")),
				command(main("library")),
				upsertEvent("browser/map-1"),
			],
			baseState(message),
		);
		expect(message.main_widget?.widget?.instance_id).toBe("library");
		expect(message.main_widget?.widget?.updates).toEqual([]);
	});

	test("revises repeated explicit commands but retains the revision on final replay", () => {
		const message = baseMessage();
		const first = { ...main("browser"), command_id: "first" };
		const second = { ...main("browser"), command_id: "second" };
		processChatEvents([command(first)], baseState(message));
		const firstRevision = message.main_widget?.updated_at ?? 0;
		processChatEvents([command(second)], baseState(message));
		const secondRevision = message.main_widget?.updated_at ?? 0;
		expect(secondRevision).toBeGreaterThan(firstRevision);
		expect(message.main_widget?.command_id).toBe("second");
		processChatEvents([command(second, "chat_out")], baseState(message));
		expect(message.main_widget?.updated_at).toBe(secondRevision);
	});

	test("replaying a clear does not make it newer than subsequent pane activity", () => {
		const message = baseMessage();
		const cleared = { widget: null, command_id: "clear" };
		processChatEvents([command(cleared)], baseState(message));
		const revision = message.main_widget?.updated_at;
		processChatEvents([command(cleared, "chat_out")], baseState(message));
		expect(message.main_widget?.updated_at).toBe(revision);
	});

	test("main widget updates continue across separate assistant bubbles in one run", () => {
		const message = baseMessage();
		const initial = { ...main("browser"), command_id: "open" };
		const first = command(initial);
		const result = processChatEvents(
			[
				{ ...first, payload: { ...first.payload, message_id: "first" } },
				upsertEvent("browser/map-1"),
				{
					event_type: "chat_stream",
					payload: { message_id: "second" },
				},
				upsertEvent("browser/map-1"),
				command(initial, "chat_out"),
			],
			baseState(message),
		);
		expect(result.responseMessages).toHaveLength(2);
		expect(result.responseMessage.main_widget?.widget?.updates).toHaveLength(2);
		expect(message.main_widget?.widget?.updates).toHaveLength(1);
	});

	test("a clear stays current when another run receives a late update to the old widget", () => {
		const first = baseMessage();
		processChatEvents(
			[command({ ...main("browser"), command_id: "open" })],
			baseState(first),
		);
		const second = { ...baseMessage(), id: "m2" };
		processChatEvents(
			[command({ widget: null, command_id: "clear" })],
			baseState(second),
		);
		const commandRevision = first.main_widget?.command_at;
		processChatEvents([upsertEvent("browser/map-1")], baseState(first));
		expect(first.main_widget?.command_at).toBe(commandRevision);
		expect(first.main_widget?.updated_at ?? 0).toBeGreaterThan(
			second.main_widget?.updated_at ?? 0,
		);
		expect(resolveMainChatWidget([second], "s1", first)).toBe(
			second.main_widget,
		);
	});

	test("a continuing stream retains an action update made from its shared pane", () => {
		const original = baseMessage();
		const initial = { ...main("browser"), command_id: "open" };
		processChatEvents([command(initial)], baseState(original));
		const action = {
			...baseMessage(),
			id: "action",
			main_widget: original.main_widget,
		};
		const actionUpdate = upsertEvent("browser/map-1");
		processChatEvents([actionUpdate], baseState(action));
		refreshMainChatWidget(original, action.main_widget);
		const nextUpdate = a2uiEvent({
			...actionUpdate.payload,
			value: { type: "setGeoMapViewport", viewport: { latitude: 2 } },
		});
		processChatEvents(
			[nextUpdate, command(initial, "chat_out")],
			baseState(original),
		);
		expect(original.main_widget?.widget?.updates).toEqual([
			actionUpdate.payload,
			nextUpdate.payload,
		]);
		expect(action.main_widget?.widget?.updates).toEqual([actionUpdate.payload]);
		expect(original.main_widget?.command_at).toBe(action.main_widget?.command_at);
	});
});

/** Widget B pushed into a container of widget A, then addressed by bare id and bound path. */
const nested = {
	pushChild: a2uiEvent({
		type: "upsertElement",
		element_id: "inst-a/root",
		value: { type: "pushChild", childId: "inst-b" },
	}),
	createB: a2uiEvent({
		type: "upsertElement",
		element_id: "inst-b",
		value: {
			type: "createComponent",
			component: {
				type: "widgetInstance",
				instanceId: "inst-b",
				inlineWidgetDef: {
					name: "Child",
					rootComponentId: "b-text",
					components: [
						{
							id: "b-text",
							component: { type: "text", text: { path: "/b/label" } },
						},
					],
				},
			},
		},
	}),
	setBText: a2uiEvent({
		type: "upsertElement",
		element_id: "b-text",
		value: { type: "setText", text: "hi" },
	}),
	dataUpdate: a2uiEvent({
		type: "dataModelUpdate",
		surface_id: "page",
		contents: [{ key: "/b/label", value: "hello" }],
	}),
	unrelated: a2uiEvent({
		type: "upsertElement",
		element_id: "elsewhere",
		value: { type: "setText", text: "no" },
	}),
};

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

	test("updates follow instances pushed into a widget across batches", () => {
		const message = baseMessage();

		// Each batch sees the widget objects produced by the previous one, so the
		// cached targets must carry the pushed instance forward, not just within
		// one batch.
		processChatEvents([widgetPushEvent("inst-a")], baseState(message));
		processChatEvents([nested.pushChild], baseState(message));
		processChatEvents([nested.createB], baseState(message));
		const result = processChatEvents(
			[nested.setBText, nested.dataUpdate, nested.unrelated],
			baseState(message),
		);

		const updates = result.responseMessage.widgets?.[0]?.updates as Array<
			Record<string, unknown>
		>;
		expect(updates.map((update) => update.type)).toEqual([
			"upsertElement",
			"upsertElement",
			"upsertElement",
			"dataModelUpdate",
		]);
		expect(updates[2]?.element_id).toBe("b-text");
		expect(updates[3]?.surfaceId).toBe("inst-a");
	});

	test("chat_out re-send never regresses live-appended updates", () => {
		const message = baseMessage();
		processChatEvents(
			[widgetPushEvent("inst-1"), upsertEvent("inst-1/map-1")],
			baseState(message),
		);

		// chat_out replays the widget as snapshotted at push time (no updates)
		const before = message.widgets;
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
		// Nothing new arrived, so the list (and every widget in it) keeps identity.
		expect(result.responseMessage.widgets).toBe(before);
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

describe("processChatEvents widget anchors", () => {
	const offsetsOf = (message: IMessage) =>
		(message.widgets ?? []).map((widget) => widget.content_offset);

	const chatOutWithWidgets = (content: string, instanceIds: string[]) => ({
		...chatOutEvent(content),
		payload: {
			...chatOutEvent(content).payload,
			widgets: instanceIds.flatMap((id) => widgetPushEvent(id).payload.widgets),
		},
	});

	test("Push Widget anchors at the text streamed so far; longer re-sends keep it", () => {
		const result = processChatEvents(
			[
				chunkEvent({ content: "Here is the map: " }),
				widgetPushEvent("inst-1"),
				chunkEvent({ content: "done." }),
				widgetPushEvent("inst-1", [{ type: "upsertElement" }]),
			],
			baseState(baseMessage()),
		);

		expect(offsetsOf(result.responseMessage)).toEqual([
			"Here is the map: ".length,
		]);
		expect(result.responseMessage.widgets?.[0]?.updates).toHaveLength(1);
	});

	test("Push Response detaches widget anchors; later widgets anchor against the replaced text", () => {
		const result = processChatEvents(
			[
				chunkEvent({ content: "draft" }),
				widgetPushEvent("before"),
				chatStreamEvent("The final answer"),
				widgetPushEvent("after"),
			],
			baseState(baseMessage()),
		);

		expect(offsetsOf(result.responseMessage)).toEqual([
			undefined,
			"The final answer".length,
		]);
	});

	test("the final flush keeps widget anchors when it extends the streamed text, drops them otherwise", () => {
		const streamed = () => [
			chunkEvent({ content: "Hello" }),
			widgetPushEvent("inst-1"),
		];

		const kept = processChatEvents(
			[...streamed(), chatOutWithWidgets("Hello world", ["inst-1"])],
			baseState(baseMessage()),
		);
		expect(offsetsOf(kept.responseMessage)).toEqual([5]);

		const rewritten = processChatEvents(
			[...streamed(), chatOutWithWidgets("Something else", ["inst-1"])],
			baseState(baseMessage()),
		);
		expect(offsetsOf(rewritten.responseMessage)).toEqual([undefined]);
	});

	test("widgets the final flush introduces keep the after-text placement", () => {
		const result = processChatEvents(
			[chunkEvent({ content: "Hi" }), chatOutWithWidgets("Hi there", ["late"])],
			baseState(baseMessage()),
		);

		expect(result.responseMessage.widgets).toHaveLength(1);
		expect(offsetsOf(result.responseMessage)).toEqual([undefined]);
	});

	test("live a2ui updates still reach a widget after its anchor was detached", () => {
		const result = processChatEvents(
			[
				widgetPushEvent("inst-1"),
				chatStreamEvent("Replaced"),
				upsertEvent("inst-1/map-1"),
			],
			baseState(baseMessage()),
		);

		expect(offsetsOf(result.responseMessage)).toEqual([undefined]);
		expect(result.responseMessage.widgets?.[0]?.updates).toHaveLength(1);
	});
});

describe("mergeChatWidgets anchors", () => {
	const widget = (
		updates: unknown[],
		content_offset?: number,
	): IChatWidget => ({
		instance_id: "inst-1",
		widget_id: "w1",
		surface_id: "inst-1",
		component: {},
		updates,
		...(content_offset === undefined ? {} : { content_offset }),
	});

	test("first sight takes the anchor; without one the widget is kept as sent", () => {
		expect(mergeChatWidgets([], [widget([])], 7)[0]?.content_offset).toBe(7);
		const sent = widget([]);
		expect(mergeChatWidgets([], [sent])[0]).toBe(sent);
	});

	test("a longer re-send replaces the widget but keeps its first-seen anchor", () => {
		const [merged] = mergeChatWidgets([widget([], 3)], [widget(["u"], 9)], 12);
		expect(merged?.updates).toEqual(["u"]);
		expect(merged?.content_offset).toBe(3);
	});

	test("a detached widget is never re-anchored by a re-send", () => {
		const [merged] = mergeChatWidgets([widget([])], [widget(["u"], 9)], 12);
		expect(merged?.updates).toEqual(["u"]);
		expect(merged && "content_offset" in merged).toBe(false);
	});

	test("an equal re-send keeps the existing array and objects", () => {
		const existing = [widget(["u"], 3)];
		expect(mergeChatWidgets(existing, [widget(["u"])], 12)).toBe(existing);
	});
});

describe("processChatEvents reasoning repair", () => {
	const tokenized = Array.from({ length: 8 }, (_, i) => `tok${i}`).join("\n");

	test("streams reasoning raw and repairs it once on the final flush", () => {
		const events = tokenized
			.split("\n")
			.map((token, i) =>
				chunkEvent({ reasoning: i === 0 ? token : `\n${token}` }),
			);
		const live = processChatEvents(events, baseState(baseMessage()));
		expect(live.responseMessage.plan_steps?.[0]?.reasoning).toBe(tokenized);

		const settled = processChatEvents(
			[...events, chatOutEvent("done")],
			baseState(baseMessage()),
		);
		expect(settled.responseMessage.plan_steps?.[0]?.reasoning).toBe(
			tokenized.replaceAll("\n", " "),
		);
		expect(settled.responseMessage.plan_steps?.[0]?.status).toBe("done");
	});
});

/**
 * Tests for the chat streaming reducer, exercising the wire shapes the Rust
 * chat nodes actually emit (Push Response → "chat_stream", Push Widget →
 * "chat_stream_partial", element nodes → "a2ui").
 */
import { describe, expect, test } from "bun:test";
import { Response } from "../../../lib/llm/response";
import type { IMessage } from "./chat-db";
import { processChatEvents } from "./event-processor";

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

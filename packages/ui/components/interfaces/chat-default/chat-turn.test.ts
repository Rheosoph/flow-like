import { describe, expect, test } from "bun:test";
import type { IIntercomEvent } from "../../../lib/schema/events/intercom-event";
import type { IMessage } from "./chat-db";
import { createChatTurn } from "./chat-turn";

const delta = (index: number, content: string) =>
	({
		event_id: `e${index}`,
		event_type: "chat_stream_partial",
		payload: {
			chunk: {
				id: `chunk-${index}`,
				choices: [
					{
						index: 0,
						delta: index === 0 ? { role: "assistant", content } : { content },
					},
				],
			},
		},
	}) as unknown as IIntercomEvent;

const turn = () =>
	createChatTurn({
		responseMessage: {
			id: "m1",
			appId: "app",
			sessionId: "s1",
			files: [],
			inner: { role: "assistant", content: "" } as IMessage["inner"],
			timestamp: 0,
			tools: [],
			actions: [],
		},
		tmpLocalState: null,
		tmpGlobalState: null,
		appId: "app",
		eventId: "event",
		sessionId: "s1",
	});

describe("chat turn", () => {
	const run = ["Hello", ", ", "world"].map((text, index) => delta(index, text));

	test("a detached turn catches up on what its subscriber missed", () => {
		const detached = turn();
		detached.fold(run.slice(0, 1));
		detached.catchUp(run);
		expect(detached.responseMessage.inner.content).toBe("Hello, world");
		expect(detached.live().processedEvents).toBe(3);
	});

	test("catching up an attached turn changes nothing", () => {
		const attached = turn();
		attached.fold(run);
		attached.catchUp(run);
		expect(attached.responseMessage.inner.content).toBe("Hello, world");
		expect(attached.live().processedEvents).toBe(3);
	});

	test("new message IDs isolate pushes across batches and survive catch-up", () => {
		const scoped = (messageId: string, event: IIntercomEvent) => ({
			...event,
			payload: { ...event.payload, message_id: messageId },
		});
		const partial = (messageId: string, payload: Record<string, unknown>) =>
			({
				event_type: "chat_stream_partial",
				payload: { message_id: messageId, ...payload },
			}) as IIntercomEvent;
		const snapshot = (
			messageId: string,
			text: string,
			eventType = "chat_stream",
		) =>
			({
				event_type: eventType,
				payload: {
					message_id: messageId,
					response: {
						choices: [
							{ index: 0, message: { role: "assistant", content: text } },
						],
					},
				},
			}) as IIntercomEvent;
		const usage = (messageId: string) =>
			({
				event_type: "chat_usage_stat",
				payload: {
					message_id: messageId,
					step_name: "Model",
					stats: {
						usage: { prompt_tokens: 1, completion_tokens: 2, total_tokens: 3 },
					},
				},
			}) as IIntercomEvent;
		const events = [
			scoped("first", delta(0, "First answer")),
			partial("first", {
				attachments: ["https://example.com/first.pdf"],
				plan: {
					plan: [[0, "First step"]],
					current_step: 0,
					current_message: "Reason one",
				},
			}),
			usage("first"),
			snapshot("second", "Second"),
			partial("second", {
				attachments: ["https://example.com/second.pdf"],
				plan: {
					plan: [[0, "Second step"]],
					current_step: 0,
					current_message: "Reason two",
				},
				widgets: [
					{
						instance_id: "second-widget",
						widget_id: "w",
						surface_id: "s",
						component: {},
					},
				],
			}),
			scoped("second", delta(1, " answer")),
			usage("second"),
			snapshot("second", "Second answer", "chat_out"),
		];
		const live = turn();
		live.fold(events.slice(0, 2));
		live.fold(events.slice(2, 3));
		live.catchUp(events);
		const [first, second] = live.responseMessages;
		expect(live.responseMessages).toHaveLength(2);
		expect(first.inner.content).toBe("First answer");
		expect(first.files).toEqual(["https://example.com/first.pdf"]);
		expect(first.plan_steps?.[0]).toMatchObject({
			title: "First step",
			reasoning: "Reason one",
			status: "done",
		});
		expect(first.widgets).toBeUndefined();
		expect(first.usage_stats).toHaveLength(1);
		expect(second.inner.content).toBe("Second answer");
		expect(second.files).toEqual(["https://example.com/second.pdf"]);
		expect(second.plan_steps?.[0]).toMatchObject({
			title: "Second step",
			reasoning: "Reason two",
			status: "done",
		});
		expect(second.widgets?.[0].instance_id).toBe("second-widget");
		expect(second.usage_stats).toHaveLength(1);
		expect(live.responseMessage).toBe(second);
		expect(live.live().responseMessages).toEqual([first, second]);

		const replay = turn();
		replay.fold(events);
		expect(replay.responseMessages.map(({ id }) => id)).toEqual(
			live.responseMessages.map(({ id }) => id),
		);
		expect(replay.responseMessages.map(({ timestamp }) => timestamp)).toEqual([
			0, 1,
		]);
	});

	test("the first identified response uses the waiting bubble and repeated pushes replace it", () => {
		const current = turn();
		for (const text of ["First", "Replacement"]) {
			current.fold([
				{
					event_type: "chat_stream",
					payload: {
						message_id: "same",
						response: {
							choices: [
								{ index: 0, message: { role: "assistant", content: text } },
							],
						},
					},
				} as IIntercomEvent,
			]);
		}
		expect(current.responseMessages).toHaveLength(1);
		expect(current.responseMessage.id).toBe("m1");
		expect(current.responseMessage.inner.content).toBe("Replacement");
	});
});

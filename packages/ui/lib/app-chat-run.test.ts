import { expect, test } from "bun:test";
import type { IEventState } from "../state/backend-state/event-state";
import type { IIntercomEvent } from "./schema/events/intercom-event";
import { runAppChatMessage } from "./app-chat-run";

test("app chat tool returns every message and its files across batches", async () => {
	const snapshot = (
		messageId: string,
		content: string,
		eventType = "chat_stream",
	) =>
		({
			event_type: eventType,
			payload: {
				message_id: messageId,
				response: {
					choices: [{ index: 0, message: { role: "assistant", content } }],
				},
				attachments: [`${messageId}.pdf`],
				widgets: [
					{
						instance_id: messageId,
						widget_id: "widget",
						surface_id: messageId,
						component: {},
					},
				],
			},
		}) as IIntercomEvent;
	const eventState = {
		getEvents: async () => [
			{ id: "chat", event_type: "simple_chat", active: true },
		],
		executeEvent: async (...args: Parameters<IEventState["executeEvent"]>) => {
			const emit = args[5];
			emit?.([snapshot("one", "First")]);
			emit?.([snapshot("two", "Second")]);
			emit?.([snapshot("two", "Second", "chat_out")]);
			emit?.([snapshot("three", "Ignored after completion")]);
		},
	} as unknown as IEventState;
	const result = await runAppChatMessage(
		{ eventState },
		{ appId: "app", message: "Hello" },
	);
	expect(result.response).toBe("First\n\nSecond");
	expect(result.attachments).toEqual([{ url: "one.pdf" }, { url: "two.pdf" }]);
	expect(result.pushed_widget_count).toBe(2);
});

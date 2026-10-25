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
});

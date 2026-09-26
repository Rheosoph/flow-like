import { describe, expect, test } from "bun:test";
import { Chat } from "@ai-sdk/react";
import type { FinishReason, UIMessage, UIMessageChunk } from "ai";
import type { IResponse } from "../../lib";
import { type IHistoryMessage, IRole } from "../../lib/schema/llm/history";
import type { IResponseChunk } from "../../lib/schema/llm/response-chunk";
import type { IAIState } from "../../state/backend-state/ai-state";
import { EmptyAIState } from "../../state/backend-state/empty-states/ai-state";
import {
	BackendEditorChatTransport,
	completeEditorChat,
	streamEditorChat,
	toHistoryMessages,
	toUIMessageChunks,
} from "./ai-transport";

describe("editor AI usage attribution", () => {
	test("forwards app scope through streaming and completion transports", async () => {
		const calls: Array<{ appId?: string; mode: string }> = [];
		const response = {} as IResponse;
		const aiState: IAIState = {
			streamChatComplete: async (_messages, appId) => {
				calls.push({ appId, mode: "stream" });
				return new ReadableStream();
			},
			chatComplete: async (_messages, appId) => {
				calls.push({ appId, mode: "complete" });
				return response;
			},
		};
		const messages = [] as IHistoryMessage[];

		await streamEditorChat(aiState, messages, "app-1");
		await completeEditorChat(aiState, messages, "app-1");

		expect(calls).toEqual([
			{ appId: "app-1", mode: "stream" },
			{ appId: "app-1", mode: "complete" },
		]);
	});
});

const event = (
	content?: string | null,
	finishReason?: string,
	extra: Record<string, unknown> = {},
): IResponseChunk => ({
	id: "chunk",
	choices: [
		{
			index: 0,
			delta: { content, ...extra },
			finish_reason: finishReason,
		},
	],
});

const streamOf = (batches: IResponseChunk[][]) =>
	new ReadableStream<IResponseChunk[]>({
		start(controller) {
			for (const batch of batches) controller.enqueue(batch);
			controller.close();
		},
	});

const collect = async (batches: IResponseChunk[][]) => {
	const chunks: UIMessageChunk[] = [];
	const reader = toUIMessageChunks(streamOf(batches), "t").getReader();
	for (;;) {
		const { done, value } = await reader.read();
		if (done) return chunks;
		chunks.push(value);
	}
};

const deltas = (chunks: UIMessageChunk[]) =>
	chunks.flatMap((chunk) => (chunk.type === "text-delta" ? [chunk.delta] : []));

describe("toUIMessageChunks", () => {
	test("keeps only non-empty choice-0 content, in order across batches", async () => {
		const chunks = await collect([
			[event("Hel"), event(""), event(null), event(undefined)],
			[
				event(undefined, undefined, { reasoning: "thinking" }),
				event(undefined, undefined, { refusal: "no" }),
				event(undefined, undefined, {
					tool_calls: [{ id: "1", function: { name: "x" } }],
				}),
				{
					id: "chunk",
					choices: [
						{ index: 0, delta: { content: "lo" } },
						{ index: 1, delta: { content: "IGNORED" } },
					],
				},
			],
			[{ id: "empty", choices: [] }, event(" world")],
		]);

		expect(deltas(chunks).join("")).toBe("Hello world");
		expect(chunks.map((chunk) => chunk.type)).toEqual([
			"text-start",
			"text-delta",
			"text-delta",
			"text-delta",
			"text-end",
			"finish",
		]);
	});

	test("starts the text part lazily and emits a bare finish when no text arrived", async () => {
		expect(await collect([[event(""), event(null)], []])).toEqual([
			{ type: "finish", finishReason: "stop" },
		]);
	});

	test("ends with exactly one text-end then one finish, after content that follows stop", async () => {
		const chunks = await collect([
			[event("a", "stop"), event("b", "stop")],
			[event("c")],
		]);

		expect(deltas(chunks)).toEqual(["a", "b", "c"]);
		expect(chunks.slice(-2)).toEqual([
			{ type: "text-end", id: "t" },
			{ type: "finish", finishReason: "stop" },
		]);
		expect(chunks.filter((chunk) => chunk.type === "finish")).toHaveLength(1);
	});

	test.each<[string, FinishReason]>([
		["stop", "stop"],
		["length", "length"],
		["content_filter", "content-filter"],
		["tool_calls", "tool-calls"],
		["function_call", "tool-calls"],
		["something_new", "other"],
	])("maps finish_reason %s to %s", async (reason, mapped) => {
		const chunks = await collect([[event("x", reason)]]);
		expect(chunks.at(-1)).toEqual({ type: "finish", finishReason: mapped });
	});

	test("keeps the first finish_reason and defaults to stop without one", async () => {
		expect(
			(await collect([[event("x", "length"), event("", "stop")]])).at(-1),
		).toEqual({ type: "finish", finishReason: "length" });
		expect((await collect([[event("x")]])).at(-1)).toEqual({
			type: "finish",
			finishReason: "stop",
		});
	});
});

describe("toHistoryMessages", () => {
	test("sends role plus concatenated text, dropping every other part", () => {
		const messages: UIMessage[] = [
			{
				id: "1",
				role: "user",
				parts: [
					{ type: "text", text: "rendered " },
					{ type: "text", text: "prompt" },
				],
			},
			{
				id: "2",
				role: "assistant",
				parts: [
					{ type: "step-start" },
					{ type: "reasoning", text: "hidden", state: "done" },
					{ type: "text", text: "answer", state: "done" },
				],
			},
			{ id: "3", role: "system", parts: [{ type: "text", text: "sys" }] },
		];

		expect(toHistoryMessages(messages)).toEqual([
			{ role: IRole.User, content: "rendered prompt" },
			{ role: IRole.Assistant, content: "answer" },
			{ role: IRole.System, content: "sys" },
		]);
	});
});

type Backend = {
	aiState: IAIState;
	calls: Array<{ messages: IHistoryMessage[]; appId?: string }>;
	push: (batch: IResponseChunk[]) => void;
	close: () => void;
	fail: (error: Error) => void;
	cancelled: () => boolean;
};

/** A backend whose stream the test drives; pushes after cancellation are dropped. */
const controlledBackend = (): Backend => {
	const calls: Backend["calls"] = [];
	let controller: ReadableStreamDefaultController<IResponseChunk[]> | undefined;
	let wasCancelled = false;
	return {
		calls,
		aiState: {
			streamChatComplete: async (messages, appId) => {
				calls.push({ messages, appId });
				return new ReadableStream<IResponseChunk[]>({
					start(streamController) {
						controller = streamController;
					},
					cancel() {
						wasCancelled = true;
					},
				});
			},
			chatComplete: async () => ({}) as IResponse,
		},
		push: (batch) => {
			if (!wasCancelled) controller?.enqueue(batch);
		},
		close: () => {
			if (!wasCancelled) controller?.close();
		},
		fail: (error) => controller?.error(error),
		cancelled: () => wasCancelled,
	};
};

const scriptedBackend = (answers: IResponseChunk[][][]): Backend => {
	const backend = controlledBackend();
	const open = backend.aiState.streamChatComplete;
	let turn = 0;
	backend.aiState = {
		...backend.aiState,
		streamChatComplete: async (messages, appId) => {
			const stream = await open(messages, appId);
			for (const batch of answers[Math.min(turn, answers.length - 1)])
				backend.push(batch);
			turn += 1;
			backend.close();
			return stream;
		},
	};
	return backend;
};

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

const makeChat = (aiState: IAIState, appId?: string) => {
	const statuses: string[] = [];
	const chat = new Chat<UIMessage>({
		id: "editor",
		transport: new BackendEditorChatTransport<UIMessage>({ aiState, appId }),
	});
	chat["~registerStatusCallback"](() => statuses.push(chat.status));
	return { chat, statuses };
};

const answerOf = (chat: Chat<UIMessage>) =>
	chat.messages
		.findLast((message) => message.role === "assistant")
		?.parts.filter((part) => part.type === "text");

describe("BackendEditorChatTransport with the @ai-sdk/react Chat", () => {
	test("streams one done text part, forwards appId and walks submitted → streaming → ready", async () => {
		const backend = scriptedBackend([
			[
				[event("Hello"), event(" ")],
				[event(null), event("world", "stop")],
			],
		]);
		const { chat, statuses } = makeChat(backend.aiState, "app-1");

		await chat.sendMessage({ text: "prompt" });

		expect(backend.calls).toEqual([
			{ appId: "app-1", messages: [{ role: IRole.User, content: "prompt" }] },
		]);
		expect(chat.messages.map((message) => message.role)).toEqual([
			"user",
			"assistant",
		]);
		expect(answerOf(chat)).toEqual([
			{ type: "text", text: "Hello world", state: "done" },
		]);
		expect(statuses).toEqual(["submitted", "streaming", "ready"]);
		expect(chat.error).toBeUndefined();
	});

	test("stays submitted until the first non-empty delta", async () => {
		const backend = controlledBackend();
		const { chat } = makeChat(backend.aiState);
		const done = chat.sendMessage({ text: "p" });
		await tick();

		backend.push([event(""), event(null)]);
		await tick();
		expect(chat.status).toBe("submitted");
		expect(chat.messages).toHaveLength(1);

		backend.push([event("a")]);
		await tick();
		expect(chat.status).toBe("streaming");
		expect(answerOf(chat)?.[0]).toMatchObject({
			text: "a",
			state: "streaming",
		});

		backend.close();
		await done;
		expect(chat.status).toBe("ready");
	});

	test.each([
		["no finish_reason", undefined],
		["finish_reason length", "length"],
	])("ends ready without an error for %s", async (_label, reason) => {
		const backend = scriptedBackend([[[event("x", reason)]]]);
		const { chat } = makeChat(backend.aiState);

		await chat.sendMessage({ text: "p" });

		expect(chat.status).toBe("ready");
		expect(chat.error).toBeUndefined();
		expect(answerOf(chat)).toEqual([
			{ type: "text", text: "x", state: "done" },
		]);
	});

	test("a stream without text still settles, with no assistant message", async () => {
		const backend = scriptedBackend([[[event("", "stop")]]]);
		const { chat } = makeChat(backend.aiState);

		await chat.sendMessage({ text: "p" });

		expect(chat.status).toBe("ready");
		expect(chat.messages.map((message) => message.role)).toEqual(["user"]);
	});

	test("stop cancels the backend stream, keeps the partial answer and ignores later output", async () => {
		const backend = controlledBackend();
		const { chat } = makeChat(backend.aiState);
		const done = chat.sendMessage({ text: "p" });
		await tick();
		backend.push([event("partial")]);
		await tick();
		expect(chat.status).toBe("streaming");

		await chat.stop();
		await done;
		backend.push([event(" late")]);
		await tick();

		expect(chat.status).toBe("ready");
		expect(backend.cancelled()).toBe(true);
		expect(answerOf(chat)?.map((part) => part.text)).toEqual(["partial"]);
	});

	test("stop before the backend answers settles at once and cancels the stream that arrives later", async () => {
		let deliver: (stream: ReadableStream<IResponseChunk[]>) => void = () => {};
		let cancelled = false;
		const aiState: IAIState = {
			streamChatComplete: () =>
				new Promise((resolve) => {
					deliver = resolve;
				}),
			chatComplete: async () => ({}) as IResponse,
		};
		const { chat, statuses } = makeChat(aiState);
		const done = chat.sendMessage({ text: "p" });
		await tick();
		expect(chat.status).toBe("submitted");

		await chat.stop();
		await done;
		expect(chat.status).toBe("ready");
		expect(statuses).toEqual(["submitted", "ready"]);

		deliver(
			new ReadableStream({
				cancel() {
					cancelled = true;
				},
			}),
		);
		await tick();
		expect(cancelled).toBe(true);
		expect(chat.messages.map((message) => message.role)).toEqual(["user"]);
	});

	test("the prerender placeholder backend surfaces as a chat error and keeps the user message", async () => {
		const { chat } = makeChat(new EmptyAIState());

		await chat.sendMessage({ text: "p" });

		expect(chat.status).toBe("error");
		expect(chat.error?.message).toBe("Method not implemented.");
		expect(chat.messages.map((message) => message.role)).toEqual(["user"]);
	});

	test("backend rejection and a mid-stream failure both end in error", async () => {
		const rejecting: IAIState = {
			streamChatComplete: async () => {
				throw new Error("AI stream failed: 401");
			},
			chatComplete: async () => ({}) as IResponse,
		};
		const first = makeChat(rejecting).chat;
		await first.sendMessage({ text: "p" });
		expect(first.status).toBe("error");
		expect(first.error?.message).toBe("AI stream failed: 401");
		expect(first.messages.map((message) => message.role)).toEqual(["user"]);

		const backend = controlledBackend();
		const second = makeChat(backend.aiState).chat;
		const done = second.sendMessage({ text: "p" });
		await tick();
		backend.push([event("a")]);
		await tick();
		backend.fail(new Error("socket closed"));
		await done;
		expect(second.status).toBe("error");
		expect(second.error?.message).toBe("socket closed");
	});

	test("request bodies never reach the backend, so no system message is injected", async () => {
		const backend = scriptedBackend([[[event("ok", "stop")]]]);
		const { chat } = makeChat(backend.aiState);

		await chat.sendMessage(
			{ text: "p" },
			{ body: { ctx: { children: [] }, system: "You simplify language." } },
		);

		expect(backend.calls[0].messages).toEqual([
			{ role: IRole.User, content: "p" },
		]);
	});

	test("follow-ups send the whole session; regenerate resends it without the last answer", async () => {
		const backend = scriptedBackend([
			[[event("one", "stop")]],
			[[event("two", "stop")]],
			[[event("three", "stop")]],
		]);
		const { chat } = makeChat(backend.aiState);

		await chat.sendMessage({ text: "first" });
		await chat.sendMessage({ text: "second" });
		expect(backend.calls[1].messages).toEqual([
			{ role: IRole.User, content: "first" },
			{ role: IRole.Assistant, content: "one" },
			{ role: IRole.User, content: "second" },
		]);

		await chat.regenerate();
		expect(backend.calls[2].messages).toEqual(backend.calls[1].messages);
		expect(
			chat.messages.map((message) => [
				message.role,
				message.parts.find((part) => part.type === "text")?.text,
			]),
		).toEqual([
			["user", "first"],
			["assistant", "one"],
			["user", "second"],
			["assistant", "three"],
		]);
	});

	test("clearing the messages starts a fresh session", async () => {
		const backend = scriptedBackend([[[event("a", "stop")]]]);
		const { chat } = makeChat(backend.aiState);
		await chat.sendMessage({ text: "old" });

		chat.messages = [];
		await chat.sendMessage({ text: "new" });

		expect(backend.calls[1].messages).toEqual([
			{ role: IRole.User, content: "new" },
		]);
	});
});

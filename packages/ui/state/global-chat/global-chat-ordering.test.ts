import "fake-indexeddb/auto";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { IRole } from "../../lib/schema/llm/history";
import { globalChatDb } from "./global-chat-db";
import {
	registerGlobalChatRunControl,
	unregisterGlobalChatRunControl,
} from "./global-chat-run-control";
import {
	beginGlobalChatTurnSelection,
	useGlobalChatStore,
} from "./global-chat-store";
import {
	clearUnconsumedSteeringLinks,
	makeGlobalChatMessage,
	steerGlobalChatRun,
} from "./global-chat-stream";

describe("global chat message ordering", () => {
	test("a reply always sorts after the question it answers", () => {
		// Created back-to-back, as a real turn does — same millisecond on any
		// quick machine.
		const question = makeGlobalChatMessage(IRole.User, "hi", "s1");
		const reply = makeGlobalChatMessage(IRole.Assistant, "", "s1");

		expect(reply.timestamp).toBeGreaterThan(question.timestamp);
	});

	test("stamps stay strictly increasing across a burst", () => {
		const stamps = Array.from(
			{ length: 50 },
			(_, i) =>
				makeGlobalChatMessage(
					i % 2 === 0 ? IRole.User : IRole.Assistant,
					"",
					"s1",
				).timestamp,
		);

		for (let i = 1; i < stamps.length; i++) {
			expect(stamps[i]).toBeGreaterThan(stamps[i - 1]);
		}
	});

	test("re-sorting a shuffled transcript restores turn order", () => {
		// Dexie returns equal-timestamp rows in random primary-key order; this is
		// what the transcript does with whatever order it receives.
		const turn = [
			makeGlobalChatMessage(IRole.User, "question", "s1"),
			makeGlobalChatMessage(IRole.Assistant, "answer", "s1"),
		];
		const shuffled = [turn[1], turn[0]];

		const sorted = [...shuffled].sort((a, b) => a.timestamp - b.timestamp);

		expect(sorted[0].inner.content).toBe("question");
		expect(sorted[1].inner.content).toBe("answer");
	});
});

describe("steering message ownership", () => {
	const sessionId = "steering-order";
	const runId = "steered-run";
	const otherRunId = "other-run";
	let put: ReturnType<typeof vi.spyOn<typeof globalChatDb.messages, "put">>;

	beforeEach(async () => {
		await globalChatDb.messages.clear();
		put = vi.spyOn(globalChatDb.messages, "put");
		useGlobalChatStore.setState({
			activeConversationId: sessionId,
			messages: [],
			runs: {},
			streamingMessages: [],
			isStreaming: false,
		});
		for (const id of [runId, otherRunId]) {
			const reply = makeGlobalChatMessage(IRole.Assistant, "", sessionId);
			reply.id = id;
			useGlobalChatStore.getState().startRun({
				runId: id,
				conversationId: sessionId,
				selection: beginGlobalChatTurnSelection(id),
				label: id,
				message: reply,
			});
		}
	});

	afterEach(() => {
		put.mockRestore();
		for (const id of [runId, otherRunId]) {
			unregisterGlobalChatRunControl(id);
			useGlobalChatStore.getState().endRun(id);
		}
	});

	test("accepted steering stays linked to its run in the transcript and saved message", async () => {
		const delivered: string[] = [];
		registerGlobalChatRunControl(runId, {
			cancel: async () => {},
			steer: async (content) => {
				delivered.push(content);
			},
		});

		expect(await steerGlobalChatRun(runId, "  Schedule one test run  ")).toBe(
			true,
		);
		const state = useGlobalChatStore.getState();
		expect(delivered).toEqual(["Schedule one test run"]);
		expect(state.messages).toHaveLength(1);
		expect(state.messages[0]).toMatchObject({
			id: state.runs[runId].steers[0].id,
			inner: { role: IRole.User, content: "Schedule one test run" },
			steered_run_id: runId,
		});
		expect(state.runs[runId].message?.timestamp).toBeLessThan(
			state.messages[0].timestamp,
		);
		expect(put).toHaveBeenCalledWith(
			expect.objectContaining({ steered_run_id: runId }),
		);
		expect(state.runs[runId].steers[0].status).toBe("delivered");
		expect(state.runs[otherRunId].steers).toEqual([]);
		expect(await globalChatDb.messages.get(state.messages[0].id)).toMatchObject(
			{
				steered_run_id: runId,
			},
		);
	});

	test("rejected steering leaves no question in the transcript or saved history", async () => {
		registerGlobalChatRunControl(runId, {
			cancel: async () => {},
			steer: async () => {
				throw new Error("Run finished");
			},
		});

		expect(await steerGlobalChatRun(runId, "Schedule one test run")).toBe(
			false,
		);
		const state = useGlobalChatStore.getState();
		expect(state.messages).toEqual([]);
		expect(put).not.toHaveBeenCalled();
		expect(state.runs[runId].steers[0]).toMatchObject({
			status: "failed",
			error: "Run finished",
		});
	});

	test("unconsumed copies lose only their own run link after that run ends", async () => {
		for (const id of [runId, otherRunId]) {
			registerGlobalChatRunControl(id, {
				cancel: async () => {},
				steer: async () => {},
			});
		}
		await steerGlobalChatRun(runId, "Schedule one test run");
		await steerGlobalChatRun(runId, "Schedule one test run");
		await steerGlobalChatRun(otherRunId, "Schedule one test run");
		const state = useGlobalChatStore.getState();
		const [consumed, leftover, concurrent] = state.messages;
		const steers = state.runs[runId].steers;
		state.endRun(runId);

		await clearUnconsumedSteeringLinks(runId, steers, [
			"Schedule one test run",
		]);

		const expected = [
			consumed,
			{ ...leftover, steered_run_id: undefined },
			concurrent,
		];
		expect(useGlobalChatStore.getState().messages).toEqual(expected);
		expect(
			await globalChatDb.messages.bulkGet(
				expected.map((message) => message.id),
			),
		).toEqual(expected);
	});

	test("cleanup updates saved history after switching chats and never recreates deleted messages", async () => {
		registerGlobalChatRunControl(runId, {
			cancel: async () => {},
			steer: async () => {},
		});
		await steerGlobalChatRun(runId, "Keep this instruction");
		await steerGlobalChatRun(runId, "Deleted instruction");
		const state = useGlobalChatStore.getState();
		const [saved, deleted] = state.messages;
		const steers = state.runs[runId].steers;
		await globalChatDb.messages.delete(deleted.id);
		const otherChat = makeGlobalChatMessage(
			IRole.User,
			"Other chat",
			"other-chat",
		);
		state.loadConversation("other-chat", [otherChat]);
		state.endRun(runId);

		await clearUnconsumedSteeringLinks(runId, steers, [
			"Keep this instruction",
			"Deleted instruction",
		]);

		expect(useGlobalChatStore.getState().messages).toEqual([otherChat]);
		expect(await globalChatDb.messages.get(saved.id)).toEqual({
			...saved,
			steered_run_id: undefined,
		});
		expect(await globalChatDb.messages.get(deleted.id)).toBeUndefined();
	});

	test("a late steering acknowledgement cannot restore a cleared run link", async () => {
		let accept = () => {};
		const acknowledgement = new Promise<void>((resolve) => {
			accept = resolve;
		});
		registerGlobalChatRunControl(runId, {
			cancel: async () => {},
			steer: () => acknowledgement,
		});
		const sending = steerGlobalChatRun(runId, "Schedule one test run");
		const state = useGlobalChatStore.getState();
		const steers = state.runs[runId].steers;
		expect(steers[0].status).toBe("pending");
		state.endRun(runId);
		await clearUnconsumedSteeringLinks(runId, steers, [
			"Schedule one test run",
		]);

		accept();
		expect(await sending).toBe(true);
		const [message] = useGlobalChatStore.getState().messages;
		expect(message.id).toBe(steers[0].id);
		expect(message.steered_run_id).toBeUndefined();
		expect(
			(await globalChatDb.messages.get(message.id))?.steered_run_id,
		).toBeUndefined();
	});
});

import { describe, expect, test } from "bun:test";
import { IRole } from "../../lib";
import { makeGlobalChatMessage } from "./global-chat-stream";

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

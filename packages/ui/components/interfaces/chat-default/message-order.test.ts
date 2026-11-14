import { describe, expect, test } from "bun:test";
import { IRole } from "../../../lib/schema/llm/history";
import type { IMessage } from "./chat-db";
import { sortChatMessages } from "./message-order";

function message(id: string, role: IRole, timestamp: number): IMessage {
	return {
		id,
		appId: "app",
		sessionId: "chat",
		inner: { role, content: id },
		files: [],
		timestamp,
	};
}

function reply(id: string, questionId: string, timestamp: number): IMessage {
	return {
		...message(id, IRole.Assistant, timestamp),
		run_context: {
			schema: "flowpilot.run-context/v1",
			user_message_id: questionId,
		},
	};
}

const ids = (messages: readonly IMessage[]) =>
	sortChatMessages(messages).map((entry) => entry.id);

describe("chat transcript order", () => {
	test.each([100, 150])(
		"keeps a reply below its restored question at %s",
		(questionTime) => {
			const question = message("question", IRole.User, questionTime);
			const answer = reply("answer", question.id, 100);
			const transcript = [answer, question];
			expect(ids(transcript)).toEqual(["question", "answer"]);
			expect(transcript).toEqual([answer, question]);
			expect(answer.timestamp).toBe(100);
		},
	);

	test("keeps a steered reply after every accepted instruction", () => {
		const question = message("question", IRole.User, 100);
		const answer = reply("answer", question.id, 101);
		const steer = {
			...message("steer", IRole.User, 200),
			steered_run_id: answer.id,
		};
		const laterSteer = {
			...message("later-steer", IRole.User, 300),
			steered_run_id: answer.id,
		};
		const nextQuestion = message("next-question", IRole.User, 400);
		expect(ids([nextQuestion, laterSteer, answer, question, steer])).toEqual([
			"question",
			"steer",
			"later-steer",
			"answer",
			"next-question",
		]);
	});

	test("keeps concurrent replies with their own questions", () => {
		const first = message("first", IRole.User, 100);
		const second = message("second", IRole.User, 200);
		const firstAnswer = reply("first-answer", first.id, 101);
		const secondAnswer = reply("second-answer", second.id, 201);
		const steer = {
			...message("steer", IRole.User, 300),
			steered_run_id: firstAnswer.id,
		};
		expect(ids([first, second, steer, firstAnswer, secondAnswer])).toEqual([
			"first",
			"second",
			"second-answer",
			"steer",
			"first-answer",
		]);
	});

	test("preserves chronological order and stable ties without a matching question", () => {
		const first = reply("first", "missing", 100);
		const second = message("second", IRole.Assistant, 100);
		const third = message("third", IRole.User, 200);
		expect(ids([third, first, second])).toEqual(["first", "second", "third"]);
	});

	test("ignores links across conversations and links to other answers", () => {
		const foreign = {
			...message("foreign", IRole.User, 300),
			sessionId: "other",
			steered_run_id: "answer",
		};
		const answer = reply("answer", foreign.id, 100);
		const otherAnswer = reply("other-answer", answer.id, 50);
		expect(ids([foreign, answer, otherAnswer])).toEqual([
			"other-answer",
			"answer",
			"foreign",
		]);
	});
});

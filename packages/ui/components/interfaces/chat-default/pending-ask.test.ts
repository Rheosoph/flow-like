import { describe, expect, test } from "bun:test";
import { IRole } from "../../../lib/schema/llm/history";
import type { IMessage } from "./chat-db";
import {
	type ChatSend,
	askRuleColor,
	formatPendingElapsed,
	isAwaitingFirstEvent,
	pendingAskStage,
	resolvePendingAsks,
} from "./pending-ask";

function msg(
	id: string,
	role: IRole,
	timestamp: number,
	extra: Partial<IMessage> = {},
	content: IMessage["inner"]["content"] = "",
): IMessage {
	return {
		id,
		appId: "app",
		sessionId: "s1",
		files: [],
		timestamp,
		inner: { role, content },
		...extra,
	} as IMessage;
}

const ask = (id: string, timestamp: number, content = "Hi") =>
	msg(id, IRole.User, timestamp, {}, content);
const answer = (id: string, timestamp: number, content = "") =>
	msg(id, IRole.Assistant, timestamp, {}, content);
const noHostContent = () => false;

describe("isAwaitingFirstEvent", () => {
	test("an empty answer is awaiting", () => {
		expect(isAwaitingFirstEvent(answer("a", 1), false)).toBe(true);
		expect(isAwaitingFirstEvent(answer("a", 1, "  \n"), false)).toBe(true);
	});

	test("text or inline reasoning ends the wait", () => {
		expect(isAwaitingFirstEvent(answer("a", 1, "Hal"), false)).toBe(false);
		expect(isAwaitingFirstEvent(answer("a", 1, "<think>Hm"), false)).toBe(
			false,
		);
	});

	test("the empty fallback step does not, a real step does", () => {
		const fallback = msg("a", IRole.Assistant, 1, {
			plan_steps: [{ id: "step-0", title: "Thinking" } as never],
		});
		const reasoning = msg("a", IRole.Assistant, 1, {
			plan_steps: [
				{ id: "step-0", title: "Thinking", reasoning: "Weighing it." } as never,
			],
		});
		expect(isAwaitingFirstEvent(fallback, false)).toBe(true);
		expect(isAwaitingFirstEvent(reasoning, false)).toBe(false);
	});

	test("widgets, errors, usage and host content end the wait", () => {
		const cases: Partial<IMessage>[] = [
			{ widgets: [{} as never] },
			{ error: { kind: "unknown" } as never },
			{ usage_stats: [{} as never] },
			{ files: [{ url: "x", type: "image/png" } as never] },
		];
		for (const extra of cases) {
			expect(
				isAwaitingFirstEvent(msg("a", IRole.Assistant, 1, extra), false),
			).toBe(false);
		}
		expect(isAwaitingFirstEvent(answer("a", 1), true)).toBe(false);
	});

	test("a question is never awaiting", () => {
		expect(isAwaitingFirstEvent(ask("q", 1), false)).toBe(false);
	});
});

describe("resolvePendingAsks", () => {
	test("an empty live answer is hidden and its question waits", () => {
		const result = resolvePendingAsks(
			[ask("q1", 10), answer("a1", 11, "done"), ask("q2", 20)],
			[answer("a2", 21)],
			null,
			noHostContent,
		);
		expect(result.hidden).toEqual(new Set(["a2"]));
		expect(result.byAsk.get("q2")).toEqual({ phase: "waiting", since: 20 });
		expect(result.byAsk.has("q1")).toBe(false);
	});

	test("the wait counts from the send, not from when the question was stored", () => {
		const send: ChatSend = { content: "Hi", startedAt: 15, inFlight: false };
		const result = resolvePendingAsks(
			[ask("q", 18)],
			[answer("a", 19)],
			send,
			noHostContent,
		);
		expect(result.byAsk.get("q")).toEqual({ phase: "waiting", since: 15 });
	});

	test("the first visible event ends the wait", () => {
		const result = resolvePendingAsks(
			[ask("q", 10)],
			[answer("a", 11, "Hallo")],
			null,
			noHostContent,
		);
		expect(result.byAsk.size).toBe(0);
		expect(result.hidden.size).toBe(0);
	});

	test("host content placed into the answer ends the wait", () => {
		const result = resolvePendingAsks(
			[ask("q", 10)],
			[answer("a", 11)],
			null,
			(id) => id === "a",
		);
		expect(result.byAsk.size).toBe(0);
		expect(result.hidden.size).toBe(0);
	});

	test("a send in flight without an answer is sending", () => {
		const send: ChatSend = { content: "Hi", startedAt: 5, inFlight: true };
		const result = resolvePendingAsks([ask("q", 6)], [], send, noHostContent);
		expect(result.byAsk.get("q")).toEqual({ phase: "sending", since: 5 });
	});

	test("a finished send or another question is not sending", () => {
		const done: ChatSend = { content: "Hi", startedAt: 5, inFlight: false };
		const other: ChatSend = { content: "Else", startedAt: 5, inFlight: true };
		expect(
			resolvePendingAsks([ask("q", 6)], [], done, noHostContent).byAsk.size,
		).toBe(0);
		expect(
			resolvePendingAsks([ask("q", 6)], [], other, noHostContent).byAsk.size,
		).toBe(0);
	});

	test("an answer without a question before it stays visible", () => {
		const result = resolvePendingAsks(
			[ask("q", 30)],
			[answer("a", 20)],
			null,
			noHostContent,
		);
		expect(result.hidden.size).toBe(0);
		expect(result.byAsk.size).toBe(0);
	});

	test("concurrent turns each wait on their own question", () => {
		const result = resolvePendingAsks(
			[ask("q1", 10), ask("q2", 20)],
			[answer("a1", 11), answer("a2", 21, "Hal")],
			null,
			noHostContent,
		);
		expect(result.byAsk.get("q1")).toEqual({ phase: "waiting", since: 10 });
		expect(result.byAsk.has("q2")).toBe(false);
		expect(result.hidden).toEqual(new Set(["a1"]));
	});
});

describe("stages and elapsed time", () => {
	test("waiting escalates at 8 s and 20 s, sending never does", () => {
		expect(pendingAskStage("waiting", 7_999)).toBe("waiting");
		expect(pendingAskStage("waiting", 8_000)).toBe("stillWaiting");
		expect(pendingAskStage("waiting", 20_000)).toBe("slow");
		expect(pendingAskStage("sending", 30_000)).toBe("sending");
	});

	test("only sending dims the question's rule", () => {
		const rule = "var(--fl-chat-ask-rule, var(--primary))";
		expect(askRuleColor(undefined)).toBe(rule);
		expect(askRuleColor({ phase: "waiting", since: 0 })).toBe(rule);
		expect(askRuleColor({ phase: "sending", since: 0 })).toBe(
			`color-mix(in oklch, ${rule} 45%, transparent)`,
		);
	});

	test("the time appears after 2 s and switches to minutes", () => {
		expect(formatPendingElapsed(1_999)).toBeNull();
		expect(formatPendingElapsed(2_000)).toBe("2s");
		expect(formatPendingElapsed(59_999)).toBe("59s");
		expect(formatPendingElapsed(65_000)).toBe("1m 05s");
	});
});

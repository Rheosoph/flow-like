import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { calibrateEmotionScores } from "./calibration";
import type { EmotionClient } from "./client";
import {
	BotExpressionController,
	type BotExpressionState,
	SENT_REACTION_MS,
} from "./controller";
import type {
	EmotionInput,
	EmotionLoadState,
	ExpressionScores,
} from "./protocol";
import { EmotionScorer, expressionScores } from "./scoring";

class FakeClient implements EmotionClient {
	warmup = vi.fn();
	requests: {
		input: EmotionInput;
		resolve: (scores: ExpressionScores | null) => void;
	}[] = [];
	listener?: (state: EmotionLoadState) => void;
	cancel = vi.fn();
	predict(_owner: object, input: EmotionInput) {
		return new Promise<ExpressionScores | null>((resolve) =>
			this.requests.push({ input, resolve }),
		);
	}
	subscribe(listener: (state: EmotionLoadState) => void) {
		this.listener = listener;
		listener({ status: "ready" });
		return () => {
			this.listener = undefined;
		};
	}
}

beforeEach(() => {
	vi.useFakeTimers();
	vi.setSystemTime(10_000);
});
afterEach(() => {
	vi.clearAllTimers();
	vi.useRealTimers();
});

describe("bot expression transitions", () => {
	it("waits for a draft before using the model", () => {
		const client = new FakeClient();
		const controller = new BotExpressionController(client, () => {});
		controller.update({
			draft: "",
			messages: [{ role: "user", content: "old" }],
		});
		vi.advanceTimersByTime(75_000);
		expect(client.warmup).not.toHaveBeenCalled();
		expect(client.requests).toHaveLength(0);
		controller.update({ draft: "hello", messages: [] });
		vi.advanceTimersByTime(300);
		expect(client.requests.map((request) => request.input.draft)).toEqual([
			"hello",
		]);
		controller.dispose();
	});

	it("lets validated wording correct a provisional smile without flashing it again on appended punctuation", async () => {
		const client = new FakeClient();
		let state: BotExpressionState | undefined;
		const controller = new BotExpressionController(client, (next) => {
			state = next;
		});
		controller.update({ draft: "I am devastated 🙂", messages: [] });
		expect(state?.expression).toBe("happy");
		vi.advanceTimersByTime(300);
		client.requests[0].resolve(expressionScores({ sadness: 0.95 }));
		await Promise.resolve();
		vi.advanceTimersByTime(600);
		expect(state?.expression).toBe("sad");
		controller.update({ draft: "I am devastated 🙂.", messages: [] });
		expect(state?.expression).toBe("sad");
		controller.dispose();
	});

	it("discards a prefix cue when later wording changes its meaning", async () => {
		const client = new FakeClient();
		let state: BotExpressionState | undefined;
		const controller = new BotExpressionController(client, (next) => {
			state = next;
		});
		controller.update({ draft: "😉", messages: [] });
		vi.advanceTimersByTime(300);
		controller.update({ draft: "😉 why?", messages: [] });
		client.requests[0].resolve({ ...expressionScores({}), wink: 1 });
		await Promise.resolve();
		vi.advanceTimersByTime(900);
		expect(state?.expression).toBe("neutral");
		client.requests[1].resolve(expressionScores({ curiosity: 1 }));
		await Promise.resolve();
		expect(state?.expression).toBe("curious");
		controller.dispose();
	});

	it("reacts to an appended wink immediately and does not let older inference erase it", async () => {
		const client = new FakeClient();
		let state: BotExpressionState | undefined;
		const controller = new BotExpressionController(client, (next) => {
			state = next;
		});
		controller.update({ draft: "hello", messages: [] });
		vi.advanceTimersByTime(300);
		controller.update({ draft: "hello 😉", messages: [] });
		expect(state?.expression).toBe("wink");
		client.requests[0].resolve(expressionScores({ neutral: 1 }));
		await Promise.resolve();
		vi.advanceTimersByTime(900);
		expect(state?.expression).toBe("wink");
		controller.update({ draft: "hello", messages: [] });
		expect(state?.expression).toBe("neutral");
		controller.dispose();
	});

	it.each(["before", "together", "after"])(
		"keeps a submitted reaction when history arrives %s the clear, then settles",
		async (order) => {
			const client = new FakeClient();
			let state: BotExpressionState | undefined;
			const controller = new BotExpressionController(client, (next) => {
				state = next;
			});
			const draft = `A long message ${"x".repeat(500)} ❤️`;
			const messages = [{ role: "user" as const, content: draft }];
			controller.update({ draft, messages: [] });
			expect(state?.expression).toBe("love");
			if (order === "before") controller.update({ draft, messages });
			if (order === "after") {
				controller.update({ draft: "", messages: [] });
				expect(state?.expression).toBe("neutral");
				vi.advanceTimersByTime(100);
			}
			controller.update({ draft: "", messages });
			expect(state).toEqual({
				expression: "love",
				status: "ready",
				lingering: true,
			});
			client.listener?.({ status: "loading", progress: 0.5 });
			controller.update({
				draft: "",
				messages: [...messages, { role: "assistant", content: "Thanks!" }],
			});
			expect(state?.expression).toBe("love");
			vi.advanceTimersByTime(SENT_REACTION_MS - 1);
			expect(state?.lingering).toBe(true);
			vi.advanceTimersByTime(1);
			expect(state?.expression).toBe("neutral");
			expect(state?.lingering).toBeUndefined();
			controller.dispose();
			expect(vi.getTimerCount()).toBe(0);
		},
	);

	it("does not mistake an old matching message or a streaming assistant for a new send", () => {
		const client = new FakeClient();
		let state: BotExpressionState | undefined;
		const controller = new BotExpressionController(client, (next) => {
			state = next;
		});
		const messages = [
			{ role: "user" as const, content: "Thanks ❤️" },
			{ role: "assistant" as const, content: "Hello" },
		];
		controller.update({ draft: "Thanks ❤️", messages });
		controller.update({
			draft: "",
			messages: [
				...messages.slice(0, 1),
				{ role: "assistant", content: "Hello again" },
			],
		});
		expect(state?.expression).toBe("neutral");
		expect(state?.lingering).toBeUndefined();
		controller.dispose();
	});

	it("expires the send handoff and cancels lingering when another draft starts", () => {
		const client = new FakeClient();
		let state: BotExpressionState | undefined;
		const controller = new BotExpressionController(client, (next) => {
			state = next;
		});
		controller.update({ draft: "Thanks ❤️", messages: [] });
		controller.update({ draft: "", messages: [] });
		vi.advanceTimersByTime(700);
		const messages = [{ role: "user" as const, content: "Thanks ❤️" }];
		controller.update({ draft: "", messages });
		expect(state?.expression).toBe("neutral");
		controller.update({ draft: "Bye 😉", messages });
		const sent = [...messages, { role: "user" as const, content: "Bye 😉" }];
		controller.update({ draft: "", messages: sent });
		expect(state?.lingering).toBe(true);
		controller.update({ draft: "next", messages: sent });
		expect(state?.expression).toBe("neutral");
		expect(state?.lingering).toBeUndefined();
		controller.dispose();
		expect(vi.getTimerCount()).toBe(0);
	});

	it("shows a calibrated German reaction after typing stops, even after a neutral result", async () => {
		const client = new FakeClient();
		let state: BotExpressionState | undefined;
		const controller = new BotExpressionController(client, (next) => {
			state = next;
		});
		controller.update({ draft: "du kleiner", messages: [] });
		vi.advanceTimersByTime(300);
		client.requests[0].resolve(expressionScores({ neutral: 1 }));
		await Promise.resolve();
		const draft = "du kleiner wichser brings wirklich garnichts hin";
		const input = {
			draft,
			messages: [
				{ role: "assistant" as const, content: "A factual response." },
			],
		};
		controller.update(input);
		vi.advanceTimersByTime(300);
		const scorer = new EmotionScorer(async (text) =>
			calibrateEmotionScores(
				text === draft
					? {
							neutral: 0.5234704614,
							annoyance: 0.1919647604,
							disapproval: 0.2104866207,
						}
					: { neutral: 1 },
			),
		);
		client.requests[1].resolve(await scorer.score(input));
		await Promise.resolve();
		vi.advanceTimersByTime(900);
		expect(state?.expression).toBe("concerned");
		expect(client.requests).toHaveLength(2);
		controller.dispose();
	});

	it("throttles continuous typing without waiting for the user to stop", async () => {
		const client = new FakeClient();
		let state: BotExpressionState | undefined;
		const controller = new BotExpressionController(client, (next) => {
			state = next;
		});
		controller.update({ draft: "a", messages: [] });
		vi.advanceTimersByTime(200);
		controller.update({ draft: "ab", messages: [] });
		vi.advanceTimersByTime(100);
		expect(client.requests.map((request) => request.input.draft)).toEqual([
			"ab",
		]);
		controller.update({ draft: "abc", messages: [] });
		vi.advanceTimersByTime(300);
		controller.update({ draft: "abcd", messages: [] });
		expect(client.requests).toHaveLength(1);
		client.requests[0].resolve(expressionScores({ joy: 1 }));
		await Promise.resolve();
		expect(state?.expression).toBe("happy");
		vi.advanceTimersByTime(1);
		expect(client.requests.map((request) => request.input.draft)).toEqual([
			"ab",
			"abcd",
		]);
		expect(client.cancel).not.toHaveBeenCalled();
		controller.dispose();
	});

	it("reacts locally while the model loads and when it is unavailable", () => {
		const client = new FakeClient();
		let state: BotExpressionState | undefined;
		const controller = new BotExpressionController(client, (next) => {
			state = next;
		});
		client.listener?.({ status: "loading", progress: 0.1 });
		controller.update({ draft: "I love this ❤️", messages: [] });
		expect(state).toEqual({
			expression: "love",
			status: "loading",
			progress: 0.1,
		});
		client.listener?.({ status: "unavailable" });
		controller.update({ draft: "I'm feeling sad", messages: [] });
		expect(state).toEqual({ expression: "sad", status: "unavailable" });
		controller.update({ draft: "", messages: [] });
		expect(state?.expression).toBe("neutral");
		controller.dispose();
	});

	it("ignores stale results and resets immediately when the draft clears", async () => {
		const client = new FakeClient();
		let state: BotExpressionState | undefined;
		const controller = new BotExpressionController(client, (next) => {
			state = next;
		});
		controller.update({ draft: "happy", messages: [] });
		vi.advanceTimersByTime(300);
		controller.update({ draft: "sad", messages: [] });
		client.requests[0].resolve(expressionScores({ joy: 1 }));
		await Promise.resolve();
		expect(state?.expression).toBe("neutral");
		vi.advanceTimersByTime(300);
		client.requests[1].resolve(expressionScores({ sadness: 1 }));
		await Promise.resolve();
		expect(state?.expression).toBe("sad");
		controller.update({ draft: "", messages: [] });
		expect(state?.expression).toBe("neutral");
		vi.advanceTimersByTime(2_000);
		expect(client.requests).toHaveLength(2);
		controller.dispose();
	});

	it("holds a visible expression before switching and cancels the hold on clear", async () => {
		const client = new FakeClient();
		let state: BotExpressionState | undefined;
		const controller = new BotExpressionController(client, (next) => {
			state = next;
		});
		controller.update({ draft: "happy", messages: [] });
		vi.advanceTimersByTime(300);
		client.requests[0].resolve(expressionScores({ joy: 1 }));
		await Promise.resolve();
		controller.update({ draft: "sad", messages: [] });
		vi.advanceTimersByTime(300);
		client.requests[1].resolve(expressionScores({ sadness: 1 }));
		await Promise.resolve();
		expect(state?.expression).toBe("happy");
		vi.advanceTimersByTime(600);
		expect(state?.expression).toBe("sad");
		controller.update({ draft: "happy again", messages: [] });
		vi.advanceTimersByTime(1);
		client.requests[2].resolve(expressionScores({ joy: 1 }));
		await Promise.resolve();
		controller.update({ draft: "", messages: [] });
		vi.advanceTimersByTime(1000);
		expect(state?.expression).toBe("neutral");
		controller.dispose();
	});

	it("does not apply results after disposal and falls back to the current draft on failure", async () => {
		const client = new FakeClient();
		const changes = vi.fn();
		const controller = new BotExpressionController(client, changes);
		controller.update({ draft: "happy", messages: [] });
		vi.advanceTimersByTime(300);
		client.requests[0].resolve(expressionScores({ joy: 1 }));
		await Promise.resolve();
		client.listener?.({ status: "unavailable" });
		expect(changes.mock.lastCall?.[0]).toEqual({
			expression: "happy",
			status: "unavailable",
		});
		controller.update({ draft: "later", messages: [] });
		vi.advanceTimersByTime(300);
		controller.dispose();
		const count = changes.mock.calls.length;
		client.requests[1].resolve(expressionScores({ joy: 1 }));
		await Promise.resolve();
		expect(changes).toHaveBeenCalledTimes(count);
		expect(vi.getTimerCount()).toBe(0);
	});
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { EmotionClient } from "./client";
import { BotExpressionController, type BotExpressionState } from "./controller";
import type {
	EmotionInput,
	EmotionLoadState,
	ExpressionScores,
} from "./protocol";
import { expressionScores } from "./scoring";

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
	it("throttles continuous typing without waiting for the user to stop", async () => {
		const client = new FakeClient();
		const controller = new BotExpressionController(client, () => {});
		controller.update({ draft: "a", messages: [] });
		vi.advanceTimersByTime(200);
		controller.update({ draft: "ab", messages: [] });
		vi.advanceTimersByTime(100);
		expect(client.requests.map((request) => request.input.draft)).toEqual([
			"ab",
		]);
		controller.update({ draft: "abc", messages: [] });
		vi.advanceTimersByTime(300);
		expect(client.requests.map((request) => request.input.draft)).toEqual([
			"ab",
			"abc",
		]);
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

	it("does not apply results after disposal or keep an expression after failure", async () => {
		const client = new FakeClient();
		const changes = vi.fn();
		const controller = new BotExpressionController(client, changes);
		controller.update({ draft: "happy", messages: [] });
		vi.advanceTimersByTime(300);
		client.requests[0].resolve(expressionScores({ joy: 1 }));
		await Promise.resolve();
		client.listener?.({ status: "unavailable" });
		expect(changes.mock.lastCall?.[0]).toEqual({
			expression: "neutral",
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

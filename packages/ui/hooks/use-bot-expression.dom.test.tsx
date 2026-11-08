// @vitest-environment happy-dom

import { StrictMode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
	type EmotionClient,
	getBotEmotionClient,
} from "../lib/bot-emotion/client";
import type { BotExpressionState } from "../lib/bot-emotion/controller";
import type {
	EmotionInput,
	EmotionLoadState,
	ExpressionScores,
} from "../lib/bot-emotion/protocol";
import { expressionScores } from "../lib/bot-emotion/scoring";
import {
	type UseBotExpressionOptions,
	useBotExpression,
} from "./use-bot-expression";

vi.mock("../lib/bot-emotion/client", () => ({ getBotEmotionClient: vi.fn() }));

class Client implements EmotionClient {
	warmup = vi.fn();
	cancel = vi.fn();
	requests: {
		input: EmotionInput;
		resolve: (scores: ExpressionScores | null) => void;
	}[] = [];
	listeners = new Set<(state: EmotionLoadState) => void>();
	predict(_owner: object, input: EmotionInput) {
		return new Promise<ExpressionScores | null>((resolve) =>
			this.requests.push({ input, resolve }),
		);
	}
	subscribe(listener: (state: EmotionLoadState) => void) {
		this.listeners.add(listener);
		listener({ status: "ready" });
		return () => {
			this.listeners.delete(listener);
		};
	}
}

let container: HTMLDivElement;
let root: Root;
let client: Client;

function Probe(props: UseBotExpressionOptions) {
	return <output>{JSON.stringify(useBotExpression(props))}</output>;
}

function state(): BotExpressionState {
	return JSON.parse(container.querySelector("output")?.textContent ?? "{}");
}
async function render(props: Partial<UseBotExpressionOptions> = {}) {
	await act(async () =>
		root.render(
			<StrictMode>
				<Probe conversationId="one" draft="" messages={[]} {...props} />
			</StrictMode>,
		),
	);
}
async function advance(ms: number) {
	await act(async () => vi.advanceTimersByTime(ms));
}
async function resolve(index: number, scores: ExpressionScores) {
	await act(async () => client.requests[index].resolve(scores));
}

beforeEach(() => {
	vi.useFakeTimers();
	vi.setSystemTime(10_000);
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	client = new Client();
	vi.mocked(getBotEmotionClient).mockReturnValue(client);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.clearAllTimers();
	vi.useRealTimers();
	vi.unstubAllGlobals();
});

describe("useBotExpression lifecycle", () => {
	it("exposes a submitted reaction on an empty composer and clears it on conversation change", async () => {
		await render({ draft: "Thank you ❤️" });
		expect(state().expression).toBe("love");
		await render({ messages: [{ role: "user", content: "Thank you ❤️" }] });
		expect(state().expression).toBe("love");
		expect(state().lingering).toBe(true);
		await render({ conversationId: "two" });
		expect(state().expression).toBe("neutral");
		await advance(2500);
		expect(state().expression).toBe("neutral");
	});

	it("keeps an empty mounted composer idle under StrictMode", async () => {
		await render();
		await advance(75_000);
		expect(client.warmup).not.toHaveBeenCalled();
		expect(client.requests).toHaveLength(0);
		expect(client.listeners.size).toBe(1);
	});

	it("resets on conversation change and ignores results from the previous conversation", async () => {
		await render({ draft: "same text" });
		await advance(300);
		await render({ conversationId: "two", draft: "same text" });
		await resolve(0, expressionScores({ joy: 1 }));
		expect(state().expression).toBe("neutral");
		await advance(300);
		expect(client.requests).toHaveLength(2);
		await resolve(1, expressionScores({ sadness: 1 }));
		expect(state().expression).toBe("sad");
		await render({ conversationId: "two", draft: "" });
		expect(state().expression).toBe("neutral");
	});

	it("pauses for composition and resumes neutrally without stale results", async () => {
		await render({ draft: "typing", enabled: false });
		await advance(1000);
		expect(client.requests).toHaveLength(0);
		expect(client.warmup).not.toHaveBeenCalled();
		await render({ draft: "typing", enabled: true });
		await advance(300);
		await resolve(0, expressionScores({ joy: 1 }));
		expect(state().expression).toBe("happy");
		await render({ draft: "next", enabled: false });
		expect(state().expression).toBe("neutral");
		await render({ draft: "next", enabled: true });
		expect(state().expression).toBe("neutral");
		await advance(300);
		expect(client.requests).toHaveLength(2);
	});

	it("uses content equality for history so a parent rerender does not repeat inference", async () => {
		await render({
			draft: "current",
			messages: [{ role: "user", content: "prior" }],
		});
		await advance(300);
		await resolve(0, expressionScores({ curiosity: 1 }));
		await render({
			draft: "current",
			messages: [{ role: "user", content: "prior" }],
		});
		await advance(300);
		expect(client.requests).toHaveLength(1);
	});

	it("transitions from an older worker's face scores to love without losing model readiness", async () => {
		await render({ draft: "good mood" });
		await advance(300);
		await resolve(0, {
			neutral: 0,
			happy: 1,
			concerned: 0,
			surprised: 0,
			curious: 0,
			sad: 0,
		} as ExpressionScores);
		expect(state()).toEqual({ expression: "happy", status: "ready" });
		await render({ draft: "affection" });
		await advance(300);
		await resolve(1, expressionScores({ love: 1 }));
		expect(state().expression).toBe("happy");
		await advance(600);
		expect(state()).toEqual({ expression: "love", status: "ready" });
	});
});

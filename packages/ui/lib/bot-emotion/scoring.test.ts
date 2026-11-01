import { describe, expect, it, vi } from "vitest";
import { BOT_EXPRESSIONS } from "../../components/bots/types";
import {
	type EmotionScores,
	MAX_CONTEXT_CHARS,
	MAX_DRAFT_CHARS,
	boundEmotionInput,
} from "./protocol";
import {
	EmotionScorer,
	chooseExpression,
	expressionScores,
	smoothScores,
} from "./scoring";

describe("bot emotion scoring", () => {
	it("bounds recent text before it reaches the worker", () => {
		const bounded = boundEmotionInput({
			draft: `old ${"x".repeat(2000)} newest`,
			messages: [
				{ role: "user", content: "discarded" },
				{ role: "assistant", content: "discarded too" },
				{ role: "user", content: "oldest" },
				{ role: "assistant", content: "a".repeat(1000) },
				{ role: "user", content: "current" },
				{ role: "assistant", content: "   " },
			],
		});
		expect(bounded.draft).toHaveLength(MAX_DRAFT_CHARS);
		expect(bounded.draft.endsWith("newest")).toBe(true);
		expect(bounded.messages).toHaveLength(4);
		expect(bounded.messages[0].content).toBe("discarded too");
		expect(bounded.messages[2].content).toHaveLength(MAX_CONTEXT_CHARS);
		expect(bounded.messages[3].content).toBe("current");
	});

	it("maps anger to concern and keeps weak or conflicting evidence neutral", () => {
		expect(chooseExpression(expressionScores({ anger: 0.8 }))).toBe(
			"concerned",
		);
		expect(chooseExpression(expressionScores({ sadness: 0.8 }))).toBe("sad");
		expect(chooseExpression(expressionScores({ curiosity: 0.8 }))).toBe(
			"curious",
		);
		expect(chooseExpression(expressionScores({ joy: 0.3 }))).toBe("neutral");
		expect(chooseExpression(expressionScores({ joy: 0.6, anger: 0.58 }))).toBe(
			"neutral",
		);
		expect(chooseExpression(expressionScores({ joy: Number.NaN }))).toBe(
			"neutral",
		);
	});

	it.each(["love", "caring", "desire"])(
		"uses the love face for a distinct %s signal",
		(label) => {
			const scores = expressionScores({ [label]: 0.9 });
			expect(scores.love).toBe(0.9);
			expect(scores.happy).toBe(0);
			expect(chooseExpression(scores)).toBe("love");
		},
	);

	it("returns every face score while reserving bot moods and activity for explicit state", () => {
		const scores = expressionScores({
			joy: 2,
			sadness: -1,
			love: Number.POSITIVE_INFINITY,
			anger: 0.8,
			wink: 1,
			angry: 1,
			focused: 1,
			oops: 1,
		});
		expect(Object.keys(scores)).toEqual([...BOT_EXPRESSIONS]);
		expect(Object.values(scores).every(Number.isFinite)).toBe(true);
		expect(scores).toMatchObject({
			happy: 1,
			sad: 0,
			love: 0,
			concerned: 0.8,
			wink: 0,
			angry: 0,
			focused: 0,
			oops: 0,
		});
	});

	it("fills missing face scores when smoothing results from older workers", () => {
		const legacy = {
			neutral: 0,
			happy: 0.8,
			concerned: 0,
			surprised: 0,
			curious: 0,
			sad: 0,
		};
		const initial = smoothScores(legacy);
		expect(Object.keys(initial)).toEqual([...BOT_EXPRESSIONS]);
		expect(initial.love).toBe(0);
		expect(chooseExpression(initial)).toBe("happy");
		const next = smoothScores(expressionScores({ love: 1 }), legacy);
		expect(next.love).toBeCloseTo(0.7);
		expect(next.happy).toBeCloseTo(0.24);
		expect(Object.values(next).every(Number.isFinite)).toBe(true);
		expect(chooseExpression(next)).toBe("love");
		expect(smoothScores(legacy, { love: 1 }).love).toBeCloseTo(0.3);
	});

	it("ignores invalid and unknown scores instead of selecting an unsupported face", () => {
		const scores = {
			happy: Number.NaN,
			sad: Number.POSITIVE_INFINITY,
			curious: -1,
			love: 0.8,
			unknown: 1,
		};
		expect(chooseExpression(scores)).toBe("love");
		const smoothed = smoothScores(scores, { happy: Number.NaN });
		expect(Object.values(smoothed).every(Number.isFinite)).toBe(true);
		expect(chooseExpression(smoothed)).toBe("love");
	});

	it("classifies context separately, caches it, and lets the current draft dominate", async () => {
		const classify = vi.fn(
			async (text: string): Promise<EmotionScores> =>
				text.startsWith("draft") ? { joy: 0.9 } : { sadness: 1 },
		);
		const scorer = new EmotionScorer(classify);
		const messages = [
			{ role: "assistant" as const, content: "sad previous answer" },
		];
		const scores = await scorer.score({ draft: "draft one", messages });
		expect(chooseExpression(scores)).toBe("happy");
		expect(scores.happy).toBeCloseTo(0.81);
		expect(scores.sad).toBeCloseTo(0.1);
		await scorer.score({ draft: "draft two", messages });
		expect(classify.mock.calls.map(([text]) => text)).toEqual([
			"draft one",
			"sad previous answer",
			"draft two",
		]);
	});

	it("weights assistant wording below the user's recent wording", async () => {
		const scorer = new EmotionScorer(
			async (text): Promise<EmotionScores> =>
				text === "user"
					? { joy: 1 }
					: text === "assistant"
						? { anger: 1 }
						: { neutral: 1 },
		);
		const result = await scorer.score({
			draft: "current",
			messages: [
				{ role: "user", content: "user" },
				{ role: "assistant", content: "assistant" },
			],
		});
		expect(result.happy).toBeGreaterThan(result.concerned);
		expect(chooseExpression(result)).toBe("neutral");
	});

	it("does not run a model for a cleared draft even with emotional history", async () => {
		const classify = vi.fn();
		const scorer = new EmotionScorer(classify);
		expect(
			chooseExpression(
				await scorer.score({
					draft: " ",
					messages: [{ role: "user", content: "sad" }],
				}),
			),
		).toBe("neutral");
		expect(classify).not.toHaveBeenCalled();
	});
});

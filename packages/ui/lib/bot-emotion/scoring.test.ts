import { describe, expect, it, vi } from "vitest";
import { BOT_EXPRESSIONS } from "../../components/bots/types";
import { calibrateEmotionScores } from "./calibration";
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
	it("keeps technical and quoted content out of model input, including history", async () => {
		const classify = vi.fn(async (_text: string) => ({ neutral: 1 }));
		const scorer = new EmotionScorer(classify);
		expect(
			chooseExpression(
				await scorer.score({
					draft: '```js\nconst message = "I hate you 😡";\n```',
					messages: [],
				}),
			),
		).toBe("neutral");
		expect(classify).not.toHaveBeenCalled();
		await scorer.score({
			draft: 'Translate "I love you ❤️" into German.',
			messages: [{ role: "assistant", content: "Code: `I am furious 😡`" }],
		});
		expect(
			classify.mock.calls.map(([text]) => text.replace(/\s+/g, " ")),
		).toEqual(["Translate into German.", "Code:"]);
	});

	it("does not let cheerful history turn marked sarcasm into happiness", async () => {
		const scorer = new EmotionScorer(
			async (text): Promise<EmotionScores> =>
				text === "Great job /s" ? { joy: 0.8, anger: 0.77 } : { joy: 1 },
		);
		const draft = "Great job /s";
		expect(chooseExpression(await scorer.score({ draft, messages: [] }))).toBe(
			"neutral",
		);
		expect(
			chooseExpression(
				await scorer.score({
					draft,
					messages: [{ role: "assistant", content: "Cheerful context" }],
				}),
			),
		).toBe("neutral");
	});

	it.each([
		["admiration", "happy"],
		["amusement", "happy"],
		["anger", "angry"],
		["annoyance", "concerned"],
		["approval", "happy"],
		["caring", "love"],
		["confusion", "curious"],
		["curiosity", "curious"],
		["desire", "curious"],
		["disappointment", "sad"],
		["disapproval", "concerned"],
		["disgust", "angry"],
		["embarrassment", "oops"],
		["excitement", "happy"],
		["fear", "concerned"],
		["gratitude", "happy"],
		["grief", "sad"],
		["joy", "happy"],
		["love", "love"],
		["nervousness", "concerned"],
		["optimism", "happy"],
		["pride", "happy"],
		["realization", "surprised"],
		["relief", "happy"],
		["remorse", "oops"],
		["sadness", "sad"],
		["surprise", "surprised"],
		["neutral", "neutral"],
	])(
		"keeps the model label %s reachable as %s after calibration",
		(label, expected) => {
			expect(
				chooseExpression(
					expressionScores(calibrateEmotionScores({ [label]: 1 })),
				),
			).toBe(expected);
		},
	);

	it.each([
		["😉", "wink"],
		["😅", "oops"],
		["😡", "angry"],
		["🥰", "love"],
		["😐", "neutral"],
	])(
		"keeps %s expressive even when history and model would choose another face",
		async (draft, expected) => {
			const classify = vi.fn(async () => ({ sadness: 1 }));
			const scorer = new EmotionScorer(classify);
			const scores = await scorer.score({
				draft,
				messages: [{ role: "assistant", content: "A sad previous answer" }],
			});
			expect(chooseExpression(scores)).toBe(expected);
			expect(classify).not.toHaveBeenCalled();
		},
	);

	it("still classifies text when an earlier emoji is superseded by new wording", async () => {
		const classify = vi.fn(async () => ({ joy: 1 }));
		const scorer = new EmotionScorer(classify);
		expect(
			chooseExpression(
				await scorer.score({ draft: "I was 😢, now I'm happy", messages: [] }),
			),
		).toBe("happy");
		expect(classify).toHaveBeenCalledOnce();
	});

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

	it("distinguishes anger from concern and keeps weak or conflicting evidence neutral", () => {
		expect(chooseExpression(expressionScores({ anger: 0.8 }))).toBe("angry");
		expect(chooseExpression(expressionScores({ fear: 0.8 }))).toBe("concerned");
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

	it.each(["love", "caring"])(
		"uses the love face for a distinct %s signal",
		(label) => {
			const scores = expressionScores({ [label]: 0.9 });
			expect(scores.love).toBe(0.9);
			expect(scores.happy).toBe(0);
			expect(chooseExpression(scores)).toBe("love");
		},
	);

	it("validates prose before keeping a smile or a playful wink", async () => {
		const classify = vi.fn(
			async (text: string): Promise<EmotionScores> =>
				text.includes("devastated") ? { sadness: 0.95 } : { amusement: 0.8 },
		);
		const scorer = new EmotionScorer(classify);
		expect(
			chooseExpression(
				await scorer.score({ draft: "I am devastated 🙂", messages: [] }),
			),
		).toBe("sad");
		expect(
			chooseExpression(
				await scorer.score({ draft: "Just kidding 😉", messages: [] }),
			),
		).toBe("wink");
		expect(classify).toHaveBeenCalledTimes(2);
	});

	it("responds with concern when strong distress labels agree across different faces", () => {
		expect(
			chooseExpression(
				expressionScores(
					calibrateEmotionScores({ anger: 0.9, annoyance: 0.9 }),
				),
			),
		).toBe("concerned");
		expect(chooseExpression({ sad: 0.8, oops: 0.79, concerned: 0.78 })).toBe(
			"concerned",
		);
		expect(chooseExpression({ sad: 0.8, oops: 0.79, happy: 0.78 })).toBe(
			"neutral",
		);
		expect(chooseExpression({ sad: 0.4, concerned: 0.35 })).toBe("neutral");
	});

	it("preserves a warm reaction when affection and joy agree", () => {
		expect(chooseExpression(expressionScores({ joy: 0.8, love: 0.78 }))).toBe(
			"love",
		);
		expect(chooseExpression(expressionScores({ joy: 0.78, love: 0.8 }))).toBe(
			"love",
		);
		expect(chooseExpression(expressionScores({ joy: 0.4, love: 0.35 }))).toBe(
			"neutral",
		);
		expect(
			chooseExpression(expressionScores({ joy: 0.8, love: 0.79, anger: 0.78 })),
		).toBe("neutral");
		expect(
			chooseExpression(
				expressionScores({ joy: 0.8, love: 0.79, neutral: 0.78 }),
			),
		).toBe("neutral");
	});

	it("returns finite face scores without accepting expression names as model labels", () => {
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
			concerned: 0,
			wink: 0,
			angry: 0.8,
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
		expect(result.happy).toBeGreaterThan(result.angry);
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

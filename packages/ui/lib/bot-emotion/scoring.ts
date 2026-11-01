import {
	BOT_EXPRESSIONS,
	type BotExpression,
} from "../../components/bots/types";
import {
	type EmotionInput,
	type EmotionScores,
	type ExpressionScores,
	boundEmotionInput,
} from "./protocol";

const GROUPS: Record<BotExpression, readonly string[]> = {
	neutral: ["neutral"],
	happy: [
		"admiration",
		"amusement",
		"approval",
		"excitement",
		"gratitude",
		"joy",
		"optimism",
		"pride",
		"relief",
	],
	concerned: [
		"anger",
		"annoyance",
		"disapproval",
		"disgust",
		"embarrassment",
		"fear",
		"nervousness",
		"remorse",
	],
	surprised: ["surprise", "realization"],
	curious: ["curiosity", "confusion"],
	sad: ["sadness", "grief", "disappointment"],
	love: ["love", "caring", "desire"],
	// These faces express the bot's own mood or activity, not the user's wording.
	wink: [],
	angry: [],
	focused: [],
	oops: [],
};

function finiteScore(score: number | undefined): number {
	return typeof score === "number" && Number.isFinite(score)
		? Math.max(0, Math.min(1, score))
		: 0;
}

export function expressionScores(scores: EmotionScores): ExpressionScores {
	return Object.fromEntries(
		BOT_EXPRESSIONS.map((expression) => [
			expression,
			Math.max(
				0,
				...GROUPS[expression].map((label) => finiteScore(scores[label])),
			),
		]),
	) as ExpressionScores;
}

export function chooseExpression(
	scores: Partial<ExpressionScores>,
): BotExpression {
	const ranked = BOT_EXPRESSIONS.map((expression): [BotExpression, number] => [
		expression,
		finiteScore(scores[expression]),
	]).sort((a, b) => b[1] - a[1]);
	const [first, second] = ranked;
	// Partial sentences are often ambiguous. Keep the face resting until a signal separates.
	if (!first || first[1] < 0.38 || first[1] - (second?.[1] ?? 0) < 0.07)
		return "neutral";
	return first[0];
}

export function smoothScores(
	current: Partial<ExpressionScores>,
	previous?: Partial<ExpressionScores>,
): ExpressionScores {
	return Object.fromEntries(
		BOT_EXPRESSIONS.map((expression) => [
			expression,
			previous
				? 0.7 * finiteScore(current[expression]) +
					0.3 * finiteScore(previous[expression])
				: finiteScore(current[expression]),
		]),
	) as ExpressionScores;
}

/** Scores messages separately so assistant wording cannot be mistaken for the draft. */
export class EmotionScorer {
	private contextCache = new Map<string, EmotionScores>();

	constructor(private classify: (text: string) => Promise<EmotionScores>) {}

	async score(raw: EmotionInput): Promise<ExpressionScores> {
		const input = boundEmotionInput(raw);
		if (!input.draft) return expressionScores({ neutral: 1 });
		const draft = expressionScores(await this.classify(input.draft));
		if (!input.messages.length) return draft;
		const context = expressionScores({});
		let totalWeight = 0;
		for (const [index, message] of input.messages.entries()) {
			let scores = this.contextCache.get(message.content);
			if (!scores) {
				scores = await this.classify(message.content);
				this.contextCache.set(message.content, scores);
				if (this.contextCache.size > 8) {
					const oldest = this.contextCache.keys().next().value;
					if (oldest !== undefined) this.contextCache.delete(oldest);
				}
			}
			const weight = (index + 1) * (message.role === "user" ? 1 : 0.35);
			const mapped = expressionScores(scores);
			for (const key of Object.keys(context) as BotExpression[])
				context[key] += mapped[key] * weight;
			totalWeight += weight;
		}
		// History can break a close tie, while current wording retains most of the weight.
		for (const key of Object.keys(draft) as BotExpression[])
			draft[key] = draft[key] * 0.9 + (context[key] / totalWeight) * 0.1;
		return draft;
	}
}

import {
	BOT_EXPRESSIONS,
	type BotExpression,
} from "../../components/bots/types";
import {
	explicitDraftCue,
	hasReactionProse,
	hasSarcasmCue,
	reactionText,
} from "./draft-cues";
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
	concerned: ["annoyance", "disapproval", "fear", "nervousness"],
	surprised: ["surprise", "realization"],
	// Desire also covers ordinary requests such as wanting a report exported.
	curious: ["curiosity", "confusion", "desire"],
	sad: ["sadness", "grief", "disappointment"],
	love: ["love", "caring"],
	angry: ["anger", "disgust"],
	oops: ["embarrassment", "remorse"],
	// Playful winks need an explicit cue; concentration comes from bot activity.
	wink: [],
	focused: [],
};

const WARM: readonly BotExpression[] = ["happy", "love", "wink"];
const DISTRESS: readonly BotExpression[] = [
	"concerned",
	"sad",
	"angry",
	"oops",
];

function cueConflictsWithText(cue: BotExpression, scores: ExpressionScores) {
	const opposing =
		WARM.includes(cue) || cue === "oops"
			? DISTRESS.filter((expression) => expression !== "oops")
			: DISTRESS.includes(cue)
				? WARM
				: [];
	return opposing.some((expression) => scores[expression] >= 0.65);
}

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
	if (!first || first[1] < 0.38) return "neutral";
	if (first[1] - (second?.[1] ?? 0) < 0.07) {
		const contenders = ranked.filter(([, score]) => first[1] - score < 0.07);
		// Affection and happiness reinforce each other instead of cancelling to neutral.
		if (
			contenders.every(
				([expression, score]) =>
					score >= 0.38 && (expression === "happy" || expression === "love"),
			)
		)
			return "love";
		if (
			contenders.every(
				([expression, score]) => score >= 0.38 && DISTRESS.includes(expression),
			)
		)
			return "concerned";
		// Conflicting signals need more wording before choosing a face.
		return "neutral";
	}
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
		const explicit = explicitDraftCue(input.draft);
		if (explicit && !hasReactionProse(input.draft))
			return { ...expressionScores({}), [explicit]: 1 };
		const text = reactionText(input.draft).trim();
		if (!text) return expressionScores({ neutral: 1 });
		const draft = expressionScores(await this.classify(text));
		// Emoji are fast provisional cues. Strong opposing wording gets the final say.
		if (explicit && !cueConflictsWithText(explicit, draft))
			return { ...expressionScores({}), [explicit]: 1 };
		// A marked sarcastic compliment is not reliable evidence for a cheerful face.
		const finalScores = () =>
			hasSarcasmCue(input.draft) && WARM.includes(chooseExpression(draft))
				? expressionScores({ neutral: 1 })
				: draft;
		if (!input.messages.length) return finalScores();
		const context = expressionScores({});
		let totalWeight = 0;
		for (const [index, message] of input.messages.entries()) {
			let scores = this.contextCache.get(message.content);
			if (!scores) {
				const contextText = reactionText(message.content).trim();
				scores = contextText
					? await this.classify(contextText)
					: { neutral: 1 };
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
		return finalScores();
	}
}

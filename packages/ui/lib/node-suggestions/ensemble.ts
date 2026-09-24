import type { NgramModel } from "./ngram";
import {
	GHOST_ALTERNATIVES,
	type RankedCandidate,
	type RankedSuggestion,
	type SuggestionContext,
	type SuggestionModel,
	type SuggestionResult,
} from "./types";

/** Weight of the neural log-probability in the geometric mixture. */
export const NEURAL_WEIGHT = 0.3;
/** Candidates each model contributes to the mixture. */
export const MODEL_POOL = 60;
const EMPTY_FLOOR = -30;

function logScores(candidates: RankedCandidate[]): Map<string, number> {
	const scores = new Map<string, number>();
	for (const { type, probability } of candidates) {
		if (probability > 0) scores.set(type, Math.log(probability));
	}
	return scores;
}

function softmax(scores: Map<string, number>): [string, number][] {
	let max = Number.NEGATIVE_INFINITY;
	for (const score of scores.values()) max = Math.max(max, score);
	let total = 0;
	const weighted: [string, number][] = [];
	for (const [type, score] of scores) {
		const weight = Math.exp(score - max);
		total += weight;
		weighted.push([type, weight]);
	}
	return weighted.map(([type, weight]) => [type, weight / total]);
}

/**
 * Geometric mixture of an n-gram and a neural top list, renormalized over their union, best first.
 * A type missing from one list scores that list's floor; an empty neural list leaves the n-gram
 * distribution unchanged.
 */
export function mixRanked(
	ngramRanked: RankedCandidate[],
	neuralRanked: RankedCandidate[],
	neuralWeight: number,
): [string, number][] {
	const ngramScores = logScores(ngramRanked);
	const neuralScores = logScores(neuralRanked);
	const weight = neuralScores.size > 0 ? neuralWeight : 0;
	const ngramFloor =
		ngramScores.size > 0 ? Math.min(...ngramScores.values()) - 1 : EMPTY_FLOOR;
	const neuralFloor =
		neuralScores.size > 0 ? Math.min(...neuralScores.values()) : EMPTY_FLOOR;

	const mixed = new Map<string, number>();
	for (const type of new Set([...ngramScores.keys(), ...neuralScores.keys()])) {
		mixed.set(
			type,
			weight * (neuralScores.get(type) ?? neuralFloor) +
				(1 - weight) * (ngramScores.get(type) ?? ngramFloor),
		);
	}
	return softmax(mixed).sort((a, b) => b[1] - a[1]);
}

/**
 * Ranks a self-type top-1 second, in place: same-type repeats are common in finished boards but
 * rarely what the user places next.
 */
export function rankSelfTypeSecond(
	ranked: [string, number][],
	src: string,
): [string, number][] {
	if (ranked.length > 1 && ranked[0][0] === src) {
		[ranked[0], ranked[1]] = [ranked[1], ranked[0]];
	}
	return ranked;
}

/**
 * `mixRanked` of the n-gram and (optional) neural distributions with `NEURAL_WEIGHT` — the
 * combination the weight was tuned on — and the self-type policy applied.
 */
export function suggest(
	models: { ngram: NgramModel; neural?: SuggestionModel },
	context: SuggestionContext,
	limit = GHOST_ALTERNATIVES * 2,
): SuggestionResult {
	const { ngram, neural } = models;
	const ranked = rankSelfTypeSecond(
		mixRanked(
			ngram.rank(context, MODEL_POOL),
			neural?.rank(context, MODEL_POOL) ?? [],
			NEURAL_WEIGHT,
		),
		context.src,
	);
	const candidates: RankedSuggestion[] = ranked
		.slice(0, Math.max(limit, 0))
		.map(([type, probability]) => ({
			type,
			pin: ngram.predictPin(context, type),
			probability,
		}));
	const pUnconnected = ngram.pUnconnected(context);
	return {
		candidates,
		pUnconnected,
		confidence:
			candidates.length > 0
				? (1 - pUnconnected) * candidates[0].probability
				: 0,
	};
}

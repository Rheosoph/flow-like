import { createHash } from "node:crypto";

/** Reciprocal ranks past this position count as 0 (MRR@10). */
export const MRR_CUTOFF = 10;

/** `int(md5(app)) % folds` — the app-split of the Python reference, so both see the same folds. */
export function foldOf(app: string, folds: number): number {
	const digest = createHash("md5").update(app).digest("hex");
	return Number(BigInt(`0x${digest}`) % BigInt(folds));
}

/** 1-based position of `target` in `types`, `0` when it is missing. */
export function rankOf(types: readonly string[], target: string): number {
	return types.indexOf(target) + 1;
}

export interface RankTally {
	n: number;
	hit1: number;
	hit3: number;
	hit5: number;
	reciprocal: number;
}

export interface RankMetrics {
	n: number;
	hit1: number;
	hit3: number;
	hit5: number;
	mrr10: number;
}

export const emptyTally = (): RankTally => ({
	n: 0,
	hit1: 0,
	hit3: 0,
	hit5: 0,
	reciprocal: 0,
});

/** Counts one query; `rank` is 1-based, `0` for a miss. */
export function addRank(tally: RankTally, rank: number): void {
	tally.n++;
	if (rank < 1) return;
	if (rank <= 1) tally.hit1++;
	if (rank <= 3) tally.hit3++;
	if (rank <= 5) tally.hit5++;
	if (rank <= MRR_CUTOFF) tally.reciprocal += 1 / rank;
}

export function rankMetrics(tally: RankTally): RankMetrics {
	const share = (count: number) => (tally.n > 0 ? count / tally.n : 0);
	return {
		n: tally.n,
		hit1: share(tally.hit1),
		hit3: share(tally.hit3),
		hit5: share(tally.hit5),
		mrr10: share(tally.reciprocal),
	};
}

/** One anchor pin: the confidence its ghost would carry (`0` without a ghost) and whether it was right. */
export interface GateItem {
	confidence: number;
	correct: boolean;
}

export interface GatePoint {
	threshold: number;
	/** Share of all pins that show a ghost. */
	coverage: number;
	/** Share of shown ghosts that were right; `0` when none is shown. */
	precision: number;
	shown: number;
}

/** A ghost is shown when its confidence reaches `threshold`, as in the editor. */
export function gatePoint(
	items: readonly GateItem[],
	threshold: number,
): GatePoint {
	let shown = 0;
	let correct = 0;
	for (const item of items) {
		if (item.confidence < threshold) continue;
		shown++;
		if (item.correct) correct++;
	}
	return {
		threshold,
		coverage: items.length > 0 ? shown / items.length : 0,
		precision: shown > 0 ? correct / shown : 0,
		shown,
	};
}

/**
 * The lowest threshold whose shown ghosts still reach `precision`, i.e. the largest coverage at that
 * precision. Pins without a ghost never count; equal confidences are cut together, as a threshold would.
 */
export function thresholdForPrecision(
	items: readonly GateItem[],
	precision: number,
): GatePoint | undefined {
	const sorted = items
		.filter((item) => item.confidence > 0)
		.sort((a, b) => b.confidence - a.confidence);
	let correct = 0;
	let best: GatePoint | undefined;
	for (let index = 0; index < sorted.length; index++) {
		if (sorted[index].correct) correct++;
		if (sorted[index + 1]?.confidence === sorted[index].confidence) continue;
		const shown = index + 1;
		if (correct / shown >= precision) {
			best = {
				threshold: sorted[index].confidence,
				coverage: shown / items.length,
				precision: correct / shown,
				shown,
			};
		}
	}
	return best;
}

export interface CrossFoldGate {
	/** Pooled over the held-out folds. */
	coverage: number;
	precision: number;
	/** Threshold picked on the other folds, per held-out fold that had one. */
	thresholds: number[];
	/** Lowest and highest precision of the folds that showed a ghost. */
	precisionRange: [number, number];
}

/**
 * Honest calibration: each fold is gated with the threshold that reaches `precision` on the other
 * folds, so the reported precision is what a threshold picked offline realizes on unseen apps.
 */
export function crossFoldThreshold(
	itemsByFold: readonly (readonly GateItem[])[],
	precision: number,
): CrossFoldGate | undefined {
	let total = 0;
	let shown = 0;
	let correct = 0;
	const thresholds: number[] = [];
	const precisions: number[] = [];
	itemsByFold.forEach((held, fold) => {
		const others = itemsByFold.filter((_, other) => other !== fold).flat();
		const picked = thresholdForPrecision(others, precision);
		if (!picked || held.length === 0) return;
		const point = gatePoint(held, picked.threshold);
		thresholds.push(picked.threshold);
		if (point.shown > 0) precisions.push(point.precision);
		total += held.length;
		shown += point.shown;
		correct += point.precision * point.shown;
	});
	if (total === 0) return undefined;
	return {
		coverage: shown / total,
		precision: shown > 0 ? correct / shown : 0,
		thresholds,
		precisionRange:
			precisions.length > 0
				? [Math.min(...precisions), Math.max(...precisions)]
				: [0, 0],
	};
}

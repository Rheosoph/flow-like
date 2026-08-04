import type { IScoreCategory } from "../../../lib/board-metrics";

/**
 * Every class string here is written out in full. Tailwind has no runtime JIT in
 * this app, so a class assembled from a template literal simply does not exist.
 */

export type IScoreBand = "flagged" | "watch" | "good" | "unscored";

/** Mirrors the server's SCORE_FLAG_THRESHOLD of 4. */
export function bandOf(score: number | undefined): IScoreBand {
	if (score === undefined) return "unscored";
	if (score >= 7) return "good";
	if (score >= 4) return "watch";
	return "flagged";
}

export const BAND_LABEL: Record<IScoreBand, string> = {
	flagged: "Flagged",
	watch: "Watch",
	good: "Good",
	unscored: "Not scored",
};

export const BAND_DESCRIPTOR: Record<IScoreBand, string> = {
	flagged: "weakest dimension is under 4",
	watch: "weakest dimension is 4 to 6",
	good: "every dimension is 7 or better",
	unscored: "no node declares a score",
};

export const BAND_SQUARE: Record<IScoreBand, string> = {
	flagged: "bg-red-500",
	watch: "bg-amber-500",
	good: "bg-emerald-500",
	unscored: "bg-muted-foreground/40",
};

export const BAND_TEXT: Record<IScoreBand, string> = {
	flagged: "text-red-600 dark:text-red-400",
	watch: "text-amber-600 dark:text-amber-400",
	good: "text-emerald-600 dark:text-emerald-400",
	unscored: "text-muted-foreground",
};

export const BAND_FILL: Record<IScoreBand, string> = {
	flagged: "bg-red-500",
	watch: "bg-amber-500",
	good: "bg-emerald-500",
	unscored: "bg-muted-foreground/30",
};

/** Background for the weakest cell in the meter and for cause chips. */
export const BAND_TINT: Record<IScoreBand, string> = {
	flagged: "bg-red-500/10 dark:bg-red-500/15",
	watch: "bg-amber-500/10 dark:bg-amber-500/15",
	good: "bg-emerald-500/10 dark:bg-emerald-500/15",
	unscored: "bg-muted/60",
};

export const DIMENSION_LABEL: Record<IScoreCategory, string> = {
	security: "Security",
	privacy: "Privacy",
	governance: "Governance",
	performance: "Performance",
	reliability: "Reliability",
	cost: "Cost",
};

export const DIMENSION_SHORT: Record<IScoreCategory, string> = {
	security: "SEC",
	privacy: "PRIV",
	governance: "GOV",
	performance: "PERF",
	reliability: "REL",
	cost: "COST",
};

/**
 * The technical-quality component of the app's EU AI Act conformity score,
 * mirroring `conformity_score` in
 * `packages/api/src/routes/app/ai_act/questionnaire.rs`:
 *   board = min(security, governance) / 10 * 25, out of 100 total.
 * The server assumes a neutral 5 when a board is unscored; we only render the
 * line when at least one board carries real scores.
 */
export const AI_ACT_MAX_POINTS = 25;

export function aiActPoints(worstSecurityGovernance: number): number {
	const clamped = Math.max(0, Math.min(10, worstSecurityGovernance));
	return Math.round((clamped / 10) * AI_ACT_MAX_POINTS);
}

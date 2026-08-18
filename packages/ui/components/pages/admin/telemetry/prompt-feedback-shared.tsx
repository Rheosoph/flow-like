"use client";

import { useTranslation } from "@flow-like/locales";
import { ThumbsDown, ThumbsUp } from "lucide-react";
import { Badge } from "../../../ui";

/**
 * The rating scale is unsigned 0..5: the top of it is praise, the bottom a complaint. Reading
 * `rating > 0` as positive would count every thumbs-down as praise — a mistake this codebase has
 * already made twice (analytics-dashboard.tsx and the app analytics rollup), which is why both ends
 * are explicit and live in one place. Mirrors `is_positive`/`is_negative` in
 * packages/api/src/routes/admin/telemetry/prompt_feedback.rs.
 */
export function isPositiveRating(rating: number) {
	return rating >= 4;
}

export function isNegativeRating(rating: number) {
	return rating > 0 && rating <= 2;
}

/**
 * A turn the user was not happy with. `partial` counts: the assistant stopped short of what it was
 * asked for, which is exactly the case a reviewer is hunting.
 */
export function isFailedOutcome(outcome: string | null | undefined) {
	return outcome === "error" || outcome === "timeout" || outcome === "partial";
}

/**
 * A withdrawn rating deletes its row server-side, so a stored mid-scale value is a real rating and
 * must not be labelled "unrated".
 */
export function PromptFeedbackRatingBadge({
	rating,
	className,
}: {
	readonly rating: number;
	readonly className?: string;
}) {
	const { t } = useTranslation("admin");

	if (isPositiveRating(rating)) {
		return (
			<Badge
				variant="outline"
				className={`gap-1 border-emerald-500/40 bg-emerald-500/10 text-[10px] uppercase text-emerald-600 dark:text-emerald-400 ${className ?? ""}`}
			>
				<ThumbsUp className="h-3 w-3" />
				{t("positive", "Positive")}
			</Badge>
		);
	}

	if (isNegativeRating(rating)) {
		return (
			<Badge
				variant="outline"
				className={`gap-1 border-destructive/40 bg-destructive/10 text-[10px] uppercase text-destructive ${className ?? ""}`}
			>
				<ThumbsDown className="h-3 w-3" />
				{t("negative", "Negative")}
			</Badge>
		);
	}

	return (
		<Badge
			variant="outline"
			className={`text-[10px] uppercase ${className ?? ""}`}
		>
			{t("ratingVal", "Rating {{val}}", { val: rating })}
		</Badge>
	);
}

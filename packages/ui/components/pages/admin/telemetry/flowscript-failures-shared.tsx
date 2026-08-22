"use client";

import { Badge } from "../../../ui";

/**
 * An outcome that means the user's edit did not land at all — as opposed to `partial`, where the
 * board changed but part of the source was skipped. Both are worth reading; only these two are
 * failures in the strict sense.
 */
export function isBlockingOutcome(outcome: string): boolean {
	return outcome === "error" || outcome === "blocked";
}

export function FlowScriptOutcomeBadge({
	outcome,
}: {
	readonly outcome: string;
}) {
	return (
		<Badge
			variant={isBlockingOutcome(outcome) ? "destructive" : "secondary"}
			className="text-[10px]"
		>
			{outcome}
		</Badge>
	);
}

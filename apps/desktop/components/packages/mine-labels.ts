"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import type { MineFilter, MineState } from "./mine-model";

export const MINE_STATE_TONE: Record<MineState, { text: string; dot: string }> =
	{
		"unpublished-changes": { text: "text-primary", dot: "bg-primary" },
		"in-review": {
			text: "text-sky-600 dark:text-sky-400",
			dot: "bg-sky-500",
		},
		live: {
			text: "text-emerald-600 dark:text-emerald-400",
			dot: "bg-emerald-500",
		},
		"not-on-this-machine": {
			text: "text-violet-600 dark:text-violet-400",
			dot: "bg-violet-500",
		},
		"local-only": {
			text: "text-muted-foreground",
			dot: "bg-muted-foreground",
		},
		disabled: { text: "text-destructive", dot: "bg-destructive" },
	};

export function useMineFilterLabels(): Record<MineFilter, string> {
	const { t } = useTranslation("common");
	return useMemo(
		() => ({
			all: t("all", "All"),
			"unpublished-changes": t("unpublishedChanges", "Unpublished changes"),
			"in-review": t("inReview", "In review"),
			live: t("live", "Live"),
			"local-only": t("localOnly", "Local only"),
			"not-on-this-machine": t("notOnThisMachine", "Not on this machine"),
			disabled: t("disabled", "Disabled"),
			issues: t("issues", "Issues"),
		}),
		[t],
	);
}

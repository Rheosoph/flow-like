"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import type { MineFilter, MineState } from "./mine-model";

export const MINE_STATE_TONE: Record<MineState, { text: string; dot: string }> =
	{
		"unpublished-changes": { text: "text-primary", dot: "bg-primary" },
		"in-review": { text: "text-info", dot: "bg-info-solid" },
		live: { text: "text-good", dot: "bg-good-solid" },
		"not-on-this-machine": { text: "text-tab-purple", dot: "bg-tab-purple" },
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

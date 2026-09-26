"use client";

import { useTranslation } from "@flow-like/locales";
import { useId } from "react";
import { Label } from "../ui/label";
import { Switch } from "../ui/switch";

/** Pages replay their last onLoad output by default; this opts a page out of that. */
export function PageNoCacheSetting({
	noCache,
	onChange,
}: Readonly<{
	noCache: boolean;
	onChange: (noCache: true | undefined) => void;
}>) {
	const { t } = useTranslation("common");
	const id = useId();
	const hintId = `${id}-hint`;
	return (
		<div className="flex items-start justify-between gap-3">
			<div className="space-y-1">
				<Label htmlFor={id}>{t("noCache", "No cache")}</Label>
				<p id={hintId} className="text-xs text-muted-foreground">
					{t(
						"onlyShowFreshWorkflowOutputShowsALoadingScreenUntilTheOnLoadWorkflowRenders",
						"Only show fresh workflow output. Shows a loading screen until the onLoad workflow renders.",
					)}
				</p>
			</div>
			<Switch
				id={id}
				aria-describedby={hintId}
				checked={noCache}
				onCheckedChange={(checked) => onChange(checked ? true : undefined)}
			/>
		</div>
	);
}

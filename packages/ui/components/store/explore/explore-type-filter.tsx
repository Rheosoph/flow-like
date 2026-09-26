"use client";

import { useTranslation } from "@flow-like/locales";
import { cn } from "../../../lib/utils";
import { useExploreLabels } from "./explore-labels";
import type { ExploreTypeFilter } from "./explore-types";

const TYPES: readonly ExploreTypeFilter[] = ["all", "apps", "packages"];

/** All · Apps · Packages, each switching to its own server-resolved view. */
export function ExploreTypeTabs({
	value,
	counts,
	onChange,
	className,
}: Readonly<{
	value: ExploreTypeFilter;
	counts: { apps: number; packages: number };
	onChange: (type: ExploreTypeFilter) => void;
	className?: string;
}>) {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	const count: Record<ExploreTypeFilter, number> = {
		all: counts.apps + counts.packages,
		apps: counts.apps,
		packages: counts.packages,
	};
	return (
		<fieldset
			aria-label={t("exploreFilterByType", "Filter by type")}
			data-explore-type-filter
			className={cn(
				"m-0 inline-flex h-9.5 min-w-0 items-center gap-0.5 rounded-lg border border-border bg-card/80 p-0.75 backdrop-blur-sm",
				className,
			)}
		>
			{TYPES.map((type) => {
				const active = type === value;
				return (
					<button
						key={type}
						type="button"
						aria-pressed={active}
						onClick={() => onChange(type)}
						className={cn(
							"inline-flex h-7.5 items-center gap-1.5 rounded-md px-3 text-[13px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
							active
								? "bg-muted font-semibold text-foreground"
								: "font-medium text-muted-foreground hover:text-foreground",
						)}
					>
						{labels.type(type)}
						<span
							className={cn(
								"font-mono text-[11px] tabular-nums",
								active ? "text-muted-foreground" : "text-muted-foreground/80",
							)}
						>
							{count[type]}
						</span>
					</button>
				);
			})}
		</fieldset>
	);
}

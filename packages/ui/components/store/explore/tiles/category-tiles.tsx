"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronRight } from "lucide-react";
import Link from "next/link";
import { useAppCategoryLabel } from "../../../../lib/app-category";
import { categoryColor, categoryIcon } from "../../../../lib/category-meta";
import { cn } from "../../../../lib/utils";
import { exploreSearchHref } from "../explore-href";
import { useExploreLabels } from "../explore-labels";
import type { ExploreRail, ExploreTypeFilter } from "../explore-types";
import {
	type ResponsiveVariants,
	type TileShape,
	responsive,
} from "./tile-shape";

const MAX_TILES = 4;

const COLUMNS: ResponsiveVariants = {
	mdRow: "@3xl/explore:grid-cols-4",
	mdColumn: "@3xl/explore:grid-cols-2",
	xlRow: "@5xl/explore:grid-cols-4",
	xlColumn: "@5xl/explore:grid-cols-2",
};

/** Up to four category tiles: 2×2 in a two-row slot, one row of four in a one-row slot. */
export function CategoryTiles({
	rail,
	type,
	short,
	mdShort,
}: Readonly<{ rail: ExploreRail; type: ExploreTypeFilter } & TileShape>) {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	const categoryLabel = useAppCategoryLabel();
	const categories = (rail.categories ?? []).slice(0, MAX_TILES);
	if (!categories.length) return null;
	return (
		<nav
			aria-label={rail.title ?? t("browseCategories", "Browse categories")}
			data-explore-tile="categories"
			className={cn(
				"grid h-full auto-rows-fr grid-cols-2 gap-4",
				responsive({ short, mdShort }, COLUMNS),
			)}
		>
			{categories.map((entry) => {
				const color = categoryColor(entry.appCategory);
				const Icon = categoryIcon(entry.appCategory);
				const name = categoryLabel(entry.appCategory);
				return (
					<Link
						key={entry.appCategory}
						href={exploreSearchHref({
							categories: [`app:${entry.appCategory}`],
							type: type === "all" ? undefined : type,
						})}
						aria-label={t("exploreBrowseCategory", {
							defaultValue: "Browse {{category}}",
							category: name,
						})}
						className="group relative flex min-h-23 min-w-0 flex-col justify-between gap-2 overflow-hidden rounded-[14px] border border-border/70 bg-card px-3.5 py-3 transition-colors hover:border-primary/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
					>
						<span className="flex items-start justify-between">
							<span
								aria-hidden="true"
								className="flex size-7 items-center justify-center rounded-lg"
								style={{
									color,
									backgroundColor: `color-mix(in oklch, ${color} 14%, transparent)`,
								}}
							>
								<Icon className="size-4" />
							</span>
							<ChevronRight
								aria-hidden="true"
								className="size-3.5 text-muted-foreground transition-transform group-hover:translate-x-0.5"
							/>
						</span>
						<span className="flex min-w-0 flex-col gap-0.5">
							<span className="truncate text-sm font-semibold leading-4.5">
								{name}
							</span>
							<span className="truncate font-mono text-[11px] text-muted-foreground">
								{labels.kindCounts(entry.apps, entry.packages)}
							</span>
						</span>
					</Link>
				);
			})}
		</nav>
	);
}

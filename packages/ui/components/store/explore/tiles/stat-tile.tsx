"use client";

import { useTranslation } from "@flow-like/locales";
import { TrendingUp } from "lucide-react";
import { cn } from "../../../../lib/utils";
import type { ExploreRail } from "../explore-types";
import {
	type ResponsiveVariants,
	type TileShape,
	responsive,
} from "./tile-shape";

const TILE: ResponsiveVariants = {
	mdRow: "@3xl/explore:flex-row @3xl/explore:items-center",
	mdColumn: "@3xl/explore:flex-col @3xl/explore:items-stretch",
	xlRow: "@5xl/explore:flex-row @5xl/explore:items-center",
	xlColumn: "@5xl/explore:flex-col @5xl/explore:items-stretch",
};

const HEADLINE: ResponsiveVariants = {
	mdRow: "@3xl/explore:flex-row @3xl/explore:items-center @3xl/explore:gap-3",
	mdColumn: "@3xl/explore:flex-col @3xl/explore:items-start @3xl/explore:gap-0",
	xlRow: "@5xl/explore:flex-row @5xl/explore:items-center @5xl/explore:gap-3",
	xlColumn: "@5xl/explore:flex-col @5xl/explore:items-start @5xl/explore:gap-0",
};

const EYEBROW: ResponsiveVariants = {
	mdRow: "@3xl/explore:hidden",
	mdColumn: "@3xl/explore:inline-flex",
	xlRow: "@5xl/explore:hidden",
	xlColumn: "@5xl/explore:inline-flex",
};

const NUMBER: ResponsiveVariants = {
	mdRow: "@3xl/explore:mt-0 @3xl/explore:pt-0 @3xl/explore:text-4xl",
	mdColumn: "@3xl/explore:mt-auto @3xl/explore:pt-2 @3xl/explore:text-[54px]",
	xlRow: "@5xl/explore:mt-0 @5xl/explore:pt-0 @5xl/explore:text-4xl",
	xlColumn: "@5xl/explore:mt-auto @5xl/explore:pt-2 @5xl/explore:text-[54px]",
};

const BREAKDOWN: ResponsiveVariants = {
	mdRow:
		"@3xl/explore:ml-auto @3xl/explore:mt-0 @3xl/explore:min-w-36 @3xl/explore:border-l @3xl/explore:border-t-0 @3xl/explore:pl-4 @3xl/explore:pt-0",
	mdColumn:
		"@3xl/explore:ml-0 @3xl/explore:mt-3 @3xl/explore:min-w-0 @3xl/explore:border-l-0 @3xl/explore:border-t @3xl/explore:pl-0 @3xl/explore:pt-2.5",
	xlRow:
		"@5xl/explore:ml-auto @5xl/explore:mt-0 @5xl/explore:min-w-36 @5xl/explore:border-l @5xl/explore:border-t-0 @5xl/explore:pl-4 @5xl/explore:pt-0",
	xlColumn:
		"@5xl/explore:ml-0 @5xl/explore:mt-3 @5xl/explore:min-w-0 @5xl/explore:border-l-0 @5xl/explore:border-t @5xl/explore:pl-0 @5xl/explore:pt-2.5",
};

/** "New this week" count. A one-row tile lays the breakdown out beside the number. */
export function StatTile({
	rail,
	short,
	mdShort,
}: Readonly<{ rail: ExploreRail } & TileShape>) {
	const { t } = useTranslation("store");
	const stat = rail.stat;
	if (!stat) return null;
	const shape = { short, mdShort };
	const mixed = stat.apps > 0 && stat.packages > 0;
	const packagesOnly = !mixed && stat.packages > 0;
	const label = mixed
		? t("exploreStatNew", "new this week")
		: packagesOnly
			? t("exploreStatNewPackages", "new packages this week")
			: t("exploreStatNewApps", "new apps this week");
	const rows: [label: string, value: number][] = mixed
		? [
				[t("apps", "Apps"), stat.apps],
				[t("packages", "Packages"), stat.packages],
			]
		: packagesOnly
			? [[t("verified", "Verified"), stat.verifiedPackages]]
			: [
					[t("free", "Free"), stat.freeApps],
					[t("exploreStatPaid", "Paid"), stat.paidApps],
				];
	return (
		<article
			aria-label={rail.title ?? t("exploreRailNew", "New this week")}
			data-explore-tile="stat"
			className={cn(
				"relative flex h-full min-w-0 flex-col gap-x-4 rounded-2xl border border-border/70 bg-card px-4.5 py-4",
				responsive(shape, TILE),
			)}
		>
			<div
				className={cn(
					"flex min-w-0 flex-1 flex-col",
					responsive(shape, HEADLINE),
				)}
			>
				<span
					className={cn(
						"inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase leading-3.5 tracking-wider text-muted-foreground",
						responsive(shape, EYEBROW),
					)}
				>
					<TrendingUp
						aria-hidden="true"
						className="size-3.25 text-emerald-500 dark:text-emerald-400"
					/>
					{t("exploreStatFresh", "Fresh")}
				</span>
				<span
					className={cn(
						"mt-2 text-5xl font-semibold leading-none tracking-tighter tabular-nums",
						responsive(shape, NUMBER),
					)}
				>
					{stat.total}
				</span>
				<span className="mt-1 text-[13px] leading-4.5">{label}</span>
			</div>
			<dl
				className={cn(
					"mt-3 flex flex-col gap-1 border-t border-border/70 pt-2.5",
					responsive(shape, BREAKDOWN),
				)}
			>
				{rows.map(([name, value]) => (
					<div
						key={name}
						className="flex items-center justify-between gap-3 text-xs leading-4.25"
					>
						<dt className="text-muted-foreground">{name}</dt>
						<dd className="font-mono font-semibold tabular-nums">{value}</dd>
					</div>
				))}
			</dl>
		</article>
	);
}

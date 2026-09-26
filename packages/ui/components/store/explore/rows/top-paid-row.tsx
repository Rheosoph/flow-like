"use client";

import { useTranslation } from "@flow-like/locales";
import { Shield, Star } from "lucide-react";
import { useAppCategoryLabel } from "../../../../lib/app-category";
import { categoryColor } from "../../../../lib/category-meta";
import { cn } from "../../../../lib/utils";
import { formatCompact } from "../../package-card";
import {
	type ExploreAppItem,
	ExploreItemLink,
	type ExplorePackageItem,
	ItemThumb,
	PackageGlyph,
	itemDescription,
	itemId,
	itemName,
} from "../explore-item-card";
import { formatPrice } from "../explore-labels";
import { layoutTopPaid } from "../explore-model";
import type { ExploreRail } from "../explore-types";
import {
	ExploreRowSection,
	type RowContext,
	useRailHeading,
} from "./row-section";

const TILE =
	"group relative flex h-18 min-w-0 items-center gap-3.5 overflow-hidden rounded-[14px] border border-border/70 bg-card px-3.5 transition-colors hover:border-primary/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

/** Two apps and two packages (four apps without packages); fewer than four tiles get the wide layout. */
export function TopPaidRow({
	rail,
	context,
}: Readonly<{ rail: ExploreRail; context: RowContext }>) {
	const heading = useRailHeading(rail, context.mix);
	const items = layoutTopPaid(rail.items, context.dev);
	if (!items.length) return null;
	const wide = items.length < 4;
	return (
		<ExploreRowSection {...heading}>
			<ol
				className={cn(
					"grid grid-cols-1 gap-4 @3xl/explore:grid-cols-2",
					!wide && "@5xl/explore:grid-cols-4",
				)}
			>
				{items.map((item, index) => (
					<li key={`${item.kind}:${itemId(item)}`} className="min-w-0">
						{item.kind === "app" ? (
							<AppRankTile item={item} rank={index + 1} wide={wide} />
						) : item.kind === "package" ? (
							<PackageRankTile item={item} rank={index + 1} wide={wide} />
						) : null}
					</li>
				))}
			</ol>
		</ExploreRowSection>
	);
}

function Rank({ rank }: Readonly<{ rank: number }>) {
	return (
		<span className="w-3 shrink-0 font-mono text-[13px] tabular-nums text-muted-foreground">
			{rank}
		</span>
	);
}

function AppRankTile({
	item,
	rank,
	wide,
}: Readonly<{ item: ExploreAppItem; rank: number; wide: boolean }>) {
	const categoryLabel = useAppCategoryLabel();
	const rated = item.app.rating_count > 0;
	return (
		<ExploreItemLink item={item} className={TILE} tile="rank">
			<Rank rank={rank} />
			<ItemThumb
				item={item}
				className="size-11 rounded-lg border border-border/60"
			/>
			<div className="min-w-0 flex-1">
				<div
					className="truncate text-[11px] font-semibold uppercase leading-3.5 tracking-wider"
					style={{ color: categoryColor(item.app.primary_category) }}
				>
					{categoryLabel(item.app.primary_category)}
				</div>
				<div className="truncate text-[15px] font-semibold leading-5">
					{itemName(item)}
				</div>
				{wide && (
					<div className="truncate text-xs text-muted-foreground">
						{itemDescription(item)}
					</div>
				)}
			</div>
			{wide && rated && (
				<span className="hidden shrink-0 items-center gap-1 text-[13px] @xl/explore:flex">
					<Star
						aria-hidden="true"
						className="size-3.5 fill-yellow-400 text-yellow-400"
					/>
					<span className="font-semibold tabular-nums">
						{(item.app.avg_rating ?? 0).toFixed(1)}
					</span>
					<span className="text-xs text-muted-foreground">
						{`(${item.app.rating_count})`}
					</span>
				</span>
			)}
			<span className="inline-flex h-7 shrink-0 items-center rounded-full bg-foreground px-3 text-xs font-bold tabular-nums text-background">
				{formatPrice(item.app.price ?? 0)}
			</span>
		</ExploreItemLink>
	);
}

function PackageRankTile({
	item,
	rank,
	wide,
}: Readonly<{ item: ExplorePackageItem; rank: number; wide: boolean }>) {
	const { t } = useTranslation("store");
	const pkg = item.package;
	const rated = (pkg.ratingCount ?? 0) > 0;
	return (
		<ExploreItemLink item={item} className={TILE} tile="rank">
			<div
				aria-hidden="true"
				className="pointer-events-none absolute inset-0 bg-[radial-gradient(var(--border)_0.5px,transparent_0.5px)] bg-size-[7px_7px] opacity-50"
			/>
			<Rank rank={rank} />
			<PackageGlyph
				pkg={pkg}
				className="size-11 rounded-[10px] border border-border/60"
				textClassName="text-[13px]"
			/>
			<div className="relative min-w-0 flex-1">
				<div className="flex min-w-0 items-center gap-1.5">
					<span className="truncate font-mono text-[13px] font-semibold">
						{itemName(item)}
					</span>
					{pkg.verified && (
						<Shield
							aria-label={t("verified", "Verified")}
							className="size-3.5 shrink-0 text-sky-500 dark:text-sky-400"
						/>
					)}
				</div>
				<div className="truncate font-mono text-[11px] text-muted-foreground">
					{t("exploreRankPackageMeta", {
						defaultValue: "{{installs}} installs · v{{version}}",
						installs: formatCompact(pkg.downloadCount),
						version: pkg.latestVersion,
					})}
				</div>
				{wide && (
					<div className="truncate text-xs text-muted-foreground">
						{itemDescription(item)}
					</div>
				)}
			</div>
			{wide && rated && (
				<span className="relative hidden shrink-0 items-center gap-1 font-mono text-[13px] font-semibold @xl/explore:flex">
					<Star
						aria-hidden="true"
						className="size-3.5 fill-yellow-400 text-yellow-400"
					/>
					{(pkg.avgRating ?? 0).toFixed(1)}
				</span>
			)}
			<span className="relative shrink-0 font-mono text-[13px] font-semibold tabular-nums text-primary">
				{formatPrice(pkg.price)}
			</span>
		</ExploreItemLink>
	);
}

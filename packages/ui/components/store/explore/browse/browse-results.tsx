"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowRight,
	Layers,
	LayoutGrid,
	Loader2,
	Package,
	Search,
	X,
} from "lucide-react";
import Link from "next/link";
import type { ReactNode } from "react";
import { useAppCategoryLabel } from "../../../../lib/app-category";
import type { WasmPackageCategory } from "../../../../lib/schema/wasm";
import { cn } from "../../../../lib/utils";
import { Button } from "../../../ui/button";
import { PackageCardStats } from "../../package-card";
import type { ExploreBrowseParams } from "../explore-href";
import {
	CollectionStack,
	ExploreAppCard,
	type ExploreAppItem,
	ExploreItemCard,
	ExplorePackageCard,
	ItemChip,
	PackageStatCells,
	collectionHref,
	itemId,
} from "../explore-item-card";
import { useExploreLabels } from "../explore-labels";
import type {
	ExploreCollection,
	ExploreResolvedItem,
	ExploreSearchPackageHit,
} from "../explore-types";

export const RESULT_GRID =
	"grid grid-cols-1 gap-4 @xl/explore:grid-cols-2 @4xl/explore:grid-cols-3 @7xl/explore:grid-cols-4";

export function BrowseGroupHeader({
	icon,
	title,
	count,
	note,
}: Readonly<{
	icon: ReactNode;
	title: string;
	count: number;
	note?: string;
}>) {
	return (
		<div className="flex min-w-0 flex-wrap items-center gap-2 text-[13px]">
			<span aria-hidden="true" className="text-muted-foreground">
				{icon}
			</span>
			<h2 className="text-sm font-semibold">{title}</h2>
			<span className="rounded-full bg-muted px-2 py-0.5 font-mono text-[11px] tabular-nums text-muted-foreground">
				{count}
			</span>
			{note && <span className="text-muted-foreground">{note}</span>}
		</div>
	);
}

function ShowMore({
	label,
	loading,
	onClick,
}: Readonly<{ label: string; loading: boolean; onClick: () => void }>) {
	return (
		<div className="flex justify-center">
			<Button
				variant="outline"
				size="sm"
				className="rounded-lg"
				disabled={loading}
				onClick={onClick}
			>
				{loading && (
					<Loader2 aria-hidden="true" className="size-3.5 animate-spin" />
				)}
				{label}
			</Button>
		</div>
	);
}

export function BrowseAppsGroup({
	items,
	total,
	query,
	hasMore,
	loadingMore,
	onMore,
	beside = false,
}: Readonly<{
	items: readonly ExploreAppItem[];
	total: number;
	query?: string;
	hasMore: boolean;
	loadingMore: boolean;
	onMore: () => void;
	/** In the narrow column beside the packages on wide screens. */
	beside?: boolean;
}>) {
	const { t } = useTranslation("store");
	return (
		<section className="flex min-w-0 flex-col gap-3" data-browse-group="apps">
			<BrowseGroupHeader
				icon={<LayoutGrid className="size-3.75" />}
				title={t("apps", "Apps")}
				count={total}
				note={query ? t("exploreMatchedInName", "Matched in name") : undefined}
			/>
			<div className={cn(RESULT_GRID, beside && "@7xl/explore:grid-cols-1")}>
				{items.map((item) => (
					<ExploreAppCard key={item.app.id} item={item} />
				))}
			</div>
			{hasMore && (
				<ShowMore
					label={t("exploreShowMoreApps", "Show more apps")}
					loading={loadingMore}
					onClick={onMore}
				/>
			)}
		</section>
	);
}

function MatchNote({ hit }: Readonly<{ hit: ExploreSearchPackageHit }>) {
	const { t } = useTranslation("store");
	const viaCollection = hit.matchedVia === "collection";
	const Icon = viaCollection ? Layers : Search;
	return (
		<span className="relative mt-auto flex min-w-0 items-center gap-1.5 pt-2 text-[11px] text-muted-foreground">
			<Icon
				aria-hidden="true"
				className={cn(
					"size-3.25 shrink-0",
					viaCollection
						? "text-emerald-500 dark:text-emerald-400"
						: "text-primary",
				)}
			/>
			<span className="truncate">
				{viaCollection && hit.collectionTitle
					? t("exploreMatchedInCollection", {
							defaultValue: "In collection “{{title}}”",
							title: hit.collectionTitle,
						})
					: t(
							"exploreMatchedInNameAndDescription",
							"Matched in name and description",
						)}
			</span>
		</span>
	);
}

export function BrowsePackagesGroup({
	hits,
	total,
	query,
	capped,
	hasMore,
	loadingMore,
	onMore,
	beside = false,
}: Readonly<{
	hits: readonly ExploreSearchPackageHit[];
	total: number;
	query?: string;
	/** The server paged within its first 200 candidates; later matches need narrower filters. */
	capped: boolean;
	hasMore: boolean;
	loadingMore: boolean;
	onMore: () => void;
	/** Next to the apps column on wide screens. */
	beside?: boolean;
}>) {
	const { t } = useTranslation("store");
	const byName = hits.filter((hit) => hit.matchedVia === "name").length;
	const viaCollection = hits.length - byName;
	const note = query
		? viaCollection > 0
			? t("exploreMatchedSplit", {
					defaultValue:
						"{{byName}} by name · {{viaCollection}} via a collection",
					byName,
					viaCollection,
				})
			: t("exploreMatchedInName", "Matched in name")
		: undefined;
	return (
		<section
			className="flex min-w-0 flex-col gap-3"
			data-browse-group="packages"
		>
			<BrowseGroupHeader
				icon={<Package className="size-3.75" />}
				title={t("packages", "Packages")}
				count={total}
				note={note}
			/>
			<div className={cn(RESULT_GRID, beside && "@7xl/explore:grid-cols-3")}>
				{hits.map((hit) => (
					<ExplorePackageCard
						key={hit.package.id}
						pkg={hit.package}
						footer={
							<>
								{query && <MatchNote hit={hit} />}
								<PackageCardStats className={query ? "mt-2.5" : undefined}>
									<PackageStatCells pkg={hit.package} />
								</PackageCardStats>
							</>
						}
					/>
				))}
			</div>
			{hasMore && (
				<ShowMore
					label={t("exploreShowMorePackages", "Show more packages")}
					loading={loadingMore}
					onClick={onMore}
				/>
			)}
			{capped && !hasMore && (
				<p className="text-center text-[13px] text-muted-foreground">
					{t(
						"explorePackagesCapped",
						"Refine your filters to see more packages.",
					)}
				</p>
			)}
		</section>
	);
}

/** The curated collection matching a query, or the header of an opened collection. */
export function BrowseCollectionCard({
	collection,
	query,
	open = true,
}: Readonly<{
	collection: ExploreCollection;
	query?: string;
	open?: boolean;
}>) {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	return (
		<article
			aria-label={t("exploreCollectionNamed", {
				defaultValue: "Collection: {{title}}",
				title: collection.title,
			})}
			data-browse-collection={collection.placementId}
			className="relative flex min-w-0 flex-col gap-5 overflow-hidden rounded-2xl border border-border/70 bg-card p-5 @3xl/explore:flex-row @3xl/explore:items-center"
		>
			<CollectionStack
				items={collection.items}
				className="hidden h-29 w-49 @3xl/explore:block"
				cardClassName="h-19 w-31"
			/>
			<div className="flex min-w-0 flex-1 flex-col gap-1.5">
				<span className="flex min-w-0 items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-emerald-600 dark:text-emerald-400">
					<Layers aria-hidden="true" className="size-3.25" />
					{t("exploreCuratedCollection", "Curated collection")}
					{query && (
						<span className="truncate font-medium normal-case tracking-normal text-muted-foreground">
							{t("exploreBestMatchFor", {
								defaultValue: "· best match for “{{query}}”",
								query,
							})}
						</span>
					)}
				</span>
				<h2 className="text-xl font-semibold tracking-tight @3xl/explore:text-2xl">
					{collection.title}
				</h2>
				{collection.blurb && (
					<p className="text-[13px] text-muted-foreground">
						{collection.blurb}
					</p>
				)}
				<div className="mt-1.5 flex min-w-0 flex-wrap gap-1.5">
					{collection.items.slice(0, 4).map((item) => (
						<ItemChip key={`${item.kind}:${itemId(item)}`} item={item} />
					))}
				</div>
			</div>
			<div className="flex shrink-0 items-center justify-between gap-4 @3xl/explore:flex-col @3xl/explore:items-end">
				<span className="font-mono text-xs tabular-nums text-muted-foreground">
					{labels.kindCounts(collection.apps, collection.packages)}
				</span>
				{open && (
					<Link
						href={collectionHref(collection.placementId)}
						className="inline-flex h-9 items-center gap-2 rounded-full bg-foreground px-4 text-[13px] font-semibold text-background transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
					>
						{t("exploreOpenCollection", "Open collection")}
						<ArrowRight aria-hidden="true" className="size-3.5" />
					</Link>
				)}
			</div>
		</article>
	);
}

export function BrowseRelated({
	items,
	query,
}: Readonly<{ items: readonly ExploreResolvedItem[]; query?: string }>) {
	const { t } = useTranslation("store");
	if (!items.length) return null;
	return (
		<section
			className="flex min-w-0 flex-col gap-3"
			data-browse-group="related"
		>
			<div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
				<h2 className="text-sm font-semibold">
					{t("exploreYouMightAlsoLike", "You might also like")}
				</h2>
				{query && (
					<span className="text-[13px] text-muted-foreground">
						{t("exploreRelatedNote", {
							defaultValue: "Popular nearby · not matching “{{query}}”",
							query,
						})}
					</span>
				)}
			</div>
			<div className={RESULT_GRID}>
				{items.map((item) => (
					<ExploreItemCard key={`${item.kind}:${itemId(item)}`} item={item} />
				))}
			</div>
		</section>
	);
}

interface ActiveChip {
	key: string;
	label: string;
	remove: () => void;
}

export function useActiveChips(
	params: ExploreBrowseParams,
	onChange: (params: ExploreBrowseParams) => void,
): ActiveChip[] {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	const categoryLabel = useAppCategoryLabel();
	const chips: ActiveChip[] = [];
	if (params.type && params.type !== "all") {
		chips.push({
			key: "type",
			label: labels.type(params.type),
			remove: () => onChange({ ...params, type: undefined }),
		});
	}
	for (const value of params.categories ?? []) {
		const [kind, name] = value.split(":");
		chips.push({
			key: value,
			label:
				kind === "app"
					? categoryLabel(name)
					: labels.packageCategory(name as WasmPackageCategory),
			remove: () =>
				onChange({
					...params,
					categories: params.categories?.filter((entry) => entry !== value),
				}),
		});
	}
	if (params.price) {
		chips.push({
			key: "price",
			label:
				params.price === "free"
					? t("free", "Free")
					: t("exploreStatPaid", "Paid"),
			remove: () => onChange({ ...params, price: undefined }),
		});
	}
	if (params.verified) {
		chips.push({
			key: "verified",
			label: t("exploreVerifiedPublisher", "Verified publisher"),
			remove: () => onChange({ ...params, verified: undefined }),
		});
	}
	for (const permission of params.permissions ?? []) {
		chips.push({
			key: `permission:${permission}`,
			label: labels.permission(permission),
			remove: () =>
				onChange({
					...params,
					permissions: params.permissions?.filter(
						(entry) => entry !== permission,
					),
				}),
		});
	}
	return chips;
}

export function ActiveChips({ chips }: Readonly<{ chips: ActiveChip[] }>) {
	const { t } = useTranslation("store");
	if (!chips.length) return null;
	return (
		<ul className="flex min-w-0 flex-wrap gap-1.5">
			{chips.map((chip) => (
				<li key={chip.key}>
					<button
						type="button"
						onClick={chip.remove}
						aria-label={t("exploreRemoveFilter", {
							defaultValue: "Remove filter {{label}}",
							label: chip.label,
						})}
						className="inline-flex h-7 items-center gap-1.5 rounded-full border border-border bg-muted/50 pl-2.5 pr-2 text-xs font-medium transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
					>
						{chip.label}
						<X aria-hidden="true" className="size-3" />
					</button>
				</li>
			))}
		</ul>
	);
}

export function BrowseEmpty({
	query,
	hasFilters,
	onClear,
}: Readonly<{ query?: string; hasFilters: boolean; onClear: () => void }>) {
	const { t } = useTranslation("store");
	return (
		<div className="flex flex-col items-center gap-3 rounded-2xl border border-dashed border-border px-6 py-14 text-center">
			<span className="flex size-11 items-center justify-center rounded-xl bg-muted">
				<Search aria-hidden="true" className="size-5 text-muted-foreground" />
			</span>
			<h2 className="text-base font-semibold">
				{hasFilters
					? t("exploreNothingMatchesFilters", "Nothing matches these filters")
					: query
						? t("exploreNothingMatchesQuery", {
								defaultValue: "Nothing matches “{{query}}”",
								query,
							})
						: t("exploreNothingHere", "Nothing to show yet")}
			</h2>
			<p className="max-w-sm text-sm text-muted-foreground">
				{hasFilters
					? t(
							"exploreTryRemovingFilter",
							"Try removing a filter or searching for something broader.",
						)
					: t("tryADifferentSearchTerm", "Try a different search term.")}
			</p>
			{hasFilters && (
				<Button
					variant="outline"
					size="sm"
					className="rounded-lg"
					onClick={onClear}
				>
					{t("clearFilters", "Clear filters")}
				</Button>
			)}
		</div>
	);
}

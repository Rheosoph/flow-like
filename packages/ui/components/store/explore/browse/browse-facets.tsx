"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, ShieldCheck, SlidersHorizontal } from "lucide-react";
import type { ReactNode } from "react";
import { useAppCategoryLabel } from "../../../../lib/app-category";
import { categoryColor } from "../../../../lib/category-meta";
import type { WasmPackageCategory } from "../../../../lib/schema/wasm";
import { cn } from "../../../../lib/utils";
import { Button } from "../../../ui/button";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	SheetTrigger,
} from "../../../ui/sheet";
import { Skeleton } from "../../../ui/skeleton";
import type { ExploreBrowseParams } from "../explore-href";
import { useExploreLabels } from "../explore-labels";
import {
	EXPLORE_SEARCH_LIMITS,
	type ExplorePermissionFacet,
	type ExploreSearchFacets,
	type ExploreSearchType,
} from "../explore-types";

const PERMISSIONS: readonly ExplorePermissionFacet[] = [
	"none",
	"network",
	"models",
	"storage",
];

export function activeFilterCount(params: ExploreBrowseParams): number {
	return (
		(params.type && params.type !== "all" ? 1 : 0) +
		(params.categories?.length ?? 0) +
		(params.price ? 1 : 0) +
		(params.verified ? 1 : 0) +
		(params.permissions?.length ?? 0)
	);
}

function toggle<T>(list: readonly T[] | undefined, value: T): T[] | undefined {
	const current = list ?? [];
	const next = current.includes(value)
		? current.filter((entry) => entry !== value)
		: [...current, value];
	return next.length ? next : undefined;
}

export interface BrowseFacetsProps {
	facets?: ExploreSearchFacets;
	params: ExploreBrowseParams;
	/** Package content is on: the Packages type and the package-only groups show. */
	dev: boolean;
	onChange: (params: ExploreBrowseParams) => void;
}

function FacetGroup({
	title,
	hint,
	children,
}: Readonly<{ title: string; hint?: string; children: ReactNode }>) {
	return (
		<fieldset className="flex min-w-0 flex-col gap-0.5">
			<legend className="mb-1.5 flex w-full items-center justify-between gap-2 px-1.5 font-mono text-[11px] uppercase tracking-wider text-muted-foreground">
				<span>{title}</span>
				{hint && (
					<span className="normal-case tracking-normal text-muted-foreground/80">
						{hint}
					</span>
				)}
			</legend>
			{children}
		</fieldset>
	);
}

function FacetOption({
	label,
	count,
	selected,
	disabled,
	marker,
	onToggle,
}: Readonly<{
	label: string;
	count: number;
	selected: boolean;
	disabled?: boolean;
	marker?: ReactNode;
	onToggle: () => void;
}>) {
	return (
		<button
			type="button"
			aria-pressed={selected}
			disabled={disabled && !selected}
			onClick={onToggle}
			className="flex min-h-8 w-full min-w-0 items-center gap-2.5 rounded-md px-1.5 text-left text-[13px] transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-45 disabled:hover:bg-transparent"
		>
			<span
				aria-hidden="true"
				className={cn(
					"flex size-4 shrink-0 items-center justify-center rounded border",
					selected
						? "border-primary bg-primary text-primary-foreground"
						: "border-border bg-background",
				)}
			>
				{selected && <Check className="size-3" />}
			</span>
			{marker}
			<span className="min-w-0 flex-1 truncate">{label}</span>
			<span className="shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground">
				{count}
			</span>
		</button>
	);
}

function Dot({ color }: Readonly<{ color?: string }>) {
	return (
		<span
			aria-hidden="true"
			className={cn(
				"size-2 shrink-0 rounded-full",
				!color && "bg-muted-foreground/60",
			)}
			style={color ? { backgroundColor: color } : undefined}
		/>
	);
}

/** Facet groups with counts. Selections go into the URL unchanged; package-only groups need package content. */
export function BrowseFacetList({
	facets,
	params,
	dev,
	onChange,
}: Readonly<BrowseFacetsProps>) {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	const categoryLabel = useAppCategoryLabel();
	if (!facets) {
		return (
			<div aria-hidden="true" className="flex flex-col gap-2">
				{[0, 1, 2, 3, 4, 5].map((index) => (
					<Skeleton key={index} className="h-7 rounded-md" />
				))}
			</div>
		);
	}
	const set = (patch: Partial<ExploreBrowseParams>) =>
		onChange({ ...params, ...patch });
	const types: [ExploreSearchType, number][] = [
		["apps", facets.types.apps],
		...(dev
			? ([["packages", facets.types.packages]] as [ExploreSearchType, number][])
			: []),
		["collections", facets.types.collections],
	];
	const selectedCategories = params.categories ?? [];
	const atCategoryCap =
		selectedCategories.length >= EXPLORE_SEARCH_LIMITS.categories;
	const categories = facets.categories.filter(
		(entry) => dev || entry.kind === "app",
	);
	const packagesOnly = t("explorePackagesOnly", "packages only");

	return (
		<div className="flex flex-col gap-5">
			<FacetGroup title={t("exploreFacetType", "Type")}>
				{types.map(([type, count]) => (
					<FacetOption
						key={type}
						label={labels.type(type)}
						count={count}
						selected={params.type === type}
						disabled={count === 0}
						onToggle={() =>
							set({ type: params.type === type ? undefined : type })
						}
					/>
				))}
			</FacetGroup>
			{categories.length > 0 && (
				<FacetGroup title={t("exploreFacetCategory", "Category")}>
					{categories.map((entry) => {
						const [, name] = entry.value.split(":");
						const selected = selectedCategories.includes(entry.value);
						return (
							<FacetOption
								key={entry.value}
								label={
									entry.kind === "app"
										? categoryLabel(name)
										: labels.packageCategory(name as WasmPackageCategory)
								}
								count={entry.count}
								selected={selected}
								disabled={entry.count === 0 || atCategoryCap}
								marker={
									<Dot
										color={
											entry.kind === "app" ? categoryColor(name) : undefined
										}
									/>
								}
								onToggle={() =>
									set({ categories: toggle(params.categories, entry.value) })
								}
							/>
						);
					})}
				</FacetGroup>
			)}
			<FacetGroup title={t("exploreFacetPrice", "Price")}>
				<FacetOption
					label={t("free", "Free")}
					count={facets.price.free}
					selected={params.price === "free"}
					disabled={facets.price.free === 0}
					onToggle={() =>
						set({ price: params.price === "free" ? undefined : "free" })
					}
				/>
				<FacetOption
					label={t("exploreStatPaid", "Paid")}
					count={facets.price.paid}
					selected={params.price === "paid"}
					disabled={facets.price.paid === 0}
					onToggle={() =>
						set({ price: params.price === "paid" ? undefined : "paid" })
					}
				/>
			</FacetGroup>
			{dev && (
				<FacetGroup title={t("exploreFacetTrust", "Trust")} hint={packagesOnly}>
					<FacetOption
						label={t("exploreVerifiedPublisher", "Verified publisher")}
						count={facets.verified}
						selected={!!params.verified}
						disabled={facets.verified === 0}
						marker={
							<ShieldCheck
								aria-hidden="true"
								className="size-3.5 shrink-0 text-sky-500 dark:text-sky-400"
							/>
						}
						onToggle={() =>
							set({ verified: params.verified ? undefined : true })
						}
					/>
				</FacetGroup>
			)}
			{dev && (
				<FacetGroup title={t("permissions", "Permissions")} hint={packagesOnly}>
					{PERMISSIONS.map((permission) => (
						<FacetOption
							key={permission}
							label={labels.permission(permission)}
							count={facets.permissions[permission]}
							selected={params.permissions?.includes(permission) ?? false}
							disabled={facets.permissions[permission] === 0}
							onToggle={() =>
								set({ permissions: toggle(params.permissions, permission) })
							}
						/>
					))}
				</FacetGroup>
			)}
			{dev && (
				<p className="px-1.5 text-xs leading-relaxed text-muted-foreground">
					{t(
						"explorePermissionsNote",
						"Permissions come from each package's manifest and are checked when you install.",
					)}
				</p>
			)}
		</div>
	);
}

function FacetsHeader({
	count,
	onClear,
}: Readonly<{ count: number; onClear: () => void }>) {
	const { t } = useTranslation("store");
	return (
		<div className="flex items-center justify-between gap-2 px-1.5">
			<span className="flex items-center gap-2 text-sm font-semibold">
				<SlidersHorizontal aria-hidden="true" className="size-4" />
				{t("exploreFilters", "Filters")}
			</span>
			{count > 0 && (
				<button
					type="button"
					onClick={onClear}
					className="rounded-sm text-[13px] font-medium text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					{t("exploreClearAll", "Clear all")}
				</button>
			)}
		</div>
	);
}

type BrowseFacetsPanelProps = Readonly<
	BrowseFacetsProps & { onClear: () => void }
>;

/** Left column on wide containers. */
export function BrowseFacetsSidebar(props: BrowseFacetsPanelProps) {
	const { t } = useTranslation("store");
	return (
		<aside
			aria-label={t("exploreFilters", "Filters")}
			className="hidden w-60 shrink-0 flex-col gap-5 @5xl/explore:flex"
		>
			<FacetsHeader
				count={activeFilterCount(props.params)}
				onClear={props.onClear}
			/>
			<BrowseFacetList {...props} />
		</aside>
	);
}

/** "Filters" button and Sheet for narrow containers. */
export function BrowseFacetsSheet(props: BrowseFacetsPanelProps) {
	const { t } = useTranslation("store");
	const count = activeFilterCount(props.params);
	return (
		<Sheet>
			<SheetTrigger asChild>
				<Button
					variant="outline"
					size="sm"
					className="rounded-lg @5xl/explore:hidden"
				>
					<SlidersHorizontal aria-hidden="true" className="size-3.5" />
					{t("exploreFilters", "Filters")}
					{count > 0 && (
						<span className="rounded-full bg-primary px-1.5 font-mono text-[10px] text-primary-foreground">
							{count}
						</span>
					)}
				</Button>
			</SheetTrigger>
			<SheetContent side="left" className="w-80 overflow-y-auto">
				<SheetHeader>
					<SheetTitle>{t("exploreFilters", "Filters")}</SheetTitle>
					<SheetDescription className="sr-only">
						{t("exploreFiltersDescription", "Narrow the results")}
					</SheetDescription>
				</SheetHeader>
				<div className="flex flex-col gap-5 px-4 pb-6">
					<FacetsHeader count={count} onClear={props.onClear} />
					<BrowseFacetList {...props} />
				</div>
			</SheetContent>
		</Sheet>
	);
}

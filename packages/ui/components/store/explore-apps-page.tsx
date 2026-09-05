"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertCircle,
	ArrowRight,
	ArrowUpRight,
	Clock3,
	Compass,
	Loader2,
	PackageOpen,
	RotateCw,
	Search,
	TrendingUp,
	X,
} from "lucide-react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import {
	Suspense,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useInfiniteInvoke, useInvoke } from "../../hooks/use-invoke";
import {
	CATEGORY_TRANSLATION_KEYS,
	formatAppCategory,
} from "../../lib/app-category";
import { APP_CATEGORY_ORDER, categoryColor } from "../../lib/category-meta";
import type { IApp } from "../../lib/schema/app/app";
import {
	IAppCategory,
	IAppSearchSort,
} from "../../lib/schema/app/app-search-query";
import type { IMetadata } from "../../lib/schema/bit/bit-pack";
import { useBackend } from "../../state/backend-state";
import type { IEventMapping } from "../interfaces/interfaces";
import { CARD_MIN_W_DESKTOP } from "../library/library-types";
import { Alert, AlertDescription } from "../ui/alert";
import { AppCard } from "../ui/app-card";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { ScrollRail } from "../ui/scroll-rail";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { Skeleton } from "../ui/skeleton";
import { ExploreCategoryFilter } from "./explore-category-filter";
import { ExploreHubLayout } from "./explore-hub-layout";
import { SuitesRail } from "./suites";

type SortOption = "popular" | "newest" | "rated" | "updated";

const SORT_MAP: Record<SortOption, IAppSearchSort> = {
	popular: IAppSearchSort.MostPopular,
	newest: IAppSearchSort.NewestCreated,
	rated: IAppSearchSort.BestRated,
	updated: IAppSearchSort.NewestUpdated,
};

const SORT_LABEL: Record<SortOption, { key: string; defaultValue: string }> = {
	popular: { key: "mostPopular", defaultValue: "Most popular" },
	newest: { key: "newestFirst", defaultValue: "Newest first" },
	rated: { key: "bestRated", defaultValue: "Best rated" },
	updated: { key: "recentlyUpdated", defaultValue: "Recently updated" },
};

const SORT_OPTIONS = Object.keys(SORT_MAP) as SortOption[];

const isSortOption = (value: string | null): value is SortOption =>
	!!value && SORT_OPTIONS.includes(value as SortOption);

const isCategory = (value: string | null): value is IAppCategory =>
	!!value && (Object.values(IAppCategory) as string[]).includes(value);

// Rail order lookup keyed by the stable category enum. Display labels are
// translated separately and must never affect grouping or ordering.
const CATEGORY_LABEL_ORDER = new Map(
	APP_CATEGORY_ORDER.map((category, index) => [category, index]),
);

function normalizeCategory(category?: string | null): IAppCategory {
	const candidate = category ?? null;
	return isCategory(candidate) ? candidate : IAppCategory.Other;
}

type AppEntry = [IApp, IMetadata | undefined];

export interface ExploreAppsPageProps {
	eventConfig?: IEventMapping;
}

export function ExploreAppsPage(props: Readonly<ExploreAppsPageProps>) {
	return (
		<Suspense fallback={<ExploreAppsSkeleton />}>
			<ExploreAppsContent {...props} />
		</Suspense>
	);
}

function ExploreAppsContent({ eventConfig }: Readonly<ExploreAppsPageProps>) {
	const { t } = useTranslation("store");
	const router = useRouter();
	const pathname = usePathname();
	const searchParams = useSearchParams();
	const backend = useBackend();
	const resultsRef = useRef<HTMLDivElement>(null);
	const searchRef = useRef<HTMLInputElement>(null);

	const [searchQuery, setSearchQuery] = useState(
		() => searchParams.get("q") ?? "",
	);
	const [debouncedQuery, setDebouncedQuery] = useState(searchQuery.trim());
	const [selectedCategory, setSelectedCategory] = useState<
		IAppCategory | undefined
	>(() => {
		const param = searchParams.get("category");
		return isCategory(param) ? param : undefined;
	});
	const [sortKey, setSortKey] = useState<SortOption>(() => {
		const param = searchParams.get("sort");
		return isSortOption(param) ? param : "popular";
	});

	const userApps = useInvoke(backend.appState.getApps, backend.appState, []);

	useEffect(() => {
		const timeout = setTimeout(
			() => setDebouncedQuery(searchQuery.trim()),
			300,
		);
		return () => clearTimeout(timeout);
	}, [searchQuery]);

	// Filters live in the URL (?q=…&category=…&sort=…) so they are shareable and
	// deep-linkable. Two-way sync: URL changes we did not write ourselves
	// (sidebar click, FlowPilot navigation) are adopted into state; state
	// changes are written back via router.replace. The refs break the loop.
	const lastParamsRef = useRef(searchParams.toString());
	const adoptingRef = useRef(false);

	useEffect(() => {
		const current = searchParams.toString();
		if (current === lastParamsRef.current) return;
		lastParamsRef.current = current;
		adoptingRef.current = true;
		const q = searchParams.get("q") ?? "";
		const category = searchParams.get("category");
		const sort = searchParams.get("sort");
		setSearchQuery(q);
		setDebouncedQuery(q.trim());
		setSelectedCategory(isCategory(category) ? category : undefined);
		setSortKey(isSortOption(sort) ? sort : "popular");
	}, [searchParams]);

	useEffect(() => {
		if (adoptingRef.current) {
			adoptingRef.current = false;
			return;
		}
		const params = new URLSearchParams(searchParams.toString());
		params.delete("q");
		params.delete("category");
		params.delete("sort");
		if (debouncedQuery) params.set("q", debouncedQuery);
		if (selectedCategory) params.set("category", selectedCategory);
		if (sortKey !== "popular") params.set("sort", sortKey);
		const next = params.toString();
		if (next !== searchParams.toString()) {
			lastParamsRef.current = next;
			router.replace(next ? `${pathname}?${next}` : pathname, {
				scroll: false,
			});
		}
	}, [
		debouncedQuery,
		selectedCategory,
		sortKey,
		pathname,
		router,
		searchParams,
	]);

	const {
		data: searchResults,
		hasNextPage,
		fetchNextPage,
		isLoading,
		isFetching,
		isFetchNextPageError,
		error,
		refetch,
	} = useInfiniteInvoke(backend.appState.searchApps, backend.appState, [
		undefined,
		debouncedQuery || undefined,
		undefined,
		selectedCategory,
		undefined,
		SORT_MAP[sortKey],
		undefined,
	]);

	// Keep each app once when the order shifts between page requests.
	const combinedApps = useMemo(() => {
		const seen = new Set<string>();
		const deduped: AppEntry[] = [];
		for (const entry of searchResults?.pages.flat() ?? []) {
			if (seen.has(entry[0].id)) continue;
			seen.add(entry[0].id);
			deduped.push(entry);
		}
		return deduped;
	}, [searchResults]);

	const userAppIds = useMemo(
		() => new Set(userApps.data?.map(([app]) => app.id) ?? []),
		[userApps.data],
	);

	const usableEvents = useMemo(() => {
		const set = new Set<string>();
		for (const config of Object.values(eventConfig ?? {})) {
			const usable = Object.keys(config.useInterfaces);
			for (const eventType of usable) {
				if (config.eventTypes.includes(eventType)) set.add(eventType);
			}
		}
		return set;
	}, [eventConfig]);

	const resolveUseHref = useCallback(
		async (appId: string) => {
			if (!userAppIds.has(appId)) return null;

			const [routes, events] = await Promise.all([
				backend.routeState.getRoutes(appId, true).catch(() => []),
				backend.eventState.getEvents(appId, true).catch(() => []),
			]);
			const activeEvents = events.filter((event) => event.active);
			const activeEventsById = new Map(
				activeEvents.map((event) => [event.id, event] as const),
			);

			const hasUsableRoute = routes.some((route) => {
				const routeEvent = activeEventsById.get(route.eventId);
				return Boolean(
					routeEvent?.default_page_id ||
						(routeEvent && usableEvents.has(routeEvent.event_type)),
				);
			});
			if (hasUsableRoute) return `/use?id=${appId}`;

			const fallbackEvent = activeEvents.find(
				(event) => event.default_page_id || usableEvents.has(event.event_type),
			);
			if (!fallbackEvent) return null;

			return `/use?id=${appId}&eventId=${fallbackEvent.id}`;
		},
		[backend.eventState, backend.routeState, usableEvents, userAppIds],
	);

	const handleAppClick = useCallback(
		async (appId: string) => {
			const useHref = await resolveUseHref(appId);
			router.push(useHref ?? `/store?id=${appId}`);
		},
		[resolveUseHref, router],
	);

	const appHref = useCallback((appId: string) => `/store?id=${appId}`, []);

	const isFiltered =
		!!debouncedQuery || !!selectedCategory || sortKey !== "popular";
	const hasActiveFilters = isFiltered || !!searchQuery;
	const isSearching = searchQuery.trim() !== debouncedQuery || isLoading;

	// A new search starts at the top; pagination keeps the reader's position.
	// biome-ignore lint/correctness/useExhaustiveDependencies: Each filter change resets the results viewport.
	useEffect(() => {
		resultsRef.current?.scrollTo({ top: 0 });
	}, [debouncedQuery, selectedCategory, sortKey]);

	const clearFilters = useCallback(() => {
		setSearchQuery("");
		setDebouncedQuery("");
		setSelectedCategory(undefined);
		setSortKey("popular");
	}, []);

	const categoryRails = useMemo(() => {
		if (isFiltered) return null;
		const groups = new Map<IAppCategory, AppEntry[]>();
		for (const entry of combinedApps) {
			const category = normalizeCategory(entry[0].primary_category);
			const existing = groups.get(category) ?? [];
			existing.push(entry);
			groups.set(category, existing);
		}
		return Array.from(groups.entries())
			.map(([category, items]) => ({
				category,
				label: t(
					CATEGORY_TRANSLATION_KEYS[category],
					formatAppCategory(category),
				),
				items,
			}))
			.toSorted(
				(a, b) =>
					(CATEGORY_LABEL_ORDER.get(a.category) ?? Number.MAX_SAFE_INTEGER) -
					(CATEGORY_LABEL_ORDER.get(b.category) ?? Number.MAX_SAFE_INTEGER),
			);
	}, [combinedApps, isFiltered, t]);

	const displayCount = `${combinedApps.length}${hasNextPage ? "+" : ""}`;
	const selectedCategoryLabel = selectedCategory
		? t(
				CATEGORY_TRANSLATION_KEYS[selectedCategory],
				formatAppCategory(selectedCategory),
			)
		: undefined;
	const resultsSummary = selectedCategoryLabel
		? debouncedQuery
			? t("showingAppsInCategoryForQuery", {
					count: combinedApps.length,
					displayCount,
					category: selectedCategoryLabel,
					query: debouncedQuery,
					defaultValue_one:
						"Showing {{displayCount}} app in {{category}} for “{{query}}”",
					defaultValue_other:
						"Showing {{displayCount}} apps in {{category}} for “{{query}}”",
				})
			: t("showingAppsInCategory", {
					count: combinedApps.length,
					displayCount,
					category: selectedCategoryLabel,
					defaultValue_one: "Showing {{displayCount}} app in {{category}}",
					defaultValue_other: "Showing {{displayCount}} apps in {{category}}",
				})
		: debouncedQuery
			? t("showingAppsForQuery", {
					count: combinedApps.length,
					displayCount,
					query: debouncedQuery,
					defaultValue_one: "Showing {{displayCount}} app for “{{query}}”",
					defaultValue_other: "Showing {{displayCount}} apps for “{{query}}”",
				})
			: t("showingApps", {
					count: combinedApps.length,
					displayCount,
					defaultValue_one: "Showing {{displayCount}} app",
					defaultValue_other: "Showing {{displayCount}} apps",
				});

	return (
		<ExploreHubLayout
			active="apps"
			subtitle={t(
				"exploreAppsSubtitle",
				"Find community apps to use and make your own.",
			)}
			scrollRef={resultsRef}
			toolbar={
				<div className="flex flex-col gap-3 sm:flex-row sm:items-center">
					<div className="relative min-w-0 flex-1">
						{isSearching ? (
							<Loader2
								aria-hidden="true"
								className="pointer-events-none absolute left-4 top-1/2 h-5 w-5 -translate-y-1/2 animate-spin text-primary"
							/>
						) : (
							<Search
								aria-hidden="true"
								className="pointer-events-none absolute left-4 top-1/2 h-5 w-5 -translate-y-1/2 text-muted-foreground"
							/>
						)}
						<Input
							ref={searchRef}
							type="search"
							aria-label={t("searchCommunityApps", "Search community apps…")}
							aria-controls="explore-results"
							placeholder={t("searchCommunityApps", "Search community apps…")}
							value={searchQuery}
							onChange={(e) => setSearchQuery(e.target.value)}
							className="h-12 rounded-xl border-border/60 bg-muted/30 pr-12 pl-12 text-sm shadow-none transition-colors focus-visible:bg-background [&::-webkit-search-cancel-button]:appearance-none"
						/>
						{searchQuery && (
							<button
								type="button"
								aria-label={t("clearSearch", "Clear search")}
								onClick={() => {
									setSearchQuery("");
									setDebouncedQuery("");
									searchRef.current?.focus();
								}}
								className="absolute right-1 top-1/2 flex h-10 w-10 -translate-y-1/2 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
							>
								<X aria-hidden="true" className="h-4 w-4" />
							</button>
						)}
					</div>
					<div className="flex items-center justify-between gap-2 sm:justify-start">
						<Select
							value={sortKey}
							onValueChange={(value) => {
								if (isSortOption(value)) setSortKey(value);
							}}
						>
							<SelectTrigger
								aria-label={t("sortResults", "Sort results")}
								className="min-h-12 min-w-0 max-w-48 flex-1 gap-2 rounded-xl border-border/60 bg-background text-sm sm:w-48 sm:flex-none"
							>
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								{SORT_OPTIONS.map((option) => (
									<SelectItem key={option} value={option}>
										{t(SORT_LABEL[option].key, SORT_LABEL[option].defaultValue)}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
						<div className="w-28 shrink-0">
							<Button
								variant="ghost"
								size="sm"
								className={`min-h-12 w-full rounded-xl text-muted-foreground ${hasActiveFilters ? "" : "invisible"}`}
								disabled={!hasActiveFilters}
								onClick={clearFilters}
							>
								<X aria-hidden="true" className="mr-1 h-3.5 w-3.5" />
								{t("clearFilters", "Clear filters")}
							</Button>
						</div>
					</div>
				</div>
			}
			filters={
				<ExploreCategoryFilter
					selected={selectedCategory}
					onSelect={setSelectedCategory}
				/>
			}
		>
			<div
				id="explore-results"
				className="flex flex-col gap-8"
				aria-busy={isFetching}
			>
				<output className="sr-only" aria-live="polite">
					{isSearching
						? t("loadingApps", "Loading apps…")
						: error
							? t(
									"exploreLoadError",
									"Apps could not be loaded. Please try again.",
								)
							: resultsSummary}
				</output>
				{error && !isFetchNextPageError && (
					<Alert variant="destructive" className="rounded-xl">
						<AlertCircle className="h-4 w-4" />
						<AlertDescription className="flex flex-wrap items-center justify-between gap-3">
							<span>
								{t(
									"exploreLoadError",
									"Apps could not be loaded. Please try again.",
								)}
							</span>
							<Button
								variant="outline"
								size="sm"
								className="rounded-lg"
								disabled={isFetching}
								onClick={() => refetch()}
							>
								<RotateCw
									aria-hidden="true"
									className={`mr-1.5 h-3.5 w-3.5 ${isFetching ? "animate-spin" : ""}`}
								/>
								{t("retry", "Retry")}
							</Button>
						</AlertDescription>
					</Alert>
				)}
				{isLoading ? (
					<ResultsSkeleton discovery={!isFiltered} />
				) : combinedApps.length === 0 ? (
					!error && (
						<ExploreEmpty hasFilters={isFiltered} onClear={clearFilters} />
					)
				) : categoryRails ? (
					<>
						<section
							aria-labelledby="explore-popular-heading"
							className="space-y-4"
						>
							<div className="flex flex-wrap items-center justify-between gap-3">
								<div>
									<div className="mb-1 flex items-center gap-2 text-primary">
										<TrendingUp aria-hidden="true" className="h-4 w-4" />
										<span className="text-xs font-medium">
											{t("communityFavorites", "Community favorites")}
										</span>
									</div>
									<h2
										id="explore-popular-heading"
										className="text-xl font-semibold tracking-tight sm:text-2xl"
									>
										{t("popularRightNow", "Popular right now")}
									</h2>
								</div>
								<Button
									variant="outline"
									size="sm"
									className="h-10 gap-2 rounded-xl"
									onClick={() => setSortKey("newest")}
								>
									<Clock3 aria-hidden="true" className="h-4 w-4" />
									{t("newArrivals", "New arrivals")}
									<ArrowUpRight
										aria-hidden="true"
										className="h-3.5 w-3.5 text-muted-foreground"
									/>
								</Button>
							</div>
							<ScrollRail>
								{combinedApps.slice(0, 6).map(([app, metadata]) => (
									<div
										key={app.id}
										className="w-60 shrink-0 snap-start sm:w-72"
									>
										<ExploreAppCard
											isOwned={userAppIds.has(app.id)}
											app={app}
											metadata={metadata}
											variant="extended"
											onClick={() => handleAppClick(app.id)}
											href={appHref(app.id)}
											className="min-h-80 w-full rounded-2xl"
										/>
									</div>
								))}
							</ScrollRail>
						</section>
					</>
				) : (
					<section
						className="space-y-5"
						aria-labelledby="explore-results-heading"
					>
						<div className="space-y-1">
							<h2
								id="explore-results-heading"
								className="text-xl font-semibold tracking-tight"
							>
								{debouncedQuery
									? t("searchResults", "Search results")
									: (selectedCategoryLabel ??
										t(
											SORT_LABEL[sortKey].key,
											SORT_LABEL[sortKey].defaultValue,
										))}
							</h2>
							<p className="text-sm text-muted-foreground">{resultsSummary}</p>
						</div>
						<ExploreGrid
							apps={combinedApps}
							userAppIds={userAppIds}
							onAppClick={handleAppClick}
							appHref={appHref}
						/>
					</section>
				)}
				{!isLoading && combinedApps.length > 0 && categoryRails && (
					<div className="space-y-7 border-t border-border/50 pt-7">
						<div className="flex items-center gap-2 text-muted-foreground">
							<Compass aria-hidden="true" className="h-4 w-4" />
							<h2 className="text-sm font-medium">
								{t("exploreByCategory", "Explore by category")}
							</h2>
						</div>
						{categoryRails.map(({ category, label, items }) => (
							<CategoryRailSection
								key={category}
								category={category}
								label={label}
								apps={items}
								userAppIds={userAppIds}
								onAppClick={handleAppClick}
								appHref={appHref}
								onSeeAll={() => setSelectedCategory(category)}
							/>
						))}
					</div>
				)}
				{isFetchNextPageError && (
					<Alert variant="destructive" className="rounded-xl">
						<AlertCircle className="h-4 w-4" />
						<AlertDescription className="flex flex-wrap items-center justify-between gap-3">
							<span>
								{t(
									"exploreLoadMoreError",
									"More apps could not be loaded. Your results are still here.",
								)}
							</span>
							<Button
								variant="outline"
								size="sm"
								className="rounded-lg"
								disabled={isFetching}
								onClick={() => fetchNextPage()}
							>
								<RotateCw
									aria-hidden="true"
									className={`mr-1.5 h-3.5 w-3.5 ${isFetching ? "animate-spin" : ""}`}
								/>
								{t("retry", "Retry")}
							</Button>
						</AlertDescription>
					</Alert>
				)}
				{hasNextPage && combinedApps.length > 0 && !isFetchNextPageError && (
					<LoadMoreButton isFetching={isFetching} onFetch={fetchNextPage} />
				)}
				{!isFiltered && <SuitesRail />}
			</div>
		</ExploreHubLayout>
	);
}

function ExploreAppCard({
	app,
	metadata,
	isOwned,
	variant,
	className,
	href,
	onClick,
}: Readonly<{
	app: IApp;
	metadata?: IMetadata;
	isOwned: boolean;
	variant: "extended" | "small";
	className?: string;
	href: string;
	onClick: () => void;
}>) {
	return (
		<Link
			href={href}
			aria-label={metadata?.name || app.id}
			className="block h-full min-w-0 rounded-2xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
			onClick={(event) => {
				if (
					event.button !== 0 ||
					event.metaKey ||
					event.ctrlKey ||
					event.shiftKey ||
					event.altKey
				)
					return;
				event.preventDefault();
				onClick();
			}}
		>
			<AppCard
				app={app}
				metadata={metadata}
				isOwned={isOwned}
				variant={variant}
				className={className}
				href={href}
			/>
		</Link>
	);
}

function CategoryRailSection({
	category,
	label,
	apps,
	userAppIds,
	onAppClick,
	appHref,
	onSeeAll,
}: Readonly<{
	category: IAppCategory;
	label: string;
	apps: AppEntry[];
	userAppIds: Set<string>;
	onAppClick: (id: string) => void;
	appHref: (id: string) => string;
	onSeeAll: () => void;
}>) {
	const { t } = useTranslation("store");
	if (apps.length === 0) return null;
	const color = categoryColor(category);

	return (
		<section aria-label={label}>
			<div className="mb-3 flex items-center justify-between gap-3">
				<div className="flex items-center gap-2 min-w-0">
					<span
						className="h-2 w-2 shrink-0 rounded-full"
						style={{ backgroundColor: color }}
					/>
					<h3 className="truncate text-base font-semibold tracking-tight text-foreground">
						{label}
					</h3>
				</div>
				<button
					type="button"
					onClick={onSeeAll}
					aria-label={t("seeAllCategoryApps", {
						category: label,
						defaultValue: "See all {{category}} apps",
					})}
					className="group/link flex min-h-10 shrink-0 items-center gap-1 rounded-lg px-2 text-sm font-medium text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					{t("seeAll", "See all")}
					<ArrowRight className="h-3.5 w-3.5 transition-transform group-hover/link:translate-x-0.5" />
				</button>
			</div>
			<ScrollRail>
				{apps.map(([app, metadata]) => (
					<div key={app.id} className="w-56 shrink-0 snap-start md:w-64">
						<ExploreAppCard
							isOwned={userAppIds.has(app.id)}
							app={app}
							metadata={metadata}
							variant="extended"
							onClick={() => onAppClick(app.id)}
							href={appHref(app.id)}
							className="min-h-72 w-full rounded-2xl"
						/>
					</div>
				))}
			</ScrollRail>
		</section>
	);
}

function ExploreGrid({
	apps,
	userAppIds,
	onAppClick,
	appHref,
}: Readonly<{
	apps: AppEntry[];
	userAppIds: Set<string>;
	onAppClick: (id: string) => void;
	appHref: (id: string) => string;
}>) {
	return (
		<>
			<div className="divide-y divide-border/30 md:hidden">
				{apps.map(([app, metadata]) => (
					<ExploreAppCard
						key={app.id}
						isOwned={userAppIds.has(app.id)}
						app={app}
						metadata={metadata}
						variant="small"
						onClick={() => onAppClick(app.id)}
						href={appHref(app.id)}
						className="w-full rounded-none border-0 shadow-none bg-transparent"
					/>
				))}
			</div>
			<div
				className="hidden gap-3 md:grid"
				style={{
					gridTemplateColumns: `repeat(auto-fill, minmax(min(100%, ${CARD_MIN_W_DESKTOP}px), 1fr))`,
				}}
			>
				{apps.map(([app, metadata]) => (
					<ExploreAppCard
						key={app.id}
						isOwned={userAppIds.has(app.id)}
						app={app}
						metadata={metadata}
						variant="extended"
						onClick={() => onAppClick(app.id)}
						href={appHref(app.id)}
						className="min-h-72 w-full rounded-2xl"
					/>
				))}
			</div>
		</>
	);
}

function ExploreEmpty({
	hasFilters,
	onClear,
}: Readonly<{ hasFilters: boolean; onClear: () => void }>) {
	const { t } = useTranslation("store");
	return (
		<div className="flex flex-col items-center justify-center rounded-2xl border border-dashed border-border bg-muted/10 px-6 py-16 text-center sm:py-24">
			<div className="rounded-full bg-muted/30 p-5 mb-5">
				<PackageOpen className="h-7 w-7 text-muted-foreground" />
			</div>
			<p className="text-lg font-semibold text-foreground mb-2">
				{hasFilters
					? t("noAppsMatchYourFilters", "No apps match your filters")
					: t("noAppsFound", "No apps found")}
			</p>
			<p className="max-w-sm text-sm text-muted-foreground mb-5">
				{hasFilters
					? t(
							"tryAdjustingYourSearchOrFilters",
							"Try adjusting your search or filters",
						)
					: t(
							"checkBackLaterForNewCommunityApps",
							"Check back later for new community apps",
						)}
			</p>
			{hasFilters && (
				<Button
					variant="outline"
					size="sm"
					className="rounded-full"
					onClick={onClear}
				>
					{t("clearFilters", "Clear filters")}
				</Button>
			)}
		</div>
	);
}

function LoadMoreButton({
	isFetching,
	onFetch,
}: Readonly<{
	isFetching: boolean;
	onFetch: () => void;
}>) {
	const { t } = useTranslation("store");
	return (
		<div className="flex justify-center mt-3">
			<button
				type="button"
				onClick={onFetch}
				disabled={isFetching}
				className="flex min-h-11 items-center gap-2 rounded-xl border border-border/60 px-6 py-2.5 text-sm font-medium text-muted-foreground transition-colors hover:border-border hover:bg-muted/30 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
			>
				{isFetching ? (
					<>
						<RotateCw className="h-3 w-3 animate-spin" />
						{t("loading", "Loading…")}
					</>
				) : (
					t("loadMore", "Load more")
				)}
			</button>
		</div>
	);
}

const SKELETON_KEYS = [
	"card-a",
	"card-b",
	"card-c",
	"card-d",
	"card-e",
	"card-f",
];

function ResultsSkeleton({
	discovery = true,
}: Readonly<{ discovery?: boolean }>) {
	return (
		<div aria-hidden="true" className={discovery ? "space-y-4" : "space-y-5"}>
			<div className="space-y-1">
				<Skeleton
					className={discovery ? "h-4 w-32 rounded" : "h-7 w-40 rounded"}
				/>
				<Skeleton
					className={discovery ? "h-7 w-48 rounded sm:h-8" : "h-5 w-56 rounded"}
				/>
			</div>
			{discovery ? (
				<div className="flex gap-4 overflow-hidden pb-1">
					{SKELETON_KEYS.map((key) => (
						<Skeleton
							key={key}
							className="h-80 w-60 shrink-0 rounded-2xl sm:w-72"
						/>
					))}
				</div>
			) : (
				<>
					<div className="divide-y divide-border/30 md:hidden">
						{SKELETON_KEYS.map((key) => (
							<div key={key} className="flex h-[69px] items-center gap-3 p-3">
								<Skeleton className="h-11 w-11 shrink-0 rounded-xl" />
								<div className="flex-1 space-y-2">
									<Skeleton className="h-4 w-28" />
									<Skeleton className="h-3 w-full" />
								</div>
							</div>
						))}
					</div>
					<div
						className="hidden gap-3 md:grid"
						style={{
							gridTemplateColumns: `repeat(auto-fill, minmax(min(100%, ${CARD_MIN_W_DESKTOP}px), 1fr))`,
						}}
					>
						{SKELETON_KEYS.map((key) => (
							<Skeleton key={key} className="h-72 rounded-2xl" />
						))}
					</div>
				</>
			)}
		</div>
	);
}

function ExploreAppsSkeleton() {
	const { t } = useTranslation("store");
	return (
		<ExploreHubLayout
			active="apps"
			subtitle={t(
				"exploreAppsSubtitle",
				"Find community apps to use and make your own.",
			)}
			toolbar={
				<div className="flex flex-col gap-3 sm:flex-row">
					<Skeleton className="h-12 flex-1 rounded-xl" />
					<div className="flex gap-2">
						<Skeleton className="h-12 min-w-0 flex-1 rounded-xl sm:w-48 sm:flex-none" />
						<div className="w-28" />
					</div>
				</div>
			}
			filters={<ExploreCategoryFilter onSelect={() => {}} />}
		>
			<ResultsSkeleton />
		</ExploreHubLayout>
	);
}

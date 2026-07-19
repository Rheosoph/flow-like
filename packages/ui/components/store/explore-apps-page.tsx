"use client";

import {
	AlertCircle,
	ArrowRight,
	PackageOpen,
	RotateCw,
	Search,
	X,
} from "lucide-react";
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
import { useIsMobile } from "../../hooks/use-mobile";
import { formatAppCategory } from "../../lib/app-category";
import {
	APP_CATEGORY_ORDER,
	CATEGORY_ICONS,
	categoryColor,
} from "../../lib/category-meta";
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
import { ExploreHubHeader } from "./explore-hub-header";
import { SuitesRail } from "./suites";

type SortOption = "popular" | "newest" | "rated" | "updated";

const SORT_MAP: Record<SortOption, IAppSearchSort> = {
	popular: IAppSearchSort.MostPopular,
	newest: IAppSearchSort.NewestCreated,
	rated: IAppSearchSort.BestRated,
	updated: IAppSearchSort.NewestUpdated,
};

const SORT_LABEL: Record<SortOption, string> = {
	popular: "Most popular",
	newest: "Newest first",
	rated: "Best rated",
	updated: "Recently updated",
};

const SORT_OPTIONS = Object.keys(SORT_MAP) as SortOption[];

const isSortOption = (value: string | null): value is SortOption =>
	!!value && value in SORT_MAP;

const isCategory = (value: string | null): value is IAppCategory =>
	!!value && (Object.values(IAppCategory) as string[]).includes(value);

// Rail order lookup keyed by formatted label (the grouping key).
const CATEGORY_LABEL_ORDER = new Map(
	APP_CATEGORY_ORDER.map((category, index) => [
		formatAppCategory(category),
		index,
	]),
);

type AppEntry = [IApp, IMetadata | undefined];

export interface ExploreAppsPageProps {
	eventConfig?: IEventMapping;
}

export function ExploreAppsPage(props: Readonly<ExploreAppsPageProps>) {
	return (
		<Suspense
			fallback={
				<main className="flex flex-col w-full flex-1 min-h-0">
					<ResultsSkeleton className="px-4 sm:px-8 pt-6" />
				</main>
			}
		>
			<ExploreAppsContent {...props} />
		</Suspense>
	);
}

function ExploreAppsContent({ eventConfig }: Readonly<ExploreAppsPageProps>) {
	const router = useRouter();
	const pathname = usePathname();
	const searchParams = useSearchParams();
	const backend = useBackend();
	const isMobile = useIsMobile();

	const [searchQuery, setSearchQuery] = useState(
		() => searchParams.get("q") ?? "",
	);
	const [debouncedQuery, setDebouncedQuery] = useState(searchQuery);
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
		const timeout = setTimeout(() => setDebouncedQuery(searchQuery), 300);
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
		setDebouncedQuery(q);
		setSelectedCategory(isCategory(category) ? category : undefined);
		setSortKey(isSortOption(sort) ? sort : "popular");
	}, [searchParams]);

	useEffect(() => {
		if (adoptingRef.current) {
			adoptingRef.current = false;
			return;
		}
		const params = new URLSearchParams();
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
		isFetchingNextPage,
		isLoading,
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

	// Offset pagination can hand the same app back across page boundaries when
	// the underlying order shifts between fetches — dedupe by id.
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
			if (!userAppIds.has(appId) || usableEvents.size === 0) return null;

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

	const clearFilters = useCallback(() => {
		setSearchQuery("");
		setDebouncedQuery("");
		setSelectedCategory(undefined);
		setSortKey("popular");
	}, []);

	const categoryRails = useMemo(() => {
		if (isFiltered) return null;
		const groups = new Map<string, AppEntry[]>();
		for (const entry of combinedApps) {
			const label = formatAppCategory(entry[0].primary_category);
			const existing = groups.get(label) ?? [];
			existing.push(entry);
			groups.set(label, existing);
		}
		return Array.from(groups.entries())
			.map(([label, items]) => ({ label, items }))
			.toSorted(
				(a, b) =>
					(CATEGORY_LABEL_ORDER.get(a.label) ?? Number.MAX_SAFE_INTEGER) -
					(CATEGORY_LABEL_ORDER.get(b.label) ?? Number.MAX_SAFE_INTEGER),
			);
	}, [combinedApps, isFiltered]);

	return (
		<main className="flex flex-col w-full flex-1 min-h-0">
			<div
				className={`pt-6 pb-4 space-y-4 ${isMobile ? "px-4" : "px-4 sm:px-8"}`}
			>
				<ExploreHubHeader
					active="apps"
					subtitle="Community apps, ready to use or fork."
				/>

				<div className="flex items-center gap-2">
					<div className="relative flex-1 max-w-lg">
						<Search className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground/40 pointer-events-none" />
						<Input
							placeholder="Search community apps…"
							value={searchQuery}
							onChange={(e) => setSearchQuery(e.target.value)}
							className="pl-11 h-10 rounded-full bg-muted/30 border-transparent focus:border-border/40 focus:bg-muted/50 transition-all text-sm"
						/>
						{searchQuery && (
							<button
								type="button"
								aria-label="Clear search"
								onClick={() => setSearchQuery("")}
								className="absolute right-4 top-1/2 -translate-y-1/2 text-muted-foreground/40 hover:text-foreground transition-colors"
							>
								<X className="h-4 w-4" />
							</button>
						)}
					</div>

					<Select
						value={sortKey}
						onValueChange={(value) => setSortKey(value as SortOption)}
					>
						<SelectTrigger
							aria-label="Sort results"
							className="w-auto gap-1.5 rounded-full border-border/40 bg-muted/30 text-sm h-10"
						>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{SORT_OPTIONS.map((option) => (
								<SelectItem key={option} value={option}>
									{SORT_LABEL[option]}
								</SelectItem>
							))}
						</SelectContent>
					</Select>

					{isFiltered && (
						<Button
							variant="ghost"
							size="sm"
							className="rounded-full text-muted-foreground/70 hover:text-foreground"
							onClick={clearFilters}
						>
							<X className="h-3.5 w-3.5 mr-1" />
							Clear
						</Button>
					)}
				</div>

				<CategoryChips
					selected={selectedCategory}
					onSelect={setSelectedCategory}
					isMobile={isMobile}
				/>
			</div>

			<div
				className={`flex-1 overflow-auto pb-10 ${isMobile ? "px-4" : "px-4 sm:px-8"}`}
			>
				{!isFiltered && <SuitesRail />}
				{error ? (
					<Alert variant="destructive" className="mb-4">
						<AlertCircle className="h-4 w-4" />
						<AlertDescription className="flex items-center gap-3">
							Failed to load apps: {error.message}
							<Button
								variant="outline"
								size="sm"
								className="rounded-full"
								onClick={() => refetch()}
							>
								<RotateCw className="h-3.5 w-3.5 mr-1" />
								Retry
							</Button>
						</AlertDescription>
					</Alert>
				) : isLoading ? (
					<ResultsSkeleton />
				) : combinedApps.length === 0 ? (
					<ExploreEmpty hasFilters={isFiltered} onClear={clearFilters} />
				) : categoryRails ? (
					<div className="space-y-10">
						{categoryRails.map(({ label, items }) => (
							<CategoryRailSection
								key={label}
								label={label}
								apps={items}
								userAppIds={userAppIds}
								onAppClick={handleAppClick}
								appHref={appHref}
								isMobile={isMobile}
								onSeeAll={() => {
									const match = APP_CATEGORY_ORDER.find(
										(category) => formatAppCategory(category) === label,
									);
									if (match) setSelectedCategory(match);
								}}
							/>
						))}

						{hasNextPage && (
							<LoadMoreButton
								isFetching={isFetchingNextPage}
								onFetch={fetchNextPage}
							/>
						)}
					</div>
				) : (
					<div className={isMobile ? "space-y-5" : "space-y-6"}>
						<p className="text-xs text-muted-foreground/60">
							Showing {combinedApps.length}
							{hasNextPage ? "+" : ""} app
							{combinedApps.length !== 1 ? "s" : ""}
							{selectedCategory && ` in ${formatAppCategory(selectedCategory)}`}
							{debouncedQuery && ` for “${debouncedQuery}”`}
						</p>

						<ExploreGrid
							apps={combinedApps}
							userAppIds={userAppIds}
							onAppClick={handleAppClick}
							appHref={appHref}
							isMobile={isMobile}
						/>

						{hasNextPage && (
							<LoadMoreButton
								isFetching={isFetchingNextPage}
								onFetch={fetchNextPage}
							/>
						)}
					</div>
				)}
			</div>
		</main>
	);
}

function CategoryChips({
	selected,
	onSelect,
	isMobile,
}: Readonly<{
	selected?: IAppCategory;
	onSelect: (category: IAppCategory | undefined) => void;
	isMobile: boolean;
}>) {
	return (
		<div
			className={
				isMobile
					? "flex gap-1.5 overflow-x-auto scrollbar-hide -mx-4 px-4 pb-1"
					: "flex flex-wrap gap-1.5"
			}
		>
			{APP_CATEGORY_ORDER.map((category) => {
				const label = formatAppCategory(category);
				const color = categoryColor(category);
				const Icon = CATEGORY_ICONS[category];
				const isSelected = selected === category;

				return (
					<button
						key={category}
						type="button"
						aria-pressed={isSelected}
						className={`flex shrink-0 items-center gap-1.5 rounded-full px-3 py-1.5 text-xs transition-all ${
							isSelected
								? "bg-foreground/10 text-foreground ring-1 ring-foreground/20"
								: "bg-muted/20 text-muted-foreground/70 hover:bg-muted/40 hover:text-foreground/80"
						}`}
						onClick={() => onSelect(isSelected ? undefined : category)}
					>
						<Icon
							className="h-3 w-3 shrink-0"
							style={{ color, opacity: isSelected ? 1 : 0.7 }}
						/>
						{label}
						{isSelected && <X className="h-3 w-3" />}
					</button>
				);
			})}
		</div>
	);
}

function CategoryRailSection({
	label,
	apps,
	userAppIds,
	onAppClick,
	appHref,
	isMobile,
	onSeeAll,
}: Readonly<{
	label: string;
	apps: AppEntry[];
	userAppIds: Set<string>;
	onAppClick: (id: string) => void;
	appHref: (id: string) => string;
	isMobile: boolean;
	onSeeAll: () => void;
}>) {
	if (apps.length === 0) return null;
	const color = categoryColor(label);

	return (
		<section>
			<div className="mb-3 flex items-center justify-between gap-3">
				<div className="flex items-center gap-2 min-w-0">
					<span
						className="h-2 w-2 shrink-0 rounded-full"
						style={{ backgroundColor: color }}
					/>
					<h2 className="truncate text-base font-bold tracking-tight text-foreground">
						{label}
					</h2>
				</div>
				<button
					type="button"
					onClick={onSeeAll}
					className="group/link flex shrink-0 items-center gap-1 text-sm font-medium text-muted-foreground transition-colors hover:text-foreground"
				>
					See all
					<ArrowRight className="h-3.5 w-3.5 transition-transform group-hover/link:translate-x-0.5" />
				</button>
			</div>
			<ScrollRail>
				{apps.map(([app, metadata]) => (
					<div
						key={app.id}
						className={`shrink-0 snap-start ${isMobile ? "w-56" : "w-64"}`}
					>
						<AppCard
							isOwned={userAppIds.has(app.id)}
							app={app}
							metadata={metadata}
							variant="extended"
							onClick={() => onAppClick(app.id)}
							href={appHref(app.id)}
							className="w-full"
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
	isMobile,
}: Readonly<{
	apps: AppEntry[];
	userAppIds: Set<string>;
	onAppClick: (id: string) => void;
	appHref: (id: string) => string;
	isMobile: boolean;
}>) {
	if (isMobile) {
		return (
			<div className="divide-y divide-border/30">
				{apps.map(([app, metadata]) => (
					<AppCard
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
		);
	}

	return (
		<div
			className="grid gap-3"
			style={{
				gridTemplateColumns: `repeat(auto-fill, minmax(${CARD_MIN_W_DESKTOP}px, 1fr))`,
			}}
		>
			{apps.map(([app, metadata]) => (
				<AppCard
					key={app.id}
					isOwned={userAppIds.has(app.id)}
					app={app}
					metadata={metadata}
					variant="extended"
					onClick={() => onAppClick(app.id)}
					href={appHref(app.id)}
					className="w-full"
				/>
			))}
		</div>
	);
}

function ExploreEmpty({
	hasFilters,
	onClear,
}: Readonly<{ hasFilters: boolean; onClear: () => void }>) {
	return (
		<div className="flex flex-col items-center justify-center py-32 text-center">
			<div className="rounded-full bg-muted/30 p-5 mb-5">
				<PackageOpen className="h-7 w-7 text-muted-foreground/40" />
			</div>
			<p className="text-sm text-foreground/60 mb-1">
				{hasFilters ? "No apps match your filters" : "No apps found"}
			</p>
			<p className="text-xs text-muted-foreground/60 mb-4">
				{hasFilters
					? "Try adjusting your search or filters"
					: "Check back later for new community apps"}
			</p>
			{hasFilters && (
				<Button
					variant="outline"
					size="sm"
					className="rounded-full"
					onClick={onClear}
				>
					Clear filters
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
	return (
		<div className="flex justify-center mt-3">
			<button
				type="button"
				onClick={onFetch}
				disabled={isFetching}
				className="flex items-center gap-1.5 text-xs text-muted-foreground/60 hover:text-foreground px-4 py-1.5 rounded-full border border-border/30 hover:border-border/50 hover:bg-muted/30 transition-colors disabled:opacity-50"
			>
				{isFetching ? (
					<>
						<RotateCw className="h-3 w-3 animate-spin" />
						Loading…
					</>
				) : (
					"Load more"
				)}
			</button>
		</div>
	);
}

const SKELETON_KEYS = {
	rows: ["row-a", "row-b"],
	cards: ["card-a", "card-b", "card-c", "card-d", "card-e"],
};

function ResultsSkeleton({ className }: Readonly<{ className?: string }>) {
	return (
		<div className={`space-y-10 ${className ?? ""}`}>
			{SKELETON_KEYS.rows.map((row) => (
				<div key={row} className="space-y-3">
					<Skeleton className="h-5 w-32 rounded" />
					<div
						className="grid gap-3"
						style={{
							gridTemplateColumns: `repeat(auto-fill, minmax(${CARD_MIN_W_DESKTOP}px, 1fr))`,
						}}
					>
						{SKELETON_KEYS.cards.map((card) => (
							<Skeleton
								key={`${row}-${card}`}
								className="h-[375px] rounded-xl"
							/>
						))}
					</div>
				</div>
			))}
		</div>
	);
}

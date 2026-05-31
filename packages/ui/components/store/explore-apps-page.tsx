"use client";

import {
	AlertCircle,
	ArrowDownAZ,
	ChevronDown,
	Clock,
	Filter,
	Loader2,
	Package,
	Search,
	TrendingUp,
	X,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useInfiniteInvoke, useInvoke } from "../../hooks/use-invoke";
import { useIsMobile } from "../../hooks/use-mobile";
import { formatAppCategory } from "../../lib/app-category";
import type { IApp } from "../../lib/schema/app/app";
import {
	IAppCategory,
	IAppSearchSort,
} from "../../lib/schema/app/app-search-query";
import type { IMetadata } from "../../lib/schema/bit/bit-pack";
import { useBackend } from "../../state/backend-state";
import type { IEventMapping } from "../interfaces/interfaces";
import {
	CARD_MIN_W_DESKTOP,
	CARD_MIN_W_MOBILE,
	CATEGORY_COLORS,
} from "../library/library-types";
import { useGridColumns } from "../library/use-grid-columns";
import { Alert, AlertDescription } from "../ui/alert";
import { AppCard } from "../ui/app-card";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Skeleton } from "../ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";

const FEATURED_CATEGORIES = [
	IAppCategory.Productivity,
	IAppCategory.Business,
	IAppCategory.Education,
	IAppCategory.Entertainment,
	IAppCategory.Games,
	IAppCategory.Communication,
	IAppCategory.Utilities,
	IAppCategory.Music,
	IAppCategory.Health,
];

type SortOption = "popular" | "newest" | "rated" | "updated";

const SORT_MAP: Record<SortOption, IAppSearchSort> = {
	popular: IAppSearchSort.MostPopular,
	newest: IAppSearchSort.NewestCreated,
	rated: IAppSearchSort.BestRated,
	updated: IAppSearchSort.NewestUpdated,
};

const SORT_CYCLE: SortOption[] = ["popular", "newest", "rated", "updated"];
const SORT_LABEL: Record<SortOption, string> = {
	popular: "Most popular",
	newest: "Newest first",
	rated: "Best rated",
	updated: "Recently updated",
};
const SORT_ICON: Record<
	SortOption,
	React.ComponentType<{ className?: string }>
> = {
	popular: TrendingUp,
	newest: Clock,
	rated: ArrowDownAZ,
	updated: Clock,
};

export interface ExploreAppsPageProps {
	eventConfig?: IEventMapping;
}

export function ExploreAppsPage({ eventConfig }: Readonly<ExploreAppsPageProps>) {
	const router = useRouter();
	const backend = useBackend();
	const isMobile = useIsMobile();

	const [searchQuery, setSearchQuery] = useState("");
	const [debouncedQuery, setDebouncedQuery] = useState("");
	const [selectedCategory, setSelectedCategory] = useState<
		IAppCategory | undefined
	>();
	const [sortKey, setSortKey] = useState<SortOption>("popular");
	const [showCategories, setShowCategories] = useState(false);

	const userApps = useInvoke(backend.appState.getApps, backend.appState, []);

	useEffect(() => {
		const timeout = setTimeout(() => setDebouncedQuery(searchQuery), 300);
		return () => clearTimeout(timeout);
	}, [searchQuery]);

	const {
		data: searchResults,
		hasNextPage,
		fetchNextPage,
		isFetchingNextPage,
		isLoading,
		error,
	} = useInfiniteInvoke(backend.appState.searchApps, backend.appState, [
		undefined,
		debouncedQuery || undefined,
		undefined,
		selectedCategory,
		undefined,
		SORT_MAP[sortKey],
		undefined,
	]);

	const combinedApps = useMemo(
		() => searchResults?.pages.flat() ?? [],
		[searchResults],
	);

	const userAppIds = useMemo(
		() => new Set(userApps.data?.map(([app]) => app.id) ?? []),
		[userApps.data],
	);

	const usableEvents = useMemo(() => {
		const set = new Set<string>();
		Object.values(eventConfig ?? {}).forEach((config) => {
			const usable = Object.keys(config.useInterfaces);
			for (const eventType of usable) {
				if (config.eventTypes.includes(eventType)) set.add(eventType);
			}
		});
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

	const appHref = useCallback(
		(appId: string) => `/store?id=${appId}`,
		[],
	);

	const hasActiveFilters =
		!!debouncedQuery || !!selectedCategory || sortKey !== "popular";

	const clearFilters = useCallback(() => {
		setSearchQuery("");
		setSelectedCategory(undefined);
		setSortKey("popular");
	}, []);

	const cycleSortMode = useCallback(() => {
		setSortKey((prev) => {
			const idx = SORT_CYCLE.indexOf(prev);
			return SORT_CYCLE[(idx + 1) % SORT_CYCLE.length];
		});
	}, []);

	const categorizedApps = useMemo(() => {
		if (selectedCategory || debouncedQuery) return null;
		const groups = new Map<string, [IApp, IMetadata | undefined][]>();
		for (const entry of combinedApps) {
			const [app] = entry;
			const label = formatAppCategory(app.primary_category);
			const existing = groups.get(label) ?? [];
			existing.push(entry);
			groups.set(label, existing);
		}
		return Array.from(groups.entries())
			.map(([label, items]) => ({ label, items }))
			.toSorted((a, b) => a.label.localeCompare(b.label));
	}, [combinedApps, selectedCategory, debouncedQuery]);

	const SortIcon = SORT_ICON[sortKey];

	if (isLoading) {
		return (
			<main className="flex flex-col w-full flex-1 min-h-0">
				<ExploreSkeleton />
			</main>
		);
	}

	return (
		<main className="flex flex-col w-full flex-1 min-h-0">
			<div
				className={`pt-5 pb-3 space-y-3 ${isMobile ? "px-4" : "px-4 sm:px-8 pb-4"}`}
			>
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
								onClick={() => setSearchQuery("")}
								className="absolute right-4 top-1/2 -translate-y-1/2 text-muted-foreground/40 hover:text-foreground transition-colors"
							>
								<X className="h-4 w-4" />
							</button>
						)}
					</div>

					<div className="flex items-center gap-1">
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className={`h-8 w-8 rounded-full ${
										sortKey !== "popular"
											? "text-foreground/80 bg-muted/40"
											: "text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
									}`}
									onClick={cycleSortMode}
								>
									<SortIcon className="h-4 w-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>
								{SORT_LABEL[sortKey]} · click to change
							</TooltipContent>
						</Tooltip>

						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant={showCategories ? "secondary" : "ghost"}
									size="icon"
									className={`h-8 w-8 rounded-full ${
										showCategories || selectedCategory
											? "text-primary bg-primary/10"
											: "text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
									}`}
									onClick={() => setShowCategories((v) => !v)}
								>
									<Filter className="h-4 w-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>
								{showCategories ? "Hide categories" : "Filter by category"}
							</TooltipContent>
						</Tooltip>

						{hasActiveFilters && (
							<Tooltip>
								<TooltipTrigger asChild>
									<Button
										variant="ghost"
										size="icon"
										className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
										onClick={clearFilters}
									>
										<X className="h-4 w-4" />
									</Button>
								</TooltipTrigger>
								<TooltipContent>Clear all filters</TooltipContent>
							</Tooltip>
						)}
					</div>
				</div>

				{showCategories && (
					<div className="flex flex-wrap gap-1.5 animate-in fade-in-0 slide-in-from-top-2 duration-200">
						{FEATURED_CATEGORIES.map((category) => {
							const label = formatAppCategory(category);
							const color = CATEGORY_COLORS[label] ?? CATEGORY_COLORS.Other;
							const isSelected = selectedCategory === category;

							return (
								<button
									key={category}
									type="button"
									className={`rounded-full px-3 py-1 text-xs transition-all flex items-center gap-1.5 ${
										isSelected
											? "bg-foreground/10 text-foreground ring-1 ring-foreground/20"
											: "bg-muted/20 text-muted-foreground/70 hover:bg-muted/40 hover:text-foreground/80"
									}`}
									onClick={() =>
										setSelectedCategory(isSelected ? undefined : category)
									}
								>
									<span
										className="w-1.5 h-1.5 rounded-full shrink-0"
										style={{
											backgroundColor: color,
											opacity: isSelected ? 1 : 0.6,
										}}
									/>
									{label}
								</button>
							);
						})}

						{selectedCategory && (
							<button
								type="button"
								className="rounded-full px-3 py-1 text-xs bg-foreground/10 text-foreground ring-1 ring-foreground/20 flex items-center gap-1.5"
								onClick={() => setSelectedCategory(undefined)}
							>
								{formatAppCategory(selectedCategory)}
								<X className="h-3 w-3" />
							</button>
						)}
					</div>
				)}
			</div>

			<div
				className={`flex-1 overflow-auto pb-8 ${isMobile ? "px-4" : "px-4 sm:px-8"}`}
			>
				{error ? (
					<Alert variant="destructive" className="mb-4">
						<AlertCircle className="h-4 w-4" />
						<AlertDescription>
							Failed to load apps: {error.message}
						</AlertDescription>
					</Alert>
				) : combinedApps.length === 0 ? (
					<ExploreEmpty hasFilters={hasActiveFilters} />
				) : categorizedApps && categorizedApps.length > 1 ? (
					<div className={isMobile ? "space-y-5" : "space-y-10"}>
						{categorizedApps.map(({ label, items }, idx) => (
							<ExploreSection
								key={label}
								title={label}
								apps={items}
								userAppIds={userAppIds}
								onAppClick={handleAppClick}
								appHref={appHref}
								isMobile={isMobile}
								categoryColor={CATEGORY_COLORS[label]}
								defaultExpanded={idx === 0}
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
						{(selectedCategory || debouncedQuery) && (
							<p className="text-xs text-muted-foreground/60">
								{combinedApps.length} result
								{combinedApps.length !== 1 ? "s" : ""}
								{selectedCategory &&
									` in ${formatAppCategory(selectedCategory)}`}
							</p>
						)}

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

function ExploreSection({
	title,
	apps,
	userAppIds,
	onAppClick,
	appHref,
	isMobile,
	categoryColor,
	defaultExpanded = false,
}: Readonly<{
	title: string;
	apps: [IApp, IMetadata | undefined][];
	userAppIds: Set<string>;
	onAppClick: (id: string) => void;
	appHref?: (id: string) => string;
	isMobile: boolean;
	categoryColor?: string;
	defaultExpanded?: boolean;
}>) {
	const containerRef = useRef<HTMLDivElement>(null);
	const cardMin = isMobile ? CARD_MIN_W_MOBILE : CARD_MIN_W_DESKTOP;
	const cols = useGridColumns(containerRef, cardMin);
	const [expanded, setExpanded] = useState(defaultExpanded);

	const collapsedCount = cols * 1;
	const needsExpand = apps.length > collapsedCount;
	const visibleApps = expanded ? apps : apps.slice(0, collapsedCount);
	const hiddenCount = apps.length - collapsedCount;

	if (apps.length === 0) return null;

	if (isMobile) {
		return (
			<section>
				<div className="flex items-center justify-between mb-2">
					<div className="flex items-center gap-2">
						{categoryColor && (
							<span
								className="w-2 h-2 rounded-full shrink-0"
								style={{ backgroundColor: categoryColor, opacity: 0.6 }}
							/>
						)}
						<h2 className="text-base font-bold tracking-tight text-foreground">
							{title}
						</h2>
						<span className="text-xs text-muted-foreground/40">
							{apps.length}
						</span>
					</div>
					{needsExpand && !expanded && (
						<button
							type="button"
							onClick={() => setExpanded(true)}
							className="text-sm font-medium text-primary"
						>
							See All
						</button>
					)}
				</div>
				<div className="divide-y divide-border/30">
					{visibleApps.map(([app, metadata]) => (
						<AppCard
							key={app.id}
							isOwned={userAppIds.has(app.id)}
							app={app}
							metadata={metadata}
							variant="small"
							onClick={() => onAppClick(app.id)}
							className="w-full rounded-none border-0 shadow-none bg-transparent"
						/>
					))}
				</div>
				{needsExpand && expanded && (
					<div className="flex justify-center mt-2">
						<button
							type="button"
							onClick={() => setExpanded(false)}
							className="flex items-center gap-1.5 text-xs text-muted-foreground/60 hover:text-foreground px-4 py-1.5 rounded-full border border-border/30 hover:border-border/50 hover:bg-muted/30 transition-colors"
						>
							Less
						</button>
					</div>
				)}
			</section>
		);
	}

	return (
		<section>
			<div className="flex items-center gap-2 mb-3">
				{categoryColor && (
					<span
						className="w-2 h-2 rounded-full shrink-0"
						style={{ backgroundColor: categoryColor, opacity: 0.6 }}
					/>
				)}
				<h2 className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
					{title}
				</h2>
				<span className="text-xs text-muted-foreground/30">{apps.length}</span>
			</div>

			<div
				ref={containerRef}
				className="grid gap-3"
				style={{
					gridTemplateColumns: `repeat(auto-fill, minmax(${cardMin}px, 1fr))`,
				}}
			>
				{visibleApps.map(([app, metadata]) => (
					<AppCard
						key={app.id}
						isOwned={userAppIds.has(app.id)}
						app={app}
						metadata={metadata}
						variant="extended"
						onClick={() => onAppClick(app.id)}
						className="w-full"
					/>
				))}
			</div>

			{needsExpand && (
				<div className="flex justify-center mt-3">
					<button
						type="button"
						onClick={() => setExpanded((e) => !e)}
						className="flex items-center gap-1.5 text-xs text-muted-foreground/60 hover:text-foreground px-4 py-1.5 rounded-full border border-border/30 hover:border-border/50 hover:bg-muted/30 transition-colors"
					>
						{expanded ? "Less" : `${hiddenCount} more`}
						<ChevronDown
							className={`h-3 w-3 transition-transform ${expanded ? "rotate-180" : ""}`}
						/>
					</button>
				</div>
			)}
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
	apps: [IApp, IMetadata | undefined][];
	userAppIds: Set<string>;
	onAppClick: (id: string) => void;
	appHref?: (id: string) => string;
	isMobile: boolean;
}>) {
	const containerRef = useRef<HTMLDivElement>(null);
	const cardMin = isMobile ? CARD_MIN_W_MOBILE : CARD_MIN_W_DESKTOP;

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
						href={appHref?.(app.id)}
						className="w-full rounded-none border-0 shadow-none bg-transparent"
					/>
				))}
			</div>
		);
	}

	return (
		<div
			ref={containerRef}
			className="grid gap-3"
			style={{
				gridTemplateColumns: `repeat(auto-fill, minmax(${cardMin}px, 1fr))`,
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
					href={appHref?.(app.id)}
					className="w-full"
				/>
			))}
		</div>
	);
}

function ExploreEmpty({ hasFilters }: { hasFilters: boolean }) {
	return (
		<div className="flex flex-col items-center justify-center py-32 text-center">
			<div className="rounded-full bg-muted/30 p-5 mb-5">
				<Package className="h-7 w-7 text-muted-foreground/40" />
			</div>
			<p className="text-sm text-foreground/60 mb-1">
				{hasFilters ? "No apps match your filters" : "No apps found"}
			</p>
			<p className="text-xs text-muted-foreground/60">
				{hasFilters
					? "Try adjusting your search or filters"
					: "Check back later for new community apps"}
			</p>
		</div>
	);
}

function LoadMoreButton({
	isFetching,
	onFetch,
}: {
	isFetching: boolean;
	onFetch: () => void;
}) {
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
						<Loader2 className="h-3 w-3 animate-spin" />
						Loading…
					</>
				) : (
					"Load more"
				)}
			</button>
		</div>
	);
}

function ExploreSkeleton() {
	return (
		<div className="space-y-8 md:space-y-12 px-4 sm:px-8 pt-6">
			<Skeleton className="h-10 w-full max-w-lg rounded-full" />
			{Array.from({ length: 3 }).map((_, row) => (
				<div key={`skel-row-${row.toString()}`} className="space-y-3">
					<Skeleton className="h-4 w-28 rounded" />
					<div className="hidden md:grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-3">
						{Array.from({ length: 5 }).map((_, i) => (
							<Skeleton
								key={`skel-${row}-${i.toString()}`}
								className="h-72 rounded-xl"
							/>
						))}
					</div>
					<div className="md:hidden space-y-0 divide-y divide-border/20">
						{Array.from({ length: 3 }).map((_, i) => (
							<div
								key={`skel-m-${row}-${i.toString()}`}
								className="flex items-center gap-3 py-3"
							>
								<Skeleton className="h-12 w-12 rounded-xl shrink-0" />
								<div className="flex-1 space-y-1.5">
									<Skeleton className="h-3.5 w-32 rounded" />
									<Skeleton className="h-3 w-48 rounded" />
								</div>
							</div>
						))}
					</div>
				</div>
			))}
		</div>
	);
}

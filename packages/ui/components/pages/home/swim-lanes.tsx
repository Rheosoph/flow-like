"use client";
import { type UseQueryResult, useQuery } from "@tanstack/react-query";
import { ArrowRight, ExternalLink } from "lucide-react";
import type { AppRouterInstance } from "next/dist/shared/lib/app-router-context.shared-runtime";
import { useRouter } from "next/navigation";
import { useInvoke } from "../../../hooks";
import type { IApp, IAppCategory, IMetadata } from "../../../lib";
import { IAppSearchSort } from "../../../lib/schema/app/app-search-query";
import { type IBackendState, useBackend } from "../../../state/backend-state";
import {
	Alert,
	AlertDescription,
	AlertTitle,
	BitCard,
	DynamicImage,
	Skeleton,
} from "../../ui";
import { AppCard } from "../../ui/app-card";

type IAppQuery = UseQueryResult<[IApp, IMetadata | undefined][], Error>;

export interface ISwimlaneItem {
	id: string;
	type: "app" | "model" | "static";
	appId?: string;
	modelId?: string;
	hub?: string;
	title?: string;
	description?: string;
	image?: string;
	link?: string;
	badge?: string;
	icon?: string;
	gradient?: string;
}

export interface ISearchQuery {
	id?: string;
	type: "search";
	query?: string;
	limit?: number;
	offset?: number;
	category?: IAppCategory;
	author?: string;
	sort?: IAppSearchSort;
	tag?: string;
}

export interface ISwimlane {
	id: string;
	title: string;
	subtitle?: string;
	size: "large" | "medium" | "small";
	items?: (ISwimlaneItem | ISwimlaneItem[] | ISearchQuery)[];
	viewAllLink?: string;
}

const swimlanesUrl = "https://cdn.flow-like.com/swimlanes.json";

function useSwimlanes() {
	return useQuery<ISwimlane[]>({
		queryKey: ["swimlanes"],
		queryFn: async () => {
			const res = await fetch(swimlanesUrl, {
				cache: "no-cache",
			});
			if (!res.ok) throw new Error("Failed to fetch swimlanes");
			return res.json();
		},
		retry: 1,
		refetchOnWindowFocus: true,
		refetchOnReconnect: true,
		refetchOnMount: "always",
		staleTime: 1000 * 60 * 60,
		gcTime: 1000 * 60 * 60 * 24 * 7,
		placeholderData: (prev) => prev,
		networkMode: "offlineFirst",
	});
}

export function HomeSwimlanes() {
	const backend = useBackend();
	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	const latestApps = useInvoke(backend.appState.searchApps, backend.appState, [
		undefined,
		undefined,
		undefined,
		undefined,
		undefined,
		IAppSearchSort.NewestCreated,
		undefined,
		undefined,
		6,
	]);
	const router = useRouter();
	const { data, error } = useSwimlanes();
	const recentLibraryApps = (apps.data ?? [])
		.toSorted(
			([leftApp], [rightApp]) =>
				(rightApp.updated_at?.secs_since_epoch ?? 0) -
				(leftApp.updated_at?.secs_since_epoch ?? 0),
		)
		.slice(0, 6);

	if (error) {
		return (
			<div className="flex-1 min-h-0 items-center w-full overflow-auto p-4 grid grid-cols-6 justify-start gap-2">
				<div className="col-span-6">
					<Alert variant="destructive">
						<ExternalLink className="h-4 w-4" />
						<AlertTitle>Connection Error</AlertTitle>
						<AlertDescription>
							Failed to load swimlanes. Please check your internet connection or
							try again later.
							{error.message && (
								<details className="mt-2">
									<summary className="cursor-pointer text-sm opacity-80 hover:opacity-100">
										Technical details
									</summary>
									<code className="text-xs bg-background/50 px-2 py-1 rounded mt-1 block">
										{error.message}
									</code>
								</details>
							)}
						</AlertDescription>
					</Alert>
				</div>
				<Skeleton className="col-span-6 h-full min-h-[30dvh]" />
				<Skeleton className="col-span-3 h-full min-h-[20dvh]" />
				<Skeleton className="col-span-3 h-full" />
				<Skeleton className="col-span-2 h-full" />
				<Skeleton className="col-span-2 h-full" />
				<Skeleton className="col-span-2 h-full" />
			</div>
		);
	}

	if (!data)
		return (
			<div className="flex-1 min-h-0 items-center w-full overflow-auto p-4 grid grid-cols-6 justify-start gap-2">
				<Skeleton className="col-span-6 h-full min-h-[30dvh]" />
				<Skeleton className="col-span-3 h-full min-h-[20dvh]" />
				<Skeleton className="col-span-3 h-full" />
				<Skeleton className="col-span-2 h-full" />
				<Skeleton className="col-span-2 h-full" />
				<Skeleton className="col-span-2 h-full" />
			</div>
		);

	return (
		<div className="flex-1 min-h-0 w-full overflow-auto bg-background flex flex-col items-center">
			<div className="w-full space-y-8 p-6 max-w-450">
				{data?.map((swimlane) => (
					<SwimlaneSection
						key={swimlane.id}
						swimlane={swimlane}
						apps={apps}
						router={router}
					/>
				))}
				<LibraryAppsSection
					recentLibraryApps={recentLibraryApps}
					router={router}
				/>
				<LatestUserAppsSection
					apps={apps}
					latestApps={latestApps}
					router={router}
				/>
			</div>
		</div>
	);
}

function LibraryAppsSection({
	recentLibraryApps,
	router,
}: Readonly<{
	recentLibraryApps: [IApp, IMetadata | undefined][];
	router: AppRouterInstance;
}>) {
	if (recentLibraryApps.length === 0) {
		return null;
	}

	return (
		<section className="space-y-4">
			<div className="flex items-center justify-between gap-4">
				<div className="space-y-1">
					<h2 className="text-2xl font-bold text-foreground">
						From Your Library
					</h2>
					<p className="text-muted-foreground">
						Jump back into the apps you updated most recently.
					</p>
				</div>
				<a href="/library">
					<button
						type="button"
						className="flex items-center gap-2 text-sm font-medium text-primary hover:text-primary/80 transition-colors"
					>
						Open Library
						<ArrowRight className="w-4 h-4" />
					</button>
				</a>
			</div>
			<div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
				{recentLibraryApps.map(([app, metadata]) => (
					<AppCard
						key={app.id}
						isOwned
						app={app}
						metadata={metadata}
						variant="extended"
						className="w-full h-full"
						onClick={() => router.push(`/use?id=${app.id}`)}
						href={`/use?id=${app.id}`}
					/>
				))}
			</div>
		</section>
	);
}

function LatestUserAppsSection({
	apps,
	latestApps,
	router,
}: Readonly<{
	apps: IAppQuery;
	latestApps: IAppQuery;
	router: AppRouterInstance;
}>) {
	if (!latestApps.data?.length) {
		if (!latestApps.isFetching) {
			return null;
		}

		return (
			<section className="space-y-4">
				<div className="space-y-1">
					<h2 className="text-2xl font-bold text-foreground">
						Latest Community Apps
					</h2>
					<p className="text-muted-foreground">
						Freshly published apps from the community.
					</p>
				</div>
				<div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
					{Array.from({ length: 6 }).map((_, index) => (
						<Skeleton
							key={`latest-app-skeleton-${index}`}
							className="h-93.75 w-full rounded-lg"
						/>
					))}
				</div>
			</section>
		);
	}

	return (
		<section className="space-y-4">
			<div className="flex items-center justify-between gap-4">
				<div className="space-y-1">
					<h2 className="text-2xl font-bold text-foreground">
						Latest Community Apps
					</h2>
					<p className="text-muted-foreground">
						Freshly published apps from the community.
					</p>
				</div>
				<a href="/store?sort=newest">
					<button
						type="button"
						className="flex items-center gap-2 text-sm font-medium text-primary hover:text-primary/80 transition-colors"
					>
						View All
						<ArrowRight className="w-4 h-4" />
					</button>
				</a>
			</div>
			<div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
				{latestApps.data.map(([app, metadata]) => {
					const isOwned =
						apps.data?.some(([ownedApp]) => ownedApp.id === app.id) ?? false;

					return (
						<AppCard
							key={app.id}
							isOwned={isOwned}
							app={app}
							metadata={metadata}
							variant="extended"
							className="w-full h-full"
							onClick={() =>
								router.push(
									isOwned ? `/use?id=${app.id}` : `/store?id=${app.id}`,
								)
							}
							href={isOwned ? `/use?id=${app.id}` : `/store?id=${app.id}`}
						/>
					);
				})}
			</div>
		</section>
	);
}

function SwimlaneSection({
	swimlane,
	apps,
	router,
}: Readonly<{
	swimlane: ISwimlane;
	apps: IAppQuery;
	router: AppRouterInstance;
}>) {
	const getGridCols = () => {
		switch (swimlane.size) {
			case "large":
				return "grid-cols-1 lg:grid-cols-2";
			case "medium":
				return "grid-cols-1 md:grid-cols-2 xl:grid-cols-3";
			case "small":
				return "grid-cols-1 md:grid-cols-2 lg:grid-cols-3";
		}
	};

	const getItemSize = () => {
		switch (swimlane.size) {
			case "large":
				return "extended";
			case "medium":
			case "small":
				return "small";
		}
	};

	return (
		<section className="space-y-4">
			<SwimlaneHeader swimlane={swimlane} apps={apps} />
			<div className={`grid ${getGridCols()} gap-4`}>
				{swimlane.items?.map((item, index) => (
					<SwimlaneSlot
						key={`slot-${index}`}
						items={Array.isArray(item) ? item : [item]}
						size={swimlane.size}
						variant={getItemSize()}
						apps={apps}
						router={router}
					/>
				))}
			</div>
		</section>
	);
}

function SwimlaneSlot({
	items,
	size,
	variant,
	apps,
	router,
}: Readonly<{
	items: (ISwimlaneItem | ISearchQuery)[];
	size: "large" | "medium" | "small";
	variant: "extended" | "small";
	apps: IAppQuery;
	router: AppRouterInstance;
}>) {
	if (items.length === 1) {
		return (
			<SwimlaneItemOrSearch
				item={items[0]}
				size={size}
				variant={variant}
				apps={apps}
				router={router}
			/>
		);
	}

	const isHorizontal = size === "large" || size === "medium";
	const scrollClass = isHorizontal
		? "flex gap-3 overflow-hidden scrollbar-hide pb-2 w-full flex-row justify-stretch"
		: "flex flex-col gap-3 overflow-y-auto scrollbar-hide max-h-[600px]";

	return (
		<div className={scrollClass}>
			{items.map((item) => (
				<div key={item.id} className={isHorizontal ? "grow w-full" : ""}>
					<SwimlaneItemOrSearch
						item={item}
						size={size}
						variant={variant}
						apps={apps}
						router={router}
					/>
				</div>
			))}
		</div>
	);
}

function SwimlaneItemOrSearch({
	item,
	size,
	variant,
	apps,
	router,
}: Readonly<{
	item: ISwimlaneItem | ISearchQuery;
	size: "large" | "medium" | "small";
	variant: "extended" | "small";
	apps: IAppQuery;
	router: AppRouterInstance;
}>) {
	if (item.type === "search") {
		return (
			<SearchResults
				searchQuery={item}
				size={size}
				variant={variant}
				apps={apps}
				router={router}
			/>
		);
	}

	return (
		<SwimlaneItem
			item={item}
			size={size}
			variant={variant}
			apps={apps}
			router={router}
		/>
	);
}

function SearchResults({
	searchQuery,
	size,
	variant,
	apps,
	router,
}: Readonly<{
	searchQuery: ISearchQuery;
	size: "large" | "medium" | "small";
	variant: "extended" | "small";
	apps: IAppQuery;
	router: AppRouterInstance;
}>) {
	const backend = useBackend();
	const searchResults = useInvoke(
		backend.appState.searchApps,
		backend.appState,
		[
			searchQuery.id,
			searchQuery.query,
			undefined,
			searchQuery.category,
			searchQuery.author,
			searchQuery.sort,
			searchQuery.tag,
			searchQuery.offset,
			searchQuery.limit,
		],
	);

	const getMaxSearchItems = () => {
		switch (size) {
			case "large":
				return 3;
			case "medium":
				return 4;
			case "small":
				return 5;
		}
	};

	if (!searchResults.data || searchResults.data.length === 0) {
		return (
			<div className="flex items-center justify-center h-32 text-muted-foreground">
				<span>No results found</span>
			</div>
		);
	}

	const searchItems = searchResults.data
		.slice(0, getMaxSearchItems())
		.map(([app, metadata], index) => ({
			id: `search-${searchQuery.id}-${app.id}-${index}`,
			type: "app" as const,
			appId: app.id,
		}));

	const isHorizontal = size === "large" || size === "medium";
	const scrollClass = isHorizontal
		? "flex gap-3 overflow-hidden scrollbar-hide pb-2 w-full flex-row justify-stretch"
		: "flex flex-col gap-3 overflow-y-auto scrollbar-hide max-h-[600px]";

	return (
		<div className={scrollClass}>
			{searchItems.map((item) => (
				<div key={item.id} className={isHorizontal ? "grow w-full" : ""}>
					<SwimlaneItem
						item={item}
						size={size}
						variant={variant}
						apps={apps}
						router={router}
					/>
				</div>
			))}
		</div>
	);
}

function SwimlaneHeader({
	swimlane,
	apps,
}: Readonly<{ swimlane: ISwimlane; apps: IAppQuery }>) {
	const isExternal = (href?: string) =>
		typeof href === "string" && /^(https?:|mailto:|tel:)/.test(href);
	return (
		<div className="flex items-center justify-between">
			<div className="space-y-1">
				<h2 className="text-2xl font-bold text-foreground">{swimlane.title}</h2>
				{swimlane.subtitle && (
					<p className="text-muted-foreground">{swimlane.subtitle}</p>
				)}
			</div>
			{swimlane.viewAllLink && (
				<a
					href={swimlane.viewAllLink}
					target={isExternal(swimlane.viewAllLink) ? "_blank" : undefined}
					rel={
						isExternal(swimlane.viewAllLink)
							? "noopener noreferrer external"
							: undefined
					}
					data-open-external={
						isExternal(swimlane.viewAllLink) ? "true" : undefined
					}
				>
					<button
						type="button"
						className="flex items-center gap-2 text-sm font-medium text-primary hover:text-primary/80 transition-colors"
					>
						View All
						<ArrowRight className="w-4 h-4" />
					</button>
				</a>
			)}
		</div>
	);
}

function SwimlaneItem({
	item,
	size,
	variant,
	apps,
	router,
}: Readonly<{
	item: ISwimlaneItem;
	size: "large" | "medium" | "small";
	variant: "extended" | "small";
	apps: IAppQuery;
	router: AppRouterInstance;
}>) {
	const backend = useBackend();

	if (item.type === "app" && item.appId) {
		return (
			<AppCardLoading
				appId={item.appId}
				variant={variant}
				backend={backend}
				apps={apps}
				router={router}
			/>
		);
	}

	if (item.type === "model" && item.modelId && item.hub) {
		return (
			<BitCardLoading backend={backend} bitId={item.modelId} hub={item.hub} />
		);
	}

	if (item.type === "static") {
		return <StaticCard item={item} size={size} />;
	}

	return null;
}

function StaticCard({
	item,
	size,
}: Readonly<{
	item: ISwimlaneItem;
	size: "large" | "medium" | "small";
}>) {
	const isLarge = size === "large";
	const cardHeight = isLarge ? "h-[375px]" : "min-h-[200px]";
	const isExternal =
		typeof item.link === "string" && /^(https?:|mailto:|tel:)/.test(item.link);

	return (
		<a
			href={item.link}
			target={isExternal ? "_blank" : "_self"}
			rel={isExternal ? "noopener noreferrer external" : undefined}
			data-open-external={isExternal ? "true" : undefined}
			className={`group relative overflow-hidden rounded-xl border border-border/40 bg-card shadow-sm hover:shadow-xl hover:border-primary/30 transition-all duration-300 ${cardHeight} w-full`}
		>
			<div className="absolute inset-0">
				{item.image ? (
					<img
						src={item.image}
						alt={item.title}
						className="w-full h-full object-cover transition-transform duration-500 group-hover:scale-105"
					/>
				) : (
					<div
						className={`w-full h-full bg-linear-to-br ${
							item.gradient || "from-primary/20 to-primary/40"
						}`}
					/>
				)}
				<div className="absolute inset-0 bg-linear-to-t from-black/20 via-black/5 dark:from-black/60 dark:via-black/20 to-transparent" />
			</div>

			<div className="relative z-10 flex flex-col justify-between h-full p-6">
				{item.badge && (
					<div className="self-start">
						<div className="bg-white/90 backdrop-blur-xs text-gray-900 rounded-full px-3 py-1 text-xs font-bold shadow-lg">
							{item.badge}
						</div>
					</div>
				)}

				<div className="space-y-3">
					<div className="flex items-center gap-2">
						{item.icon && (
							<div className="p-2 bg-white/20 backdrop-blur-xs rounded-full text-white">
								<DynamicImage url={item.icon} className="w-5 h-5 bg-white" />
							</div>
						)}
						<h3 className="font-bold text-white text-left text-lg leading-tight">
							{item.title}
						</h3>
					</div>
					{item.description && (
						<p className="text-white/90 text-left text-sm leading-relaxed max-w-md">
							{item.description}
						</p>
					)}
				</div>

				{item.link && size === "large" && (
					<div className="flex items-center gap-2 text-white/80 group-hover:text-white transition-colors">
						<span className="text-sm font-medium">Learn More</span>
						<ExternalLink className="w-4 h-4" />
					</div>
				)}
			</div>
		</a>
	);
}

function BitCardLoading({
	bitId,
	hub,
	backend,
}: Readonly<{ bitId: string; hub: string; backend: IBackendState }>) {
	const bit = useInvoke(backend.bitState.getBit, backend.bitState, [
		bitId,
		hub,
	]);

	if (!bit.data) {
		return <Skeleton className="w-full h-full rounded-lg" />;
	}

	return <BitCard bit={bit.data} wide={false} />;
}

function AppCardLoading({
	appId,
	variant,
	backend,
	apps,
	router,
}: Readonly<{
	appId: string;
	backend: IBackendState;
	variant: "small" | "extended";
	apps: UseQueryResult<[IApp, IMetadata | undefined][], Error>;
	router: AppRouterInstance;
}>) {
	const app = useInvoke(backend.appState.searchApps, backend.appState, [appId]);

	if (!app.data || (app.data?.length ?? 0) <= 0) {
		return (
			<Skeleton
				className={`w-full h-full rounded-lg ${variant === "extended" ? "min-w-72 h-93.75" : "h-15 min-w-1/3 w-full"}`}
			/>
		);
	}

	const meta = app.data[0][1];
	const data = app.data[0][0];
	const isOwned = apps.data?.some((a) => a[0].id === data.id) ?? false;

	return (
		<AppCard
			isOwned={isOwned}
			app={data}
			metadata={meta}
			variant={variant}
			className={"w-full max-w-full h-full flex grow"}
			onClick={async () => {
				if (isOwned) return router.push(`/use?id=${data.id}`);
				return router.push(`/store?id=${data.id}`);
			}}
			href={isOwned ? `/use?id=${data.id}` : `/store?id=${data.id}`}
		/>
	);
}

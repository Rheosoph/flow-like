"use client";
import { useTranslation } from "@flow-like/locales";
import { type UseQueryResult, useQuery } from "@tanstack/react-query";
import { motion, useReducedMotion } from "framer-motion";
import { ArrowRight, ArrowUpRight, CloudOff } from "lucide-react";
import type { AppRouterInstance } from "next/dist/shared/lib/app-router-context.shared-runtime";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Fragment, type ReactNode, useMemo } from "react";
import { useInvoke } from "../../../hooks";
import type { IApp, IAppCategory, IMetadata } from "../../../lib";
import { useAppCategoryLabel } from "../../../lib/app-category";
import {
	APP_CATEGORY_ORDER,
	CATEGORY_ICONS,
	categoryColor,
} from "../../../lib/category-meta";
import { IAppSearchSort } from "../../../lib/schema/app/app-search-query";
import { type IBackendState, useBackend } from "../../../state/backend-state";
import { sortAppPairsByRecency } from "../../library/library-types";
import {
	Alert,
	AlertDescription,
	AlertTitle,
	BitCard,
	DynamicImage,
	ScrollRail,
	Skeleton,
} from "../../ui";
import { AppCard, SpotlightCard } from "../../ui/app-card";

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
	/** Optional column heading, e.g. the category name in a top-charts lane. */
	title?: string;
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

// const swimlanesUrl = "https://cdn.flow-like.com/swimlanes.json";
const swimlanesUrl = "/swimlanes/swimlanes.json";

const isExternalLink = (href?: string) =>
	typeof href === "string" && /^(https?:|mailto:|tel:)/.test(href);

// The CDN config may lag behind the app: rewrite links to routes that no
// longer exist onto their current equivalents instead of a 404.
const LEGACY_LINKS: Record<string, string> = {
	"/apps/featured": "/store/explore/apps",
	"/apps/recent": "/store/explore/apps?sort=newest",
	"/store": "/store/explore/apps",
	"/store?sort=newest": "/store/explore/apps?sort=newest",
};

const normalizeLink = (href: string) => LEGACY_LINKS[href] ?? href;

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
	const { t } = useTranslation("common");
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
	const ownedIds = useMemo(
		() => new Set((apps.data ?? []).map(([app]) => app.id)),
		[apps.data],
	);
	const recentLibraryApps = useMemo(
		() => sortAppPairsByRecency(apps.data ?? []).slice(0, 6),
		[apps.data],
	);

	return (
		// Flow content (the PAGE scrolls, not this block). Gradient veil instead of a solid slab:
		// the hero's animated background bleeds softly into the content area (no hard seam), with
		// the fade completing within the first ~14rem so everything below reads on solid ground.
		<div className="w-full bg-linear-to-b from-background/0 via-background/90 via-[8rem] to-background to-[14rem] flex flex-col items-center">
			<div className="w-full space-y-12 px-6 pt-2 pb-16 max-w-450">
				<CategoryRail />

				{error && !data && (
					<Alert variant="destructive">
						<CloudOff className="h-4 w-4" />
						<AlertTitle>
							{t("couldntLoadHighlights", "Couldn’t load highlights")}
						</AlertTitle>
						<AlertDescription>
							{t(
								"theCuratedSectionsAreUnavailableRightNowCheckYourConnectionOrTryAgainLaterYourLibraryAndCommunityAppsBelowStillWork",
								"The curated sections are unavailable right now — check your connection or try again later. Your library and community apps below still work.",
							)}
						</AlertDescription>
					</Alert>
				)}

				{!error && !data && <LanesSkeleton />}

				{data?.map((swimlane) => (
					<SwimlaneSection
						key={swimlane.id}
						swimlane={swimlane}
						ownedIds={ownedIds}
						router={router}
					/>
				))}

				<LibraryAppsSection
					recentLibraryApps={recentLibraryApps}
					router={router}
				/>
				<LatestUserAppsSection
					ownedIds={ownedIds}
					latestApps={latestApps}
					router={router}
				/>
			</div>
		</div>
	);
}

// ─── Section chrome ──────────────────────────────────────────────────────────

function Reveal({ children }: Readonly<{ children: ReactNode }>) {
	const reducedMotion = useReducedMotion();

	if (reducedMotion) return <section>{children}</section>;

	return (
		<motion.section
			initial={{ opacity: 0, y: 24 }}
			whileInView={{ opacity: 1, y: 0 }}
			viewport={{ once: true, margin: "-40px" }}
			transition={{ duration: 0.5, ease: [0.21, 0.47, 0.32, 0.98] }}
		>
			{children}
		</motion.section>
	);
}

function SectionHeader({
	title,
	subtitle,
	href,
	linkLabel,
}: Readonly<{
	title: string;
	subtitle?: string;
	href?: string;
	linkLabel?: string;
}>) {
	const { t } = useTranslation("common");
	const resolvedLinkLabel = linkLabel ?? t("viewAll", "View all");
	const external = isExternalLink(href);
	const linkClass = t(
		"grouplinkFlexShrink0ItemscenterGap15RoundedfullBorderBorderborder40Bgcard60Px4Py15TextsmFontmediumTextmutedforegroundTransitionallHoverborderprimary30HovertextforegroundHovershadowsm",
		"group/link flex shrink-0 items-center gap-1.5 rounded-full border border-border/40 bg-card/60 px-4 py-1.5 text-sm font-medium text-muted-foreground transition-all hover:border-primary/30 hover:text-foreground hover:shadow-sm",
	);
	const linkContent = (
		<>
			{resolvedLinkLabel}
			{external ? (
				<ArrowUpRight className="h-3.5 w-3.5 transition-transform group-hover/link:translate-x-0.5 group-hover/link:-translate-y-0.5" />
			) : (
				<ArrowRight className="h-3.5 w-3.5 transition-transform group-hover/link:translate-x-0.5" />
			)}
		</>
	);

	return (
		<div className="mb-5 flex items-end justify-between gap-4">
			<div className="min-w-0 space-y-1">
				<h2 className="text-xl md:text-2xl font-bold tracking-tight text-foreground">
					{title}
				</h2>
				{subtitle && (
					<p className="text-sm text-muted-foreground">{subtitle}</p>
				)}
			</div>
			{href &&
				(external ? (
					<a
						href={href}
						target="_blank"
						rel="noopener noreferrer external"
						data-open-external="true"
						className={linkClass}
					>
						{linkContent}
					</a>
				) : (
					<Link href={normalizeLink(href)} className={linkClass}>
						{linkContent}
					</Link>
				))}
		</div>
	);
}

// ─── Category rail ───────────────────────────────────────────────────────────

function CategoryRail() {
	const { t } = useTranslation("common");
	const categoryLabel = useAppCategoryLabel();
	return (
		<Reveal>
			<SectionHeader
				title={t("browseByCategory", "Browse by category")}
				subtitle={t(
					"findTheRightAppForEveryJob",
					"Find the right app for every job.",
				)}
				href="/store/explore/apps"
				linkLabel={t("exploreAll", "Explore all")}
			/>
			<ScrollRail>
				{APP_CATEGORY_ORDER.map((category) => {
					const label = categoryLabel(category);
					const color = categoryColor(category);
					const Icon = CATEGORY_ICONS[category];
					return (
						<Link
							key={category}
							href={`/store/explore/apps?category=${category}`}
							className="group flex shrink-0 snap-start items-center gap-2.5 rounded-full border border-border/40 bg-card/70 py-2 pl-2 pr-4 backdrop-blur-sm transition-all hover:-translate-y-0.5 hover:border-primary/30 hover:shadow-md"
						>
							<span
								className="flex h-7 w-7 items-center justify-center rounded-full transition-transform group-hover:scale-110"
								style={{
									backgroundColor: `color-mix(in oklab, ${color} 16%, transparent)`,
								}}
							>
								<Icon className="h-3.5 w-3.5" style={{ color }} />
							</span>
							<span className="text-sm font-medium text-foreground/90 whitespace-nowrap">
								{label}
							</span>
						</Link>
					);
				})}
			</ScrollRail>
		</Reveal>
	);
}

// ─── Library / community sections ────────────────────────────────────────────

function LibraryAppsSection({
	recentLibraryApps,
	router,
}: Readonly<{
	recentLibraryApps: [IApp, IMetadata | undefined][];
	router: AppRouterInstance;
}>) {
	const { t } = useTranslation("common");
	if (recentLibraryApps.length === 0) {
		return null;
	}

	return (
		<Reveal>
			<SectionHeader
				title={t("fromYourLibrary", "From your library")}
				subtitle={t(
					"jumpBackIntoTheAppsYouUpdatedMostRecently",
					"Jump back into the apps you updated most recently.",
				)}
				href="/library"
				linkLabel={t("openLibrary", "Open library")}
			/>
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
		</Reveal>
	);
}

function LatestUserAppsSection({
	ownedIds,
	latestApps,
	router,
}: Readonly<{
	ownedIds: Set<string>;
	latestApps: IAppQuery;
	router: AppRouterInstance;
}>) {
	const { t } = useTranslation("common");
	if (!latestApps.data?.length) {
		if (!latestApps.isFetching) {
			return null;
		}

		return (
			<section>
				<SectionHeader
					title={t("latestCommunityApps", "Latest community apps")}
					subtitle={t(
						"freshlyPublishedAppsFromTheCommunity",
						"Freshly published apps from the community.",
					)}
				/>
				<div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
					{["a", "b", "c", "d", "e", "f"].map((slot) => (
						<Skeleton
							key={`latest-app-skeleton-${slot}`}
							className="h-93.75 w-full rounded-xl"
						/>
					))}
				</div>
			</section>
		);
	}

	return (
		<Reveal>
			<SectionHeader
				title={t("latestCommunityApps", "Latest community apps")}
				subtitle={t(
					"freshlyPublishedAppsFromTheCommunity",
					"Freshly published apps from the community.",
				)}
				href="/store/explore/apps?sort=newest"
			/>
			<div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
				{latestApps.data.map(([app, metadata]) => {
					const isOwned = ownedIds.has(app.id);
					const href = isOwned ? `/use?id=${app.id}` : `/store?id=${app.id}`;

					return (
						<AppCard
							key={app.id}
							isOwned={isOwned}
							app={app}
							metadata={metadata}
							variant="extended"
							className="w-full h-full"
							onClick={() => router.push(href)}
							href={href}
						/>
					);
				})}
			</div>
		</Reveal>
	);
}

// ─── CDN swimlanes ───────────────────────────────────────────────────────────

function SwimlaneSection({
	swimlane,
	ownedIds,
	router,
}: Readonly<{
	swimlane: ISwimlane;
	ownedIds: Set<string>;
	router: AppRouterInstance;
}>) {
	if (!swimlane.items?.length) return null;

	return (
		<Reveal>
			<SectionHeader
				title={swimlane.title}
				subtitle={swimlane.subtitle}
				href={swimlane.viewAllLink}
			/>
			{swimlane.size === "small" ? (
				<SmallLane swimlane={swimlane} ownedIds={ownedIds} router={router} />
			) : swimlane.size === "large" ? (
				<SpotlightLane
					swimlane={swimlane}
					ownedIds={ownedIds}
					router={router}
				/>
			) : (
				<CardLane swimlane={swimlane} ownedIds={ownedIds} router={router} />
			)}
		</Reveal>
	);
}

/**
 * Large/medium lanes: a responsive grid of feature cards. Slots holding
 * multiple items (arrays or expanded search results) become a snap rail
 * inside their cell so nothing is squeezed or clipped.
 */
function CardLane({
	swimlane,
	ownedIds,
	router,
}: Readonly<{
	swimlane: ISwimlane;
	ownedIds: Set<string>;
	router: AppRouterInstance;
}>) {
	const { t } = useTranslation("common");
	const isLarge = swimlane.size === "large";
	const gridClass = isLarge
		? t("gridGridcols1Lggridcols2Gap4", "grid grid-cols-1 lg:grid-cols-2 gap-4")
		: t(
				"gridGridcols1Mdgridcols2Xlgridcols3Gap4",
				"grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4",
			);
	const variant = isLarge ? "extended" : "small";

	return (
		<div className={gridClass}>
			{swimlane.items?.map((slot, index) => {
				const key = `${swimlane.id}-slot-${index}`;
				if (Array.isArray(slot)) {
					return (
						<ScrollRail key={key}>
							{slot.map((item, itemIndex) => (
								<div
									key={item.id ?? `${key}-${itemIndex}`}
									className="w-[85%] min-w-64 shrink-0 snap-start sm:w-[70%]"
								>
									<SwimlaneItem
										item={item}
										size={swimlane.size}
										variant={variant}
										ownedIds={ownedIds}
										router={router}
									/>
								</div>
							))}
						</ScrollRail>
					);
				}
				if (slot.type === "search") {
					return (
						<SearchCards
							key={key}
							searchQuery={slot}
							size={swimlane.size}
							variant={variant}
							ownedIds={ownedIds}
							router={router}
						/>
					);
				}
				return (
					<SwimlaneItem
						key={slot.id ?? key}
						item={slot}
						size={swimlane.size}
						variant={variant}
						ownedIds={ownedIds}
						router={router}
					/>
				);
			})}
		</div>
	);
}

/**
 * Large lanes: the featured showcase. App items render as wide, editorial
 * Spotlight cards (use case + tags + stats); static items stay banners. Arrays
 * and search results are flattened into the same responsive two-column grid.
 */
function SpotlightLane({
	swimlane,
	ownedIds,
	router,
}: Readonly<{
	swimlane: ISwimlane;
	ownedIds: Set<string>;
	router: AppRouterInstance;
}>) {
	return (
		<div className="grid grid-cols-1 gap-5 xl:grid-cols-2">
			{swimlane.items?.map((slot, index) => {
				const key = `${swimlane.id}-slot-${index}`;
				if (Array.isArray(slot)) {
					return (
						<Fragment key={key}>
							{slot.map((item, itemIndex) => (
								<SpotlightItem
									key={item.id ?? `${key}-${itemIndex}`}
									item={item}
									ownedIds={ownedIds}
									router={router}
								/>
							))}
						</Fragment>
					);
				}
				if (slot.type === "search") {
					return (
						<SpotlightSearch
							key={key}
							searchQuery={slot}
							ownedIds={ownedIds}
							router={router}
						/>
					);
				}
				return (
					<SpotlightItem
						key={slot.id ?? key}
						item={slot}
						ownedIds={ownedIds}
						router={router}
					/>
				);
			})}
		</div>
	);
}

function SpotlightItem({
	item,
	ownedIds,
	router,
}: Readonly<{
	item: ISwimlaneItem;
	ownedIds: Set<string>;
	router: AppRouterInstance;
}>) {
	const backend = useBackend();

	if (item.type === "app" && item.appId) {
		return (
			<SpotlightAppLoading
				appId={item.appId}
				backend={backend}
				ownedIds={ownedIds}
				router={router}
			/>
		);
	}
	if (item.type === "static") {
		// promo banners run full width beneath the app spotlights
		return (
			<div className="xl:col-span-2">
				<StaticCard item={item} size="medium" />
			</div>
		);
	}
	if (item.type === "model" && item.modelId && item.hub) {
		return (
			<BitCardLoading backend={backend} bitId={item.modelId} hub={item.hub} />
		);
	}
	return null;
}

function SpotlightAppLoading({
	appId,
	backend,
	ownedIds,
	router,
}: Readonly<{
	appId: string;
	backend: IBackendState;
	ownedIds: Set<string>;
	router: AppRouterInstance;
}>) {
	const app = useInvoke(backend.appState.searchApps, backend.appState, [appId]);

	if (!app.data || app.data.length === 0) {
		if (!app.isFetching) return null;
		return <Skeleton className="h-61 w-full rounded-2xl" />;
	}

	const [data, meta] = app.data[0];
	const isOwned = ownedIds.has(data.id);
	const href = isOwned ? `/use?id=${data.id}` : `/store?id=${data.id}`;

	return (
		<SpotlightCard
			app={data}
			metadata={meta}
			isOwned={isOwned}
			href={href}
			onClick={() => router.push(href)}
			className="h-full"
		/>
	);
}

function SpotlightSearch({
	searchQuery,
	ownedIds,
	router,
}: Readonly<{
	searchQuery: ISearchQuery;
	ownedIds: Set<string>;
	router: AppRouterInstance;
}>) {
	const results = useLaneSearch(searchQuery, 4);

	if (!results.data) {
		if (!results.isFetching) return null;
		return <Skeleton className="h-61 w-full rounded-2xl" />;
	}
	if (results.data.length === 0) return null;

	return (
		<>
			{results.data.map(([app, metadata]) => {
				const isOwned = ownedIds.has(app.id);
				const href = isOwned ? `/use?id=${app.id}` : `/store?id=${app.id}`;
				return (
					<SpotlightCard
						key={app.id}
						app={app}
						metadata={metadata}
						isOwned={isOwned}
						href={href}
						onClick={() => router.push(href)}
						className="h-full"
					/>
				);
			})}
		</>
	);
}

/**
 * Small lanes: static items render as a compact card grid; search slots render
 * as ranked top-list columns (the classic store chart), numbered across slots
 * via their query offsets.
 */
function SmallLane({
	swimlane,
	ownedIds,
	router,
}: Readonly<{
	swimlane: ISwimlane;
	ownedIds: Set<string>;
	router: AppRouterInstance;
}>) {
	return (
		<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-x-8 gap-y-4">
			{swimlane.items?.map((slot, index) => {
				const key = `${swimlane.id}-slot-${index}`;
				if (!Array.isArray(slot) && slot.type === "search") {
					return (
						<RankedColumn
							key={key}
							searchQuery={slot}
							ownedIds={ownedIds}
							router={router}
						/>
					);
				}
				const items = Array.isArray(slot) ? slot : [slot];
				return (
					<Fragment key={key}>
						{items.map((item, itemIndex) => (
							<SwimlaneItem
								key={item.id ?? `${key}-${itemIndex}`}
								item={item}
								size="small"
								variant="small"
								ownedIds={ownedIds}
								router={router}
							/>
						))}
					</Fragment>
				);
			})}
		</div>
	);
}

const RANKED_FALLBACK_LIMIT = 5;

function useLaneSearch(searchQuery: ISearchQuery, limit: number) {
	const backend = useBackend();
	// The slot `id` is a UI identity, NOT the searchApps app-id filter.
	return useInvoke(backend.appState.searchApps, backend.appState, [
		undefined,
		searchQuery.query,
		undefined,
		searchQuery.category,
		searchQuery.author,
		searchQuery.sort,
		searchQuery.tag,
		searchQuery.offset,
		searchQuery.limit ?? limit,
	]);
}

function RankedColumn({
	searchQuery,
	ownedIds,
	router,
}: Readonly<{
	searchQuery: ISearchQuery;
	ownedIds: Set<string>;
	router: AppRouterInstance;
}>) {
	const results = useLaneSearch(searchQuery, RANKED_FALLBACK_LIMIT);
	const rankOffset = searchQuery.offset ?? 0;

	if (!results.data) {
		if (!results.isFetching) return null;
		return (
			<div className="flex flex-col gap-2">
				<RankedColumnHeader searchQuery={searchQuery} />
				{Array.from({
					length: searchQuery.limit ?? RANKED_FALLBACK_LIMIT,
				}).map((_, index) => (
					<div
						key={`rank-skel-${rankOffset + index}`}
						className="flex items-center gap-3"
					>
						<span className="w-7 text-center text-lg font-bold tabular-nums text-muted-foreground/20">
							{rankOffset + index + 1}
						</span>
						<Skeleton className="h-[74px] flex-1 rounded-xl" />
					</div>
				))}
			</div>
		);
	}

	if (results.data.length === 0) return null;

	return (
		<div className="flex flex-col gap-2">
			<RankedColumnHeader searchQuery={searchQuery} />
			{results.data.map(([app, metadata], index) => {
				const isOwned = ownedIds.has(app.id);
				const href = isOwned ? `/use?id=${app.id}` : `/store?id=${app.id}`;
				return (
					<div key={app.id} className="flex min-w-0 items-center gap-3">
						<span className="w-7 shrink-0 text-center text-lg font-bold tabular-nums text-muted-foreground/40">
							{rankOffset + index + 1}
						</span>
						<AppCard
							isOwned={isOwned}
							app={app}
							metadata={metadata}
							variant="small"
							className="flex-1 min-w-0"
							onClick={() => router.push(href)}
							href={href}
						/>
					</div>
				);
			})}
		</div>
	);
}

function RankedColumnHeader({
	searchQuery,
}: Readonly<{ searchQuery: ISearchQuery }>) {
	if (!searchQuery.title) return null;

	return (
		<div className="flex items-center gap-2 pb-1">
			{searchQuery.category && (
				<span
					className="h-2 w-2 shrink-0 rounded-full"
					style={{ backgroundColor: categoryColor(searchQuery.category) }}
				/>
			)}
			<h3 className="text-sm font-semibold tracking-tight text-foreground">
				{searchQuery.title}
			</h3>
		</div>
	);
}

const SEARCH_CARD_LIMITS = { large: 3, medium: 4, small: 5 } as const;

/** Search slot inside a card lane: results become a snap rail of app cards. */
function SearchCards({
	searchQuery,
	size,
	variant,
	ownedIds,
	router,
}: Readonly<{
	searchQuery: ISearchQuery;
	size: "large" | "medium" | "small";
	variant: "extended" | "small";
	ownedIds: Set<string>;
	router: AppRouterInstance;
}>) {
	const results = useLaneSearch(searchQuery, SEARCH_CARD_LIMITS[size]);

	if (!results.data) {
		if (!results.isFetching) return null;
		return (
			<Skeleton
				className={`w-full rounded-xl ${variant === "extended" ? "h-93.75" : "h-[74px]"}`}
			/>
		);
	}

	if (results.data.length === 0) return null;

	return (
		<ScrollRail>
			{results.data.map(([app, metadata]) => {
				const isOwned = ownedIds.has(app.id);
				const href = isOwned ? `/use?id=${app.id}` : `/store?id=${app.id}`;
				return (
					<div
						key={app.id}
						className="w-[85%] min-w-64 shrink-0 snap-start sm:w-[70%]"
					>
						<AppCard
							isOwned={isOwned}
							app={app}
							metadata={metadata}
							variant={variant}
							className="w-full h-full"
							onClick={() => router.push(href)}
							href={href}
						/>
					</div>
				);
			})}
		</ScrollRail>
	);
}

// ─── Item renderers ──────────────────────────────────────────────────────────

function SwimlaneItem({
	item,
	size,
	variant,
	ownedIds,
	router,
}: Readonly<{
	item: ISwimlaneItem;
	size: "large" | "medium" | "small";
	variant: "extended" | "small";
	ownedIds: Set<string>;
	router: AppRouterInstance;
}>) {
	const backend = useBackend();

	if (item.type === "app" && item.appId) {
		return (
			<AppCardLoading
				appId={item.appId}
				variant={variant}
				backend={backend}
				ownedIds={ownedIds}
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
	const { t } = useTranslation("common");
	const isLarge = size === "large";
	const cardHeight = isLarge ? "h-[375px]" : "min-h-[210px]";
	const external = isExternalLink(item.link);

	const body = (
		<>
			<div className="absolute inset-0">
				{item.image ? (
					<img
						src={item.image}
						alt=""
						loading="lazy"
						decoding="async"
						className="h-full w-full object-cover transition-transform duration-700 ease-out group-hover:scale-105"
					/>
				) : (
					<div
						className={`h-full w-full bg-linear-to-br ${
							item.gradient || "from-primary/20 to-primary/40"
						}`}
					/>
				)}
				<div className="absolute inset-0 bg-linear-to-t from-black/70 via-black/25 to-black/5 transition-opacity duration-300 group-hover:opacity-90" />
			</div>

			<div className="relative z-10 flex h-full flex-col justify-between p-6">
				{item.badge ? (
					<div className="self-start rounded-full border border-white/25 bg-white/15 px-3 py-1 text-xs font-semibold text-white shadow-lg backdrop-blur-md">
						{item.badge}
					</div>
				) : (
					<span />
				)}

				<div className="space-y-2.5">
					<div className="flex items-center gap-2.5">
						{item.icon && (
							<div className="rounded-full bg-white/20 p-2 text-white backdrop-blur-sm">
								<DynamicImage
									url={item.icon}
									className="h-4.5 w-4.5 bg-white"
								/>
							</div>
						)}
						<h3
							className={`text-left font-bold leading-tight text-white ${isLarge ? "text-xl md:text-2xl" : "text-lg"}`}
						>
							{item.title}
						</h3>
					</div>
					{item.description && (
						<p className="max-w-md text-left text-sm leading-relaxed text-white/85">
							{item.description}
						</p>
					)}
					{item.link && (
						<div className="flex items-center gap-1.5 pt-1 text-sm font-medium text-white/70 transition-all duration-300 group-hover:gap-2.5 group-hover:text-white">
							<span>{external ? "Learn more" : "Open"}</span>
							<ArrowRight className="h-4 w-4" />
						</div>
					)}
				</div>
			</div>
		</>
	);

	const cardClass = t(
		"groupRelativeBlockOverflowhiddenRounded2xlBorderBorderborder40Bgcard80ShadowsmTransitionallDuration300Hovertranslatey1Hoverborderprimary30HovershadowxlCardheightWfull",
		"group relative block overflow-hidden rounded-2xl border border-border/40 bg-card/80 shadow-sm transition-all duration-300 hover:-translate-y-1 hover:border-primary/30 hover:shadow-xl {{cardHeight}} w-full",
		{ cardHeight },
	);

	if (!item.link) {
		return <div className={cardClass}>{body}</div>;
	}

	if (external) {
		return (
			<a
				href={item.link}
				target="_blank"
				rel="noopener noreferrer external"
				data-open-external="true"
				className={cardClass}
			>
				{body}
			</a>
		);
	}

	return (
		<Link href={normalizeLink(item.link)} className={cardClass}>
			{body}
		</Link>
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
		if (!bit.isFetching) return null;
		return <Skeleton className="h-full min-h-[210px] w-full rounded-xl" />;
	}

	return <BitCard bit={bit.data} wide={false} />;
}

function AppCardLoading({
	appId,
	variant,
	backend,
	ownedIds,
	router,
}: Readonly<{
	appId: string;
	backend: IBackendState;
	variant: "small" | "extended";
	ownedIds: Set<string>;
	router: AppRouterInstance;
}>) {
	const app = useInvoke(backend.appState.searchApps, backend.appState, [appId]);

	if (!app.data || app.data.length === 0) {
		// A missing app (deleted/unpublished) is terminal — don't animate forever.
		if (!app.isFetching) return null;
		return (
			<Skeleton
				className={`w-full rounded-xl ${variant === "extended" ? "min-w-72 h-93.75" : "h-[74px]"}`}
			/>
		);
	}

	const [data, meta] = app.data[0];
	const isOwned = ownedIds.has(data.id);
	const href = isOwned ? `/use?id=${data.id}` : `/store?id=${data.id}`;

	return (
		<AppCard
			isOwned={isOwned}
			app={data}
			metadata={meta}
			variant={variant}
			className="w-full max-w-full h-full flex grow"
			onClick={() => router.push(href)}
			href={href}
		/>
	);
}

// ─── Skeletons ───────────────────────────────────────────────────────────────

function LanesSkeleton() {
	return (
		<div className="space-y-12">
			<div>
				<Skeleton className="mb-5 h-7 w-56 rounded" />
				<div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
					<Skeleton className="h-93.75 w-full rounded-2xl" />
					<Skeleton className="h-93.75 w-full rounded-2xl" />
				</div>
			</div>
			<div>
				<Skeleton className="mb-5 h-7 w-44 rounded" />
				<div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
					<Skeleton className="h-[210px] w-full rounded-2xl" />
					<Skeleton className="h-[210px] w-full rounded-2xl" />
					<Skeleton className="hidden h-[210px] w-full rounded-2xl xl:block" />
				</div>
			</div>
		</div>
	);
}

"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowLeft, Layers, Search, X } from "lucide-react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { useDeveloperMode } from "../../../../hooks/use-developer-mode";
import type { IEventMapping } from "../../../interfaces/interfaces";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { Skeleton } from "../../../ui/skeleton";
import { SuitesRail } from "../../suites";
import {
	EXPLORE_PATH,
	type ExploreBrowseParams,
	exploreSearchText,
	parseBrowseParams,
	serializeBrowseParams,
} from "../explore-href";
import {
	type ExploreAppItem,
	ExploreLinksProvider,
} from "../explore-item-card";
import { useExploreLabels } from "../explore-labels";
import { isExploreUnsupportedError } from "../explore-model";
import { ExploreShell } from "../explore-page";
import { ExploreQueryError, exploreErrorStatus } from "../explore-states";
import type {
	ExploreSearchPackageHit,
	ExploreSearchResponse,
	ExploreSearchSort,
} from "../explore-types";
import { LegacyExploreFallback } from "../legacy-explore";
import { useExploreViewer } from "../use-explore";
import { useExploreSearch } from "../use-explore-search";
import {
	BuildItYourselfCard,
	RecentSearches,
	useRecentSearches,
} from "./browse-aside";
import {
	BrowseFacetsSheet,
	BrowseFacetsSidebar,
	activeFilterCount,
} from "./browse-facets";
import {
	ActiveChips,
	BrowseAppsGroup,
	BrowseCollectionCard,
	BrowseEmpty,
	BrowsePackagesGroup,
	BrowseRelated,
	RESULT_GRID,
	useActiveChips,
} from "./browse-results";

const PAGE_SIZE = 24;
const COLLECTION_PAGE_SIZE = 48;
const DEBOUNCE_MS = 300;
/** Up to this many apps sit in a narrow column beside the packages on wide screens instead of above them. */
const PAIRED_MAX_APPS = 2;
const PAIRED_GROUPS =
	"flex min-w-0 flex-col gap-6 @7xl/explore:grid @7xl/explore:grid-cols-[288px_minmax(0,1fr)] @7xl/explore:items-start";
const SORTS: readonly ExploreSearchSort[] = [
	"best",
	"newest",
	"rating",
	"installs",
	"name",
	"updated",
];

export interface ExploreBrowsePageProps {
	eventConfig?: IEventMapping;
	/** Desktop can scaffold a node package locally; web cannot. */
	canScaffold?: boolean;
}

export function ExploreBrowsePage(props: Readonly<ExploreBrowsePageProps>) {
	return (
		<Suspense
			fallback={
				<ExploreShell>
					<BrowseSkeleton />
				</ExploreShell>
			}
		>
			<ExploreLinksProvider eventConfig={props.eventConfig}>
				<BrowseRoute {...props} />
			</ExploreLinksProvider>
		</Suspense>
	);
}

/**
 * Once the hub turns out to have no Explore, the search unmounts for good: the old apps page writes its own
 * filters into the URL, and each of those writes would otherwise start another search that fails the same way.
 */
function BrowseRoute(props: Readonly<ExploreBrowsePageProps>) {
	const [legacy, setLegacy] = useState(false);
	const onUnsupported = useCallback(() => setLegacy(true), []);
	if (legacy) return <LegacyExploreFallback eventConfig={props.eventConfig} />;
	return <BrowseContent {...props} onUnsupported={onUnsupported} />;
}

function canonical(params: ExploreBrowseParams): string {
	return serializeBrowseParams(params);
}

/** `type=packages` means nothing without developer mode, so such a link shows what the viewer can see. */
function visibleParams(
	params: ExploreBrowseParams,
	developerMode: boolean,
): ExploreBrowseParams {
	return developerMode || params.type !== "packages"
		? params
		: { ...params, type: undefined };
}

/**
 * Params live in the URL. The URL is written with `router.replace` (no scroll), the query text after a 300 ms
 * pause; URL changes this page did not write (back/forward, a link) are adopted. Both sides compare the
 * canonical serialization, so neither direction echoes the other.
 */
function useBrowseParams() {
	const searchParams = useSearchParams();
	const router = useRouter();
	const pathname = usePathname();
	const [params, setParams] = useState<ExploreBrowseParams>(() =>
		parseBrowseParams(searchParams),
	);
	const [text, setText] = useState(params.q ?? "");
	const written = useRef(canonical(params));

	useEffect(() => {
		const next = parseBrowseParams(searchParams);
		const current = canonical(next);
		if (current === written.current) return;
		written.current = current;
		setParams(next);
		setText(next.q ?? "");
	}, [searchParams]);

	useEffect(() => {
		const timeout = setTimeout(() => {
			const q = exploreSearchText(text);
			setParams((previous) =>
				previous.q === q ? previous : { ...previous, q },
			);
		}, DEBOUNCE_MS);
		return () => clearTimeout(timeout);
	}, [text]);

	useEffect(() => {
		const next = canonical(params);
		if (next === written.current) return;
		written.current = next;
		router.replace(next ? `${pathname}?${next}` : pathname, { scroll: false });
	}, [params, pathname, router]);

	const commitText = useCallback((value: string) => {
		setText(value);
		const q = exploreSearchText(value);
		setParams((previous) => (previous.q === q ? previous : { ...previous, q }));
	}, []);

	return { params, setParams, text, setText, commitText };
}

type Group = "apps" | "packages";

interface MorePages {
	key: string;
	apps: ExploreAppItem[];
	packages: ExploreSearchPackageHit[];
	appsHasMore?: boolean;
	packagesHasMore?: boolean;
}

function dedupe<T>(items: readonly T[], id: (item: T) => string): T[] {
	const seen = new Set<string>();
	return items.filter((item) => {
		const key = id(item);
		if (seen.has(key)) return false;
		seen.add(key);
		return true;
	});
}

/**
 * "Show more" per group: later pages of one group, appended to the first response. Keyed like the first page (viewer
 * identity plus params), so pages loaded for another hub, profile, locale or auth state are never appended.
 */
function useMoreResults(
	params: ExploreBrowseParams,
	first: ExploreSearchResponse | undefined,
) {
	const { t } = useTranslation("store");
	const { backend, apiOrigin, profileId, signedIn, language, developerMode } =
		useExploreViewer();
	const key = JSON.stringify([
		apiOrigin,
		profileId,
		signedIn,
		language,
		developerMode,
		canonical(params),
	]);
	const empty: MorePages = { key, apps: [], packages: [] };
	const [pages, setPages] = useState<MorePages>(empty);
	const [loading, setLoading] = useState<Group | null>(null);
	const latestKey = useRef(key);
	const more = pages.key === key ? pages : empty;

	useEffect(() => {
		latestKey.current = key;
	}, [key]);

	const apps = dedupe(
		[...(first?.apps.items ?? []), ...more.apps],
		(item) => item.app.id,
	);
	const packages = dedupe(
		[...(first?.packages.items ?? []), ...more.packages],
		(hit) => hit.package.id,
	);
	const loaded = {
		apps: (first?.apps.items.length ?? 0) + more.apps.length,
		packages: (first?.packages.items.length ?? 0) + more.packages.length,
	};

	const load = async (group: Group) => {
		setLoading(group);
		try {
			const response = await backend.appState.searchExplore({
				...params,
				language,
				dev: developerMode,
				appsOffset: group === "apps" ? loaded.apps : 0,
				appsLimit: group === "apps" ? PAGE_SIZE : 1,
				packagesOffset: group === "packages" ? loaded.packages : 0,
				packagesLimit: group === "packages" ? PAGE_SIZE : 1,
			});
			if (latestKey.current !== key) return;
			setPages((previous) => {
				const base = previous.key === key ? previous : { ...empty };
				return group === "apps"
					? {
							...base,
							apps: [...base.apps, ...response.apps.items],
							appsHasMore: response.apps.hasMore,
						}
					: {
							...base,
							packages: [...base.packages, ...response.packages.items],
							packagesHasMore: response.packages.hasMore,
						};
			});
		} catch {
			toast.error(
				t("exploreLoadMoreFailed", "More results could not be loaded."),
			);
		} finally {
			setLoading(null);
		}
	};

	return {
		apps,
		packages,
		appsHasMore: more.appsHasMore ?? first?.apps.hasMore ?? false,
		packagesHasMore: more.packagesHasMore ?? first?.packages.hasMore ?? false,
		loading,
		load,
	};
}

/**
 * Earlier results stay on screen while the query or filters change, but never across a change of collection:
 * opening a collection must not show the previous search (or another collection) under its header.
 */
function useShownResponse(
	search: ReturnType<typeof useExploreSearch>,
	collection: string | undefined,
): ExploreSearchResponse | undefined {
	const current = collection ?? null;
	const [settled, setSettled] = useState<string | null>(current);
	if (search.data && !search.isPlaceholderData && settled !== current) {
		setSettled(current);
	}
	return search.isPlaceholderData && settled !== current
		? undefined
		: search.data;
}

function BrowseContent({
	canScaffold = false,
	onUnsupported,
}: Readonly<
	ExploreBrowsePageProps & {
		onUnsupported: () => void;
	}
>) {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	const { developerMode } = useDeveloperMode();
	const browse = useBrowseParams();
	const { setParams, text, setText, commitText } = browse;
	const params = visibleParams(browse.params, developerMode);
	const { recent, remember } = useRecentSearches();
	const limit = params.collection ? COLLECTION_PAGE_SIZE : PAGE_SIZE;
	const search = useExploreSearch({
		...params,
		appsLimit: limit,
		packagesLimit: limit,
	});
	const response = useShownResponse(search, params.collection);
	const results = useMoreResults(params, response);
	const chips = useActiveChips(params, setParams);
	const dev = !!response?.viewer.dev && developerMode;
	const openedCollection = params.collection
		? response?.collections.find(
				(collection) => collection.placementId === params.collection,
			)
		: undefined;
	const clearFilters = () =>
		setParams({
			q: params.q,
			sort: params.sort,
			collection: params.collection,
		});
	const submit = (value: string) => {
		commitText(value);
		remember(exploreSearchText(value));
	};
	const unsupported = !response && isExploreUnsupportedError(search.error);

	useEffect(() => {
		if (unsupported) onUnsupported();
	}, [unsupported, onUnsupported]);

	const title = params.collection
		? (openedCollection?.title ?? t("exploreCollection", "Collection"))
		: t("exploreSearchTitle", "Search");

	const header = (
		<BrowseHeader
			title={title}
			text={text}
			onText={setText}
			onSubmit={submit}
			recent={recent}
			current={params.q}
			collection={!!params.collection}
			dev={dev}
		/>
	);

	if (!response) {
		if (search.isError && !unsupported) {
			const unavailable =
				!!params.collection && exploreErrorStatus(search.error) === 404;
			return (
				<ExploreShell>
					<div className="flex flex-col gap-6">
						{header}
						{unavailable ? (
							<CollectionUnavailable />
						) : (
							<ExploreQueryError
								error={search.error}
								onRetry={() => void search.refetch()}
								retrying={search.isFetching}
								fallbackLink={{
									label: t("exploreBackToExplore", "Back to Explore"),
									href: EXPLORE_PATH,
								}}
							/>
						)}
					</div>
				</ExploreShell>
			);
		}
		return (
			<ExploreShell>
				<div className="flex flex-col gap-6">
					{header}
					<BrowseSkeleton />
				</div>
			</ExploreShell>
		);
	}

	const type = params.type ?? "all";
	const inCollection = !!params.collection;
	const showCollections =
		!inCollection && (type === "all" || type === "collections");
	const showApps = inCollection || type === "all" || type === "apps";
	const showPackages =
		dev && (inCollection || type === "all" || type === "packages");
	const collections = showCollections ? response.collections : [];
	const apps = showApps ? results.apps : [];
	const packages = showPackages ? results.packages : [];
	const total =
		(showApps ? response.apps.total : 0) +
		(showPackages ? response.packages.total : 0) +
		collections.length;
	const filtersOnly = activeFilterCount({ ...params, type: undefined });
	const showSuites =
		!inCollection &&
		!params.q &&
		filtersOnly === 0 &&
		(type === "all" || type === "apps");
	const nothing = !collections.length && !apps.length && !packages.length;
	const paired =
		apps.length > 0 && apps.length <= PAIRED_MAX_APPS && packages.length > 0;

	return (
		<ExploreShell>
			<div className="flex flex-col gap-6">
				{header}
				<div className="flex min-w-0 gap-8">
					<BrowseFacetsSidebar
						facets={response.facets}
						params={params}
						dev={dev}
						onChange={setParams}
						onClear={clearFilters}
					/>
					<section
						aria-label={t("exploreResults", "Results")}
						aria-busy={search.isFetching}
						className="flex min-w-0 flex-1 flex-col gap-6"
					>
						<div className="flex flex-wrap items-center justify-between gap-3">
							<h2 className="text-xl font-semibold tracking-tight @3xl/explore:text-[22px]">
								{params.q
									? t("exploreResultsForQuery", {
											count: total,
											query: params.q,
											defaultValue_one: "{{count}} result for “{{query}}”",
											defaultValue_other: "{{count}} results for “{{query}}”",
										})
									: t("exploreResultCount", {
											count: total,
											defaultValue_one: "{{count}} result",
											defaultValue_other: "{{count}} results",
										})}
							</h2>
							<div className="flex items-center gap-2">
								<BrowseFacetsSheet
									facets={response.facets}
									params={params}
									dev={dev}
									onChange={setParams}
									onClear={clearFilters}
								/>
								<span className="hidden text-[13px] text-muted-foreground @xl/explore:inline">
									{t("exploreSort", "Sort")}
								</span>
								<Select
									value={params.sort ?? "best"}
									onValueChange={(value) => {
										const sort = SORTS.find((entry) => entry === value);
										setParams({
											...params,
											sort: sort === "best" ? undefined : sort,
										});
									}}
								>
									<SelectTrigger
										aria-label={t("sortResults", "Sort results")}
										className="h-9 w-44 rounded-lg"
									>
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										{SORTS.map((sort) => (
											<SelectItem key={sort} value={sort}>
												{labels.sort(sort)}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
							</div>
						</div>
						<ActiveChips chips={chips} />
						{openedCollection && (
							<BrowseCollectionCard
								collection={openedCollection}
								open={false}
							/>
						)}
						{showSuites && <SuitesRail />}
						{collections.map((collection) => (
							<BrowseCollectionCard
								key={collection.placementId}
								collection={collection}
								query={params.q}
							/>
						))}
						<div className={paired ? PAIRED_GROUPS : "contents"}>
							{apps.length > 0 && (
								<BrowseAppsGroup
									items={apps}
									total={response.apps.total}
									query={params.q}
									hasMore={results.appsHasMore}
									loadingMore={results.loading === "apps"}
									onMore={() => void results.load("apps")}
									beside={paired}
								/>
							)}
							{packages.length > 0 && (
								<BrowsePackagesGroup
									hits={packages}
									total={response.packages.total}
									query={params.q}
									capped={response.facets.packagesCapped}
									hasMore={results.packagesHasMore}
									loadingMore={results.loading === "packages"}
									onMore={() => void results.load("packages")}
									beside={paired}
								/>
							)}
						</div>
						{nothing && (
							<BrowseEmpty
								query={params.q}
								hasFilters={activeFilterCount(params) > 0}
								onClear={clearFilters}
							/>
						)}
						<BrowseRelated items={response.related} query={params.q} />
						{dev && canScaffold && <BuildItYourselfCard />}
					</section>
				</div>
			</div>
		</ExploreShell>
	);
}

function BrowseHeader({
	title,
	text,
	onText,
	onSubmit,
	recent,
	current,
	collection,
	dev,
}: Readonly<{
	title: string;
	text: string;
	onText: (value: string) => void;
	onSubmit: (value: string) => void;
	recent: readonly string[];
	current?: string;
	collection: boolean;
	dev: boolean;
}>) {
	const { t } = useTranslation("store");
	const inputRef = useRef<HTMLInputElement>(null);
	return (
		<header className="flex flex-col gap-4 @4xl/explore:flex-row @4xl/explore:items-center">
			<div className="flex min-w-0 items-center gap-3 @4xl/explore:w-60 @4xl/explore:shrink-0">
				<Link
					href={EXPLORE_PATH}
					aria-label={t("exploreBackToExplore", "Back to Explore")}
					className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-border bg-card text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					<ArrowLeft aria-hidden="true" className="size-4" />
				</Link>
				<div className="flex min-w-0 flex-col">
					<span className="flex items-center gap-1 text-xs text-muted-foreground">
						<Link href={EXPLORE_PATH} className="hover:text-foreground">
							{t("explore", "Explore")}
						</Link>
						<span aria-hidden="true">/</span>
						{collection ? (
							<Layers aria-hidden="true" className="size-3" />
						) : (
							t("exploreSearchTitle", "Search")
						)}
					</span>
					<h1 className="truncate text-lg font-semibold leading-6 tracking-tight">
						{title}
					</h1>
				</div>
			</div>
			<form
				className="relative min-w-0 flex-1 @4xl/explore:max-w-160"
				onSubmit={(event) => {
					event.preventDefault();
					onSubmit(text);
				}}
			>
				<Search
					aria-hidden="true"
					className="pointer-events-none absolute left-3.5 top-1/2 z-10 size-4 -translate-y-1/2 text-muted-foreground"
				/>
				<input
					ref={inputRef}
					type="search"
					value={text}
					onChange={(event) => onText(event.target.value)}
					placeholder={
						dev
							? t("exploreSearchPlaceholderDev", "Search apps and packages")
							: t("exploreSearchPlaceholder", "Search apps")
					}
					aria-label={t("exploreSearchLabel", "Search Explore")}
					className="h-11 w-full rounded-xl border border-border bg-card pl-10 pr-11 text-sm placeholder:text-muted-foreground focus-visible:border-primary/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/30 [&::-webkit-search-cancel-button]:appearance-none"
				/>
				{text && (
					<button
						type="button"
						aria-label={t("clearSearch", "Clear search")}
						onClick={() => {
							onSubmit("");
							inputRef.current?.focus();
						}}
						className="absolute right-1.5 top-1/2 flex size-8 -translate-y-1/2 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
					>
						<X aria-hidden="true" className="size-4" />
					</button>
				)}
			</form>
			<div className="@4xl/explore:ml-auto">
				<RecentSearches recent={recent} current={current} onPick={onSubmit} />
			</div>
		</header>
	);
}

function CollectionUnavailable() {
	const { t } = useTranslation("store");
	return (
		<div className="flex flex-col items-center gap-3 rounded-2xl border border-dashed border-border px-6 py-16 text-center">
			<span className="flex size-11 items-center justify-center rounded-xl bg-muted">
				<Layers aria-hidden="true" className="size-5 text-muted-foreground" />
			</span>
			<h2 className="text-base font-semibold">
				{t("exploreCollectionUnavailable", "This collection is not available")}
			</h2>
			<p className="max-w-sm text-sm text-muted-foreground">
				{t(
					"exploreCollectionUnavailableBody",
					"It may have ended or been taken down. Explore has plenty more.",
				)}
			</p>
			<Link
				href={EXPLORE_PATH}
				className="text-sm font-semibold text-primary hover:underline"
			>
				{t("exploreBackToExplore", "Back to Explore")}
			</Link>
		</div>
	);
}

function BrowseSkeleton() {
	return (
		<div aria-hidden="true" className="flex gap-8">
			<div className="hidden w-60 shrink-0 flex-col gap-2 @5xl/explore:flex">
				{[0, 1, 2, 3, 4, 5, 6, 7].map((index) => (
					<Skeleton key={index} className="h-7 rounded-md" />
				))}
			</div>
			<div className="flex min-w-0 flex-1 flex-col gap-4">
				<Skeleton className="h-7 w-64 rounded-full" />
				<Skeleton className="h-36 rounded-2xl" />
				<div className={RESULT_GRID}>
					{[0, 1, 2].map((index) => (
						<Skeleton key={index} className="h-95 rounded-xl" />
					))}
				</div>
			</div>
		</div>
	);
}

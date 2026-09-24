"use client";

import { useTranslation } from "@flow-like/locales";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useDebounce } from "@uidotdev/usehooks";
import {
	Database,
	FileCode2,
	Folder,
	Globe,
	KeyRound,
	LayoutGrid,
	Loader2,
	Package,
	Search,
	Sparkles,
	Undo2,
	WifiOff,
} from "lucide-react";
import {
	type ComponentType,
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useInvoke } from "../../hooks/use-invoke";
import { useSearch } from "../../hooks/use-search-index";
import { asArray } from "../../lib/response-shape";
import type { InstalledPackage, SearchResults } from "../../lib/schema/wasm";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { EmptyState } from "../ui/empty-state";
import { Input } from "../ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { Skeleton } from "../ui/skeleton";
import {
	ShelfCard,
	type ShelfCardState,
	useShelfFacetLabel,
} from "./package-shelf-card";
import {
	SHELF_ACCESS_FACETS,
	type ShelfAccessFacet,
	type ShelfKind,
	type ShelfPackage,
	type ShelfSort,
	hasFacet,
	hasTopic,
	shelfPackageFromInstalled,
	shelfPackageFromSummary,
	shelfTopics,
	sortShelf,
} from "./package-shelf-model";

export interface PackageSearchDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	/** Awaited when it returns a promise; the dialog stays open so several can be added. */
	onSelect: (packageId: string, version: string) => unknown;
	/** Unlinks a package added while the dialog is open; enables undo. */
	onRemove?: (packageId: string) => unknown;
	excludePackageIds?: string[];
	appId?: string;
}

const PAGE_SIZE = 50;
const SORTS: readonly ShelfSort[] = ["relevance", "name", "least", "most"];

const FACET_ICONS: Record<
	ShelfAccessFacet,
	ComponentType<{ className?: string }>
> = {
	network: Globe,
	files: Folder,
	accounts: KeyRound,
	models: Sparkles,
	data: Database,
	widgets: LayoutGrid,
};

interface SessionEntry {
	id: string;
	name: string;
}

type PendingAction = "adding" | "removing";

const byName = (a: ShelfPackage, b: ShelfPackage) =>
	a.name.localeCompare(b.name, undefined, { sensitivity: "base" });

export function PackageSearchDialog({
	open,
	onOpenChange,
	onSelect,
	onRemove,
	excludePackageIds = [],
	appId,
}: Readonly<PackageSearchDialogProps>) {
	const { t } = useTranslation("store");
	const backend = useBackend();
	const [search, setSearch] = useState("");
	const debouncedSearch = useDebounce(search, 300);
	const [kind, setKind] = useState<ShelfKind>("packages");
	const [access, setAccess] = useState<ShelfAccessFacet | null>(null);
	const [topic, setTopic] = useState<string | null>(null);
	const [sort, setSort] = useState<ShelfSort>("relevance");
	const [session, setSession] = useState<SessionEntry[]>([]);
	const [pending, setPending] = useState<ReadonlyMap<string, PendingAction>>(
		() => new Map(),
	);

	useEffect(() => {
		if (open) setSession([]);
	}, [open]);

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const isOffline = useQuery<boolean>({
		queryKey: ["app-offline", appId],
		queryFn: () => backend.isOffline(appId ?? ""),
		enabled: !!appId && open,
	});
	const offline = isOffline.data === true;

	const remote = useInfiniteQuery<SearchResults>({
		queryKey: ["registry-search-dialog", debouncedSearch],
		initialPageParam: 0,
		queryFn: async ({ pageParam }) => {
			if (!profile.data) throw new Error("Profile not loaded");
			const params = new URLSearchParams();
			if (debouncedSearch) params.set("query", debouncedSearch);
			params.set("limit", String(PAGE_SIZE));
			params.set("offset", String(pageParam));
			params.set("include_own", "true");
			return backend.apiState.get<SearchResults>(
				profile.data,
				`registry/search?${params.toString()}`,
			);
		},
		getNextPageParam: (last) => {
			const next = (last?.offset ?? 0) + asArray(last?.packages).length;
			return next < (last?.totalCount ?? 0) ? next : undefined;
		},
		enabled: !!profile.data && open && !offline,
	});

	const localPackages = useQuery<InstalledPackage[]>({
		queryKey: ["local-installed-packages"],
		queryFn: () => backend.registryState.getInstalledPackages(),
		enabled: open && offline,
	});

	const remoteItems = useMemo(
		() =>
			asArray(remote.data?.pages)
				.flatMap((page) => asArray(page?.packages))
				.map(shelfPackageFromSummary),
		[remote.data],
	);
	const localItems = useMemo(
		() =>
			offline
				? asArray(localPackages.data)
						.map(shelfPackageFromInstalled)
						.sort(byName)
				: [],
		[offline, localPackages.data],
	);
	// Online the registry does the searching; offline we index locally.
	const localMatches = useSearch(localItems, debouncedSearch, {
		fields: ["name", "id", "description", "keywords", "authors"],
		boost: { name: 3, id: 2, keywords: 1.5 },
	});
	const searched = offline ? localMatches : remoteItems;

	const templateCount = useMemo(
		() => searched.filter((p) => p.template).length,
		[searched],
	);
	const activeKind: ShelfKind = templateCount === 0 ? "packages" : kind;
	const kindItems = useMemo(
		() => searched.filter((p) => p.template === (activeKind === "templates")),
		[searched, activeKind],
	);
	const byTopic = useMemo(
		() => (topic ? kindItems.filter((p) => hasTopic(p, topic)) : kindItems),
		[kindItems, topic],
	);
	const byAccess = useMemo(
		() => (access ? kindItems.filter((p) => hasFacet(p, access)) : kindItems),
		[kindItems, access],
	);
	const visible = useMemo(
		() =>
			sortShelf(
				access ? byTopic.filter((p) => hasFacet(p, access)) : byTopic,
				sort,
			),
		[byTopic, access, sort],
	);

	const accessOptions = useMemo(
		() =>
			SHELF_ACCESS_FACETS.map((facet) => ({
				facet,
				count: byTopic.filter((p) => hasFacet(p, facet)).length,
			})).filter((o) => o.count > 0 || o.facet === access),
		[byTopic, access],
	);
	const topicOptions = useMemo(() => {
		const topics = shelfTopics(kindItems);
		if (topic && !topics.includes(topic)) topics.push(topic);
		return topics
			.map((value) => ({
				topic: value,
				count: byAccess.filter((p) => hasTopic(p, value)).length,
			}))
			.filter((o) => o.count > 0 || o.topic === topic);
	}, [kindItems, byAccess, topic]);

	const excludeSet = useMemo(
		() => new Set(excludePackageIds),
		[excludePackageIds],
	);
	const sessionIds = useMemo(
		() => new Set(session.map((entry) => entry.id)),
		[session],
	);

	const setPendingAction = useCallback(
		(id: string, action: PendingAction | null) =>
			setPending((prev) => {
				const next = new Map(prev);
				if (action) next.set(id, action);
				else next.delete(id);
				return next;
			}),
		[],
	);

	const addPackage = useCallback(
		async (pkg: ShelfPackage) => {
			setPendingAction(pkg.id, "adding");
			try {
				await onSelect(pkg.id, pkg.version);
				setSession((prev) => [
					...prev.filter((entry) => entry.id !== pkg.id),
					{ id: pkg.id, name: pkg.name },
				]);
			} catch {
				// The caller already reported why the package could not be added.
			} finally {
				setPendingAction(pkg.id, null);
			}
		},
		[onSelect, setPendingAction],
	);

	const removeFromSession = useCallback(
		async (id: string) => {
			if (!onRemove) return;
			setPendingAction(id, "removing");
			try {
				await onRemove(id);
				setSession((prev) => prev.filter((entry) => entry.id !== id));
			} catch {
				// The caller already reported why the package could not be removed.
			} finally {
				setPendingAction(id, null);
			}
		},
		[onRemove, setPendingAction],
	);

	const removePackage = useCallback(
		(pkg: ShelfPackage) => removeFromSession(pkg.id),
		[removeFromSession],
	);
	const closeDialog = useCallback(() => onOpenChange(false), [onOpenChange]);

	const cardState = (pkg: ShelfPackage): ShelfCardState => {
		const action = pending.get(pkg.id);
		if (action) return action;
		if (sessionIds.has(pkg.id)) return "added";
		return excludeSet.has(pkg.id) ? "inProject" : "idle";
	};

	const clearFilters = () => {
		setAccess(null);
		setTopic(null);
		setSearch("");
	};

	const selectKind = (next: ShelfKind) => {
		setKind(next);
		setTopic(null);
	};

	const sentinelRef = useRef<HTMLDivElement | null>(null);
	useEffect(() => {
		if (offline) return;
		const node = sentinelRef.current;
		if (!node) return;
		if (!remote.hasNextPage || remote.isFetchingNextPage) return;

		const observer = new IntersectionObserver(
			(entries) => {
				if (entries.some((e) => e.isIntersecting)) remote.fetchNextPage();
			},
			{ rootMargin: "200px" },
		);
		observer.observe(node);
		return () => observer.disconnect();
	}, [
		remote.hasNextPage,
		remote.isFetchingNextPage,
		remote.fetchNextPage,
		offline,
	]);

	const initialLoading = offline ? localPackages.isLoading : remote.isLoading;
	const hasMore = !offline && remote.hasNextPage;
	const totalRemote = remote.data?.pages[0]?.totalCount ?? 0;
	const countLabel =
		!offline && totalRemote > 0
			? t("showingPackages", "Showing {{shown}} of {{total}} packages", {
					shown: visible.length.toLocaleString(),
					total: totalRemote.toLocaleString(),
				})
			: t("packageShelfCount", "{{count}} packages", {
					count: visible.length,
				});

	const sortLabels: Record<ShelfSort, string> = {
		relevance: t("packageShelfSortRelevance", "Best match"),
		name: t("packageShelfSortName", "Name"),
		least: t("packageShelfSortLeast", "Least access first"),
		most: t("packageShelfSortMost", "Most access first"),
	};

	let body: ReactNode;
	if (initialLoading) {
		body = <ShelfSkeleton />;
	} else if (searched.length === 0) {
		body = (
			<EmptyState
				icons={[Package]}
				title={t("noResults", "No results")}
				description={
					debouncedSearch
						? t("tryADifferentSearchTerm", "Try a different search term.")
						: offline
							? t(
									"noLocalPackagesFoundLoadAWasmPackageFirst",
									"No local packages found. Load a WASM package first.",
								)
							: t("typeToSearchForPackages", "Type to search for packages.")
				}
			/>
		);
	} else if (visible.length === 0) {
		body = (
			<div className="flex flex-col items-center gap-2 px-6 py-20 text-center">
				<p className="text-sm font-semibold">
					{t("packageShelfNoMatchTitle", "No packages match these filters")}
				</p>
				<p className="text-sm text-muted-foreground">
					{t(
						"packageShelfNoMatchDescription",
						"Loosen a filter on the left or change the search.",
					)}
				</p>
				<Button
					size="sm"
					variant="outline"
					className="mt-2"
					onClick={clearFilters}
				>
					{t("packageShelfClearFilters", "Clear filters")}
				</Button>
			</div>
		);
	} else {
		body = (
			<div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2 xl:grid-cols-3">
				{visible.map((pkg) => (
					<ShelfCard
						key={pkg.id}
						pkg={pkg}
						state={cardState(pkg)}
						canRemove={!!onRemove}
						onAdd={addPackage}
						onRemove={removePackage}
						onGetFirst={closeDialog}
					/>
				))}
			</div>
		);
	}

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="flex h-[min(52rem,calc(100dvh-2rem))] flex-col gap-0 overflow-hidden p-0 sm:max-w-6xl">
				<DialogHeader className="border-b px-6 py-5 pr-12">
					<DialogTitle>{t("packageShelfTitle", "Add packages")}</DialogTitle>
					<DialogDescription className="flex items-center gap-2">
						{offline && <WifiOff className="size-3.5 shrink-0" />}
						{offline
							? t(
									"packageShelfOfflineDescription",
									"Showing packages installed on this device. The registry is unavailable while this project is offline.",
								)
							: t(
									"packageShelfOnlineDescription",
									"Search the registry and add as many packages as you need.",
								)}
					</DialogDescription>
				</DialogHeader>

				<div className="flex min-h-0 flex-1">
					<ShelfRail
						kind={activeKind}
						kindCounts={
							templateCount > 0
								? {
										packages: searched.length - templateCount,
										templates: templateCount,
									}
								: undefined
						}
						onKind={selectKind}
						access={access}
						accessOptions={accessOptions}
						onAccess={(facet) => setAccess(facet === access ? null : facet)}
						topic={topic}
						topicOptions={topicOptions}
						onTopic={(value) => setTopic(value === topic ? null : value)}
					/>

					<div className="flex min-w-0 flex-1 flex-col">
						<div className="flex shrink-0 flex-wrap items-center gap-2 px-5 pt-4 pb-3">
							<div className="relative min-w-48 flex-1">
								<Search className="absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
								<Input
									placeholder={t("searchPackages", "Search packages...")}
									aria-label={t("searchPackages", "Search packages...")}
									value={search}
									onChange={(e) => setSearch(e.target.value)}
									className="pl-9"
									autoFocus
								/>
							</div>
							<Select
								value={sort}
								onValueChange={(value) => setSort(value as ShelfSort)}
							>
								<SelectTrigger
									className="w-48"
									aria-label={t("sortBy", "Sort by")}
								>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{SORTS.map((value) => (
										<SelectItem key={value} value={value}>
											{sortLabels[value]}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							<span className="min-w-24 text-right text-xs tabular-nums text-muted-foreground">
								{countLabel}
							</span>
						</div>

						<div className="min-h-0 flex-1 overflow-y-auto px-5 pb-5">
							{body}
							{hasMore && (
								<div
									ref={sentinelRef}
									className="flex items-center justify-center py-4 text-xs text-muted-foreground"
								>
									{remote.isFetchingNextPage ? (
										<span className="inline-flex items-center gap-2">
											<Loader2 className="size-3 animate-spin" />
											{t("loadingMore", "Loading more...")}
										</span>
									) : (
										t("scrollForMore", "Scroll for more")
									)}
								</div>
							)}
						</div>
					</div>
				</div>

				<ShelfFooter
					session={session}
					canUndo={!!onRemove}
					undoPending={
						session.length > 0 && pending.has(session[session.length - 1].id)
					}
					onUndo={() => {
						const last = session.at(-1);
						if (last) removeFromSession(last.id);
					}}
					onClose={closeDialog}
				/>
			</DialogContent>
		</Dialog>
	);
}

interface RailOptionItem {
	key: string;
	label: string;
	count: number;
	active: boolean;
	icon?: ReactNode;
	mono?: boolean;
	onSelect: () => void;
}

function ShelfRail({
	kind,
	kindCounts,
	onKind,
	access,
	accessOptions,
	onAccess,
	topic,
	topicOptions,
	onTopic,
}: Readonly<{
	kind: ShelfKind;
	kindCounts?: Record<ShelfKind, number>;
	onKind: (kind: ShelfKind) => void;
	access: ShelfAccessFacet | null;
	accessOptions: { facet: ShelfAccessFacet; count: number }[];
	onAccess: (facet: ShelfAccessFacet) => void;
	topic: string | null;
	topicOptions: { topic: string; count: number }[];
	onTopic: (topic: string) => void;
}>) {
	const { t } = useTranslation("store");
	const facetLabel = useShelfFacetLabel();

	const kindItems: RailOptionItem[] = kindCounts
		? [
				{
					key: "packages",
					label: t("packageShelfPackages", "Packages"),
					count: kindCounts.packages,
					active: kind === "packages",
					icon: <Package className="size-4" />,
					onSelect: () => onKind("packages"),
				},
				{
					key: "templates",
					label: t("packageShelfTemplates", "Templates"),
					count: kindCounts.templates,
					active: kind === "templates",
					icon: <FileCode2 className="size-4" />,
					onSelect: () => onKind("templates"),
				},
			]
		: [];

	const accessItems: RailOptionItem[] = accessOptions.map(
		({ facet, count }) => {
			const Icon = FACET_ICONS[facet];
			return {
				key: facet,
				label: facetLabel(facet),
				count,
				active: access === facet,
				icon: <Icon className="size-4" />,
				onSelect: () => onAccess(facet),
			};
		},
	);

	const topicItems: RailOptionItem[] = topicOptions.map(
		({ topic: value, count }) => ({
			key: value,
			label: value,
			count,
			active: topic === value,
			mono: true,
			onSelect: () => onTopic(value),
		}),
	);

	return (
		<nav
			aria-label={t("packageShelfFilters", "Filter packages")}
			className="hidden w-60 shrink-0 flex-col gap-5 overflow-y-auto border-r bg-muted/20 px-3 py-4 md:flex"
		>
			<RailGroup title={t("packageShelfShow", "Show")} items={kindItems} />
			<RailGroup
				title={t("packageShelfCanAccess", "Can access")}
				items={accessItems}
			/>
			<RailGroup title={t("packageShelfTopics", "Topics")} items={topicItems} />
		</nav>
	);
}

function RailGroup({
	title,
	items,
}: Readonly<{ title: string; items: RailOptionItem[] }>) {
	if (items.length === 0) return null;
	return (
		<div className="flex flex-col gap-0.5">
			<h3 className="mb-1.5 px-2.5 text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">
				{title}
			</h3>
			{items.map((item) => (
				<button
					key={item.key}
					type="button"
					aria-pressed={item.active}
					onClick={item.onSelect}
					className={cn(
						"flex h-8 w-full items-center gap-2.5 rounded-md px-2.5 text-left text-sm transition-colors",
						item.active
							? "bg-primary/10 text-primary"
							: item.count > 0
								? "text-foreground/90 hover:bg-muted"
								: "text-muted-foreground hover:bg-muted",
					)}
				>
					{item.icon}
					<span
						className={cn(
							"min-w-0 flex-1 truncate",
							item.mono && "font-mono text-xs",
						)}
					>
						{item.label}
					</span>
					<span
						className={cn(
							"font-mono text-[11px] tabular-nums",
							item.active ? "text-primary" : "text-muted-foreground",
						)}
					>
						{item.count}
					</span>
				</button>
			))}
		</div>
	);
}

function ShelfFooter({
	session,
	canUndo,
	undoPending,
	onUndo,
	onClose,
}: Readonly<{
	session: SessionEntry[];
	canUndo: boolean;
	undoPending: boolean;
	onUndo: () => void;
	onClose: () => void;
}>) {
	const { t } = useTranslation("store");
	const hasSession = session.length > 0;
	return (
		<div className="flex shrink-0 items-center gap-3 border-t bg-muted/20 px-6 py-3">
			{hasSession ? (
				<output className="flex min-w-0 flex-1 items-center gap-2 overflow-hidden">
					<span className="shrink-0 text-xs text-muted-foreground">
						{t("packageShelfAddedJustNow", "Added just now")}
					</span>
					<span className="flex min-w-0 items-center gap-1.5 overflow-hidden">
						{session.map((entry) => (
							<span
								key={entry.id}
								className="shrink-0 truncate rounded-md bg-primary/10 px-2 py-1 text-xs font-medium text-primary"
							>
								{entry.name}
							</span>
						))}
					</span>
					{canUndo && (
						<Button
							size="sm"
							variant="ghost"
							className="shrink-0"
							disabled={undoPending}
							onClick={onUndo}
						>
							<Undo2 />
							{t("packageShelfUndoLast", "Undo last")}
						</Button>
					)}
				</output>
			) : (
				<p className="min-w-0 flex-1 text-xs text-muted-foreground">
					{t(
						"packageShelfHint",
						"Add as many as you need. Each package's nodes join the catalog as soon as it is added.",
					)}
				</p>
			)}
			<Button variant={hasSession ? "default" : "outline"} onClick={onClose}>
				{hasSession ? t("common:done", "Done") : t("common:close", "Close")}
			</Button>
		</div>
	);
}

function ShelfSkeleton() {
	return (
		<div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2 xl:grid-cols-3">
			{["a", "b", "c", "d", "e", "f"].map((k) => (
				<div
					key={k}
					className="flex flex-col overflow-hidden rounded-xl border border-border/70"
				>
					<Skeleton className="h-28 rounded-none" />
					<div className="space-y-2 p-4">
						<Skeleton className="h-4 w-40" />
						<Skeleton className="h-3 w-24" />
						<Skeleton className="h-3 w-full" />
						<Skeleton className="h-3 w-4/5" />
					</div>
				</div>
			))}
		</div>
	);
}

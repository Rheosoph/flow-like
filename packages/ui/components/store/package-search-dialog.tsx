"use client";

import { useTranslation } from "@flow-like/locales";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useDebounce } from "@uidotdev/usehooks";
import { Download, HardDrive, Loader2, Package, Search } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useInvoke } from "../../hooks/use-invoke";
import { useSearch } from "../../hooks/use-search-index";
import {
	type InstalledPackage,
	PackageStatus,
	type PackageSummary,
	type SearchResults,
} from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import { Badge } from "../ui/badge";
import {
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { EmptyState } from "../ui/empty-state";
import { Input } from "../ui/input";
import { Skeleton } from "../ui/skeleton";

export interface PackageSearchDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onSelect: (packageId: string, version: string) => void;
	excludePackageIds?: string[];
	appId?: string;
}

const PAGE_SIZE = 50;

function installedToSummary(pkg: InstalledPackage): PackageSummary {
	return {
		id: pkg.manifest.id ?? pkg.id,
		name: pkg.manifest.name,
		description: pkg.manifest.description,
		latestVersion: pkg.manifest.version ?? pkg.version,
		downloadCount: 0,
		status: PackageStatus.Active,
		keywords: pkg.manifest.keywords ?? [],
		verified: false,
		price: 0,
		visibility: pkg.source.type === "local" ? "local" : "private",
	};
}

export function PackageSearchDialog({
	open,
	onOpenChange,
	onSelect,
	excludePackageIds = [],
	appId,
}: PackageSearchDialogProps) {
	const { t } = useTranslation("store");
	const backend = useBackend();
	const [search, setSearch] = useState("");
	const debouncedSearch = useDebounce(search, 300);

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
			const next = last.offset + last.packages.length;
			return next < last.totalCount ? next : undefined;
		},
		enabled: !!profile.data && open && !isOffline.data,
	});

	const localPackages = useQuery<InstalledPackage[]>({
		queryKey: ["local-installed-packages"],
		queryFn: () => backend.registryState.getInstalledPackages(),
		enabled: open && isOffline.data === true,
	});

	const remotePackages = useMemo<PackageSummary[]>(
		() => remote.data?.pages.flatMap((p) => p.packages) ?? [],
		[remote.data],
	);

	const totalRemote = remote.data?.pages[0]?.totalCount ?? 0;

	// Online the registry does the searching; offline we index locally.
	const offlineCandidates = useMemo<PackageSummary[]>(() => {
		if (!isOffline.data) return [];
		const remoteIds = new Set(remotePackages.map((p) => p.id));
		const localSummaries = (localPackages.data ?? [])
			.map(installedToSummary)
			.filter((p) => !remoteIds.has(p.id));
		return [...remotePackages, ...localSummaries];
	}, [remotePackages, localPackages.data, isOffline.data]);

	const offlineMatches = useSearch(offlineCandidates, debouncedSearch, {
		fields: ["name", "id", "description", "keywords"],
		boost: { name: 3, id: 2, keywords: 1.5 },
	});

	const mergedPackages = isOffline.data ? offlineMatches : remotePackages;

	const excludeSet = useMemo(
		() => new Set(excludePackageIds),
		[excludePackageIds],
	);

	const sentinelRef = useRef<HTMLDivElement | null>(null);
	useEffect(() => {
		if (isOffline.data) return;
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
		isOffline.data,
	]);

	const initialLoading =
		(remote.isLoading && !isOffline.data) ||
		(localPackages.isLoading && isOffline.data === true);
	const hasMore = !isOffline.data && remote.hasNextPage;

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-xl">
				<DialogHeader>
					<DialogTitle>{t("addPackage", "Add Package")}</DialogTitle>
					<DialogDescription>
						{isOffline.data
							? t(
									"addALocallyInstalledPackageToThisProject",
									"Add a locally installed package to this project.",
								)
							: t(
									"searchTheRegistryAndSelectAPackageToAdd",
									"Search the registry and select a package to add.",
								)}
					</DialogDescription>
				</DialogHeader>
				<div className="relative shrink-0">
					<Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
					<Input
						placeholder={t("searchPackages", "Search packages...")}
						value={search}
						onChange={(e) => setSearch(e.target.value)}
						className="pl-9"
						autoFocus
					/>
				</div>
				{!isOffline.data && totalRemote > 0 && (
					<div className="shrink-0 px-1 text-[11px] text-muted-foreground">
						{t("showingPackages", "Showing {{shown}} of {{total}} packages", {
							shown: mergedPackages.length.toLocaleString(),
							total: totalRemote.toLocaleString(),
						})}
					</div>
				)}
				<DialogBody>
					{initialLoading ? (
						<SearchResultsSkeleton />
					) : !mergedPackages.length ? (
						<EmptyState
							icons={[Package]}
							title={t("noResults", "No results")}
							description={
								debouncedSearch
									? t("tryADifferentSearchTerm", "Try a different search term.")
									: isOffline.data
										? t(
												"noLocalPackagesFoundLoadAWasmPackageFirst",
												"No local packages found. Load a WASM package first.",
											)
										: t(
												"typeToSearchForPackages",
												"Type to search for packages.",
											)
							}
						/>
					) : (
						<div className="space-y-1">
							{mergedPackages.map((pkg) => {
								const isExcluded = excludeSet.has(pkg.id);
								const isLocal = pkg.visibility === "local";
								return (
									<SearchResultItem
										key={pkg.id}
										name={pkg.name}
										description={pkg.description}
										latestVersion={pkg.latestVersion}
										downloadCount={pkg.downloadCount}
										disabled={isExcluded}
										isLocal={isLocal}
										onSelect={() => onSelect(pkg.id, pkg.latestVersion)}
									/>
								);
							})}
							{hasMore && (
								<div
									ref={sentinelRef}
									className="flex items-center justify-center py-3 text-xs text-muted-foreground"
								>
									{remote.isFetchingNextPage ? (
										<span className="inline-flex items-center gap-2">
											<Loader2 className="h-3 w-3 animate-spin" />
											{t("loadingMore", "Loading more...")}
										</span>
									) : (
										t("scrollForMore", "Scroll for more")
									)}
								</div>
							)}
						</div>
					)}
				</DialogBody>
			</DialogContent>
		</Dialog>
	);
}

function SearchResultItem({
	name,
	description,
	latestVersion,
	downloadCount,
	disabled,
	isLocal,
	onSelect,
}: {
	name: string;
	description: string;
	latestVersion: string;
	downloadCount: number;
	disabled: boolean;
	isLocal?: boolean;
	onSelect: () => void;
}) {
	const { t } = useTranslation("store");
	return (
		<button
			type="button"
			disabled={disabled}
			onClick={onSelect}
			className="flex w-full items-start gap-3 rounded-lg p-3 text-left transition-colors hover:bg-accent disabled:opacity-50 disabled:cursor-not-allowed"
		>
			{isLocal ? (
				<HardDrive className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
			) : (
				<Package className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
			)}
			<div className="min-w-0 flex-1">
				<div className="flex items-center gap-2">
					<span className="truncate text-sm font-medium">{name}</span>
					<Badge
						variant="outline"
						className="shrink-0 text-xs"
					>{`v${latestVersion}`}</Badge>
					{isLocal && (
						<Badge variant="secondary" className="shrink-0 text-xs">
							{t("local", "Local")}
						</Badge>
					)}
					{disabled && (
						<Badge variant="secondary" className="shrink-0 text-xs">
							{t("alreadyAdded", "Already added")}
						</Badge>
					)}
				</div>
				<p className="mt-0.5 truncate text-xs text-muted-foreground">
					{description}
				</p>
				{!isLocal && (
					<div className="mt-1 flex items-center gap-1 text-xs text-muted-foreground/60">
						<Download className="h-3 w-3" />
						{downloadCount.toLocaleString()} downloads
					</div>
				)}
			</div>
		</button>
	);
}

function SearchResultsSkeleton() {
	return (
		<div className="space-y-2 p-1">
			{["a", "b", "c", "d"].map((k) => (
				<div key={k} className="flex items-start gap-3 rounded-lg p-3">
					<Skeleton className="h-4 w-4 mt-0.5 rounded" />
					<div className="flex-1 space-y-2">
						<Skeleton className="h-4 w-40" />
						<Skeleton className="h-3 w-full" />
						<Skeleton className="h-3 w-20" />
					</div>
				</div>
			))}
		</div>
	);
}

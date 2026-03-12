"use client";

import { useQuery } from "@tanstack/react-query";
import { useDebounce } from "@uidotdev/usehooks";
import { Download, HardDrive, Package, Search } from "lucide-react";
import { useMemo, useState } from "react";
import { useInvoke } from "../../hooks/use-invoke";
import {
	PackageStatus,
	type InstalledPackage,
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

	const results = useQuery<SearchResults>({
		queryKey: ["registry-search-dialog", debouncedSearch],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const params = new URLSearchParams();
			if (debouncedSearch) params.set("query", debouncedSearch);
			params.set("limit", "20");
			params.set("include_own", "true");
			return backend.apiState.get<SearchResults>(
				profile.data,
				`registry/search?${params.toString()}`,
			);
		},
		enabled: !!profile.data && open && !isOffline.data,
	});

	const localPackages = useQuery<InstalledPackage[]>({
		queryKey: ["local-installed-packages"],
		queryFn: () => backend.registryState.getInstalledPackages(),
		enabled: open && isOffline.data === true,
	});

	const mergedPackages = useMemo<PackageSummary[]>(() => {
		const remote = results.data?.packages ?? [];
		if (!isOffline.data) return remote;

		const localSummaries = (localPackages.data ?? []).map(installedToSummary);
		const remoteIds = new Set(remote.map((p) => p.id));
		const merged = [...remote, ...localSummaries.filter((p) => !remoteIds.has(p.id))];

		if (!debouncedSearch) return merged;
		const q = debouncedSearch.toLowerCase();
		return merged.filter(
			(p) =>
				p.name.toLowerCase().includes(q) ||
				p.description.toLowerCase().includes(q) ||
				p.id.toLowerCase().includes(q),
		);
	}, [results.data, localPackages.data, isOffline.data, debouncedSearch]);

	const excludeSet = new Set(excludePackageIds);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-xl">
				<DialogHeader>
					<DialogTitle>Add Package</DialogTitle>
					<DialogDescription>
						Search the registry and select a package to add.
					</DialogDescription>
				</DialogHeader>
				<div className="relative shrink-0">
					<Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
					<Input
						placeholder="Search packages..."
						value={search}
						onChange={(e) => setSearch(e.target.value)}
						className="pl-9"
						autoFocus
					/>
				</div>
				<DialogBody>
					{results.isLoading || localPackages.isLoading ? (
						<SearchResultsSkeleton />
					) : !mergedPackages.length ? (
						<EmptyState
							icons={[Package]}
							title="No results"
							description={
								debouncedSearch
									? "Try a different search term."
									: isOffline.data
										? "No local packages found. Load a WASM package first."
										: "Type to search for packages."
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
					<Badge variant="outline" className="shrink-0 text-xs">
						v{latestVersion}
					</Badge>
					{isLocal && (
						<Badge variant="secondary" className="shrink-0 text-xs">
							Local
						</Badge>
					)}
					{disabled && (
						<Badge variant="secondary" className="shrink-0 text-xs">
							Already added
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
			{Array.from({ length: 4 }).map((_, i) => (
				<div key={i} className="flex items-start gap-3 rounded-lg p-3">
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

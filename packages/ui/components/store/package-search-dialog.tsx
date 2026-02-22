"use client";

import { useQuery } from "@tanstack/react-query";
import { useDebounce } from "@uidotdev/usehooks";
import { Download, Package, Search } from "lucide-react";
import { useState } from "react";
import { useInvoke } from "../../hooks/use-invoke";
import type { SearchResults } from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import { Badge } from "../ui/badge";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { EmptyState } from "../ui/empty-state";
import { Input } from "../ui/input";
import { ScrollArea } from "../ui/scroll-area";
import { Skeleton } from "../ui/skeleton";

export interface PackageSearchDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onSelect: (packageId: string, version: string) => void;
	excludePackageIds?: string[];
}

export function PackageSearchDialog({
	open,
	onOpenChange,
	onSelect,
	excludePackageIds = [],
}: PackageSearchDialogProps) {
	const backend = useBackend();
	const [search, setSearch] = useState("");
	const debouncedSearch = useDebounce(search, 300);

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const results = useQuery<SearchResults>({
		queryKey: ["registry-search-dialog", debouncedSearch],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const params = new URLSearchParams();
			if (debouncedSearch) params.set("query", debouncedSearch);
			params.set("limit", "10");
			return backend.apiState.get<SearchResults>(
				profile.data,
				`registry/search?${params.toString()}`,
			);
		},
		enabled: !!profile.data && open,
	});

	const excludeSet = new Set(excludePackageIds);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<DialogTitle>Add Package</DialogTitle>
					<DialogDescription>
						Search the registry and select a package to add.
					</DialogDescription>
				</DialogHeader>
				<div className="relative">
					<Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
					<Input
						placeholder="Search packages..."
						value={search}
						onChange={(e) => setSearch(e.target.value)}
						className="pl-9"
					/>
				</div>
				<ScrollArea className="max-h-80">
					{results.isLoading ? (
						<SearchResultsSkeleton />
					) : !results.data?.packages?.length ? (
						<EmptyState
							icons={[Package]}
							title="No results"
							description={
								debouncedSearch
									? "Try a different search term."
									: "Type to search for packages."
							}
						/>
					) : (
						<div className="space-y-1">
							{results.data.packages.map((pkg) => {
								const isExcluded = excludeSet.has(pkg.id);
								return (
									<SearchResultItem
										key={pkg.id}
										name={pkg.name}
										description={pkg.description}
										latestVersion={pkg.latestVersion}
										downloadCount={pkg.downloadCount}
										disabled={isExcluded}
										onSelect={() => onSelect(pkg.id, pkg.latestVersion)}
									/>
								);
							})}
						</div>
					)}
				</ScrollArea>
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
	onSelect,
}: {
	name: string;
	description: string;
	latestVersion: string;
	downloadCount: number;
	disabled: boolean;
	onSelect: () => void;
}) {
	return (
		<button
			type="button"
			disabled={disabled}
			onClick={onSelect}
			className="flex w-full items-start gap-3 rounded-lg p-3 text-left transition-colors hover:bg-accent disabled:opacity-50 disabled:cursor-not-allowed"
		>
			<Package className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
			<div className="min-w-0 flex-1">
				<div className="flex items-center gap-2">
					<span className="truncate text-sm font-medium">{name}</span>
					<Badge variant="outline" className="shrink-0 text-xs">
						v{latestVersion}
					</Badge>
					{disabled && (
						<Badge variant="secondary" className="shrink-0 text-xs">
							Already added
						</Badge>
					)}
				</div>
				<p className="mt-0.5 truncate text-xs text-muted-foreground">
					{description}
				</p>
				<div className="mt-1 flex items-center gap-1 text-xs text-muted-foreground/60">
					<Download className="h-3 w-3" />
					{downloadCount.toLocaleString()} downloads
				</div>
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

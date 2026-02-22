"use client";

import { useQuery } from "@tanstack/react-query";
import { useDebounce } from "@uidotdev/usehooks";
import {
	Download,
	KeyRound,
	Package,
	Search,
	Shield,
	SlidersHorizontal,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import type { AuthContextProps } from "react-oidc-context";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import { hashToGradient, useThemeInfo } from "../../hooks/use-theme-gradient";
import type {
	PackageSummary,
	SearchFilters,
	SearchResults,
} from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import {
	type GenericFetcher,
	StorePackageDetail,
} from "../pages/store/store-package-detail";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import { Input } from "../ui/input";
import type { CompileStatus } from "../ui/package-status-badge";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { Skeleton } from "../ui/skeleton";

type SortOption =
	| "relevance"
	| "name"
	| "downloads"
	| "updated_at"
	| "created_at";

interface PackagesStorePageProps {
	fetcher: GenericFetcher;
	auth: AuthContextProps;
	getPackageStatus?: (packageId: string) => CompileStatus | undefined;
}

function PackageCard({ pkg }: { pkg: PackageSummary }) {
	const { primaryHue, isDark } = useThemeInfo();
	const gradient = useMemo(
		() => hashToGradient(pkg.id, primaryHue, isDark),
		[pkg.id, primaryHue, isDark],
	);
	const displayName = pkg.metadata?.name ?? pkg.name;
	const displayDesc = pkg.metadata?.description ?? pkg.description;
	const icon = pkg.metadata?.icon;

	return (
		<Link
			href={`/store/packages?id=${pkg.id}`}
			className="group relative flex items-stretch rounded-xl border border-border/30 bg-card/70 backdrop-blur-sm overflow-hidden transition-all hover:border-primary/30 hover:bg-card/90 hover:shadow-md"
		>
			{/* Left accent strip with icon */}
			<div className="relative w-20 shrink-0 flex items-center justify-center overflow-hidden">
				<div
					className="absolute inset-0"
					style={{
						background: `linear-gradient(${gradient.angle}deg, ${gradient.from}, ${gradient.to})`,
						opacity: gradient.opacity,
					}}
				/>
				<div className="absolute inset-0 bg-linear-to-r from-transparent to-card/80" />
				<Avatar className="relative z-10 w-10 h-10 rounded-lg shadow-sm border border-white/20">
					{icon ? (
						<AvatarImage src={icon} alt={displayName} className="rounded-lg" />
					) : null}
					<AvatarFallback className="rounded-lg text-xs font-bold bg-background/60 backdrop-blur-sm">
						<Package className="h-4 w-4" />
					</AvatarFallback>
				</Avatar>
			</div>

			{/* Content */}
			<div className="flex-1 min-w-0 px-4 py-3.5 flex flex-col justify-between gap-1.5">
				<div className="flex items-start justify-between gap-2">
					<div className="min-w-0">
						<div className="flex items-center gap-2">
							<h3 className="text-sm font-semibold truncate group-hover:text-primary transition-colors">
								{displayName}
							</h3>
							{pkg.verified && (
								<Shield className="h-3 w-3 text-blue-500 shrink-0" />
							)}
						</div>
						<p className="text-xs text-muted-foreground line-clamp-2 mt-0.5">
							{displayDesc}
						</p>
					</div>
					<div className="shrink-0">
						{pkg.price > 0 ? (
							<span className="inline-flex items-center rounded-full bg-primary/10 text-primary px-2.5 py-0.5 text-xs font-semibold">
								€{(pkg.price / 100).toFixed(2)}
							</span>
						) : pkg.visibility === "public_request_access" ? (
							<span className="inline-flex items-center gap-1 rounded-full bg-muted/40 px-2.5 py-0.5 text-xs text-muted-foreground">
								<KeyRound className="h-3 w-3" /> Request
							</span>
						) : null}
					</div>
				</div>

				<div className="flex items-center gap-3 text-[11px] text-muted-foreground/60">
					<span className="font-mono bg-muted/30 rounded px-1.5 py-0.5">
						v{pkg.latestVersion}
					</span>
					<span className="flex items-center gap-1">
						<Download className="h-3 w-3" />
						{pkg.downloadCount.toLocaleString()}
					</span>
					{pkg.keywords.length > 0 && (
						<span className="hidden sm:inline-flex items-center gap-1 border-l border-border/30 pl-3">
							{pkg.keywords.slice(0, 2).map((kw) => (
								<span
									key={kw}
									className="rounded-full bg-muted/30 px-2 py-0.5 text-[10px] capitalize"
								>
									{kw}
								</span>
							))}
							{pkg.keywords.length > 2 && (
								<span className="text-[10px]">+{pkg.keywords.length - 2}</span>
							)}
						</span>
					)}
				</div>
			</div>
		</Link>
	);
}

function PackageCardSkeleton() {
	return (
		<div className="flex items-stretch rounded-xl border border-border/30 bg-card/70 overflow-hidden">
			<div className="w-20 shrink-0 bg-muted/20" />
			<div className="flex-1 px-4 py-3.5 space-y-2">
				<div className="flex items-start justify-between">
					<div className="space-y-1.5 flex-1">
						<Skeleton className="h-4 w-32 rounded" />
						<Skeleton className="h-3 w-full rounded" />
					</div>
					<Skeleton className="h-5 w-14 rounded-full ml-2" />
				</div>
				<div className="flex gap-2">
					<Skeleton className="h-4 w-14 rounded" />
					<Skeleton className="h-4 w-10 rounded" />
					<Skeleton className="h-4 w-12 rounded-full" />
				</div>
			</div>
		</div>
	);
}

function PackageDetailWrapper({
	fetcher,
	auth,
	getPackageStatus,
}: {
	fetcher: GenericFetcher;
	auth: AuthContextProps;
	getPackageStatus?: (packageId: string) => CompileStatus | undefined;
}) {
	const searchParams = useSearchParams();
	const router = useRouter();
	const packageId = searchParams.get("id") ?? "";
	const purchaseStatus = searchParams.get("purchase");

	useEffect(() => {
		if (!purchaseStatus) return;
		if (purchaseStatus === "success") {
			toast.success("Purchase successful! You now have access to this package.", { duration: 5000 });
		} else if (purchaseStatus === "canceled") {
			toast.info("Purchase was canceled. You can try again anytime.");
		}
		const url = new URL(window.location.href);
		url.searchParams.delete("purchase");
		router.replace(url.pathname + url.search, { scroll: false });
	}, [purchaseStatus, router]);

	const handleBack = useCallback(() => router.back(), [router]);
	const compileStatus = getPackageStatus?.(packageId);

	return (
		<StorePackageDetail
			packageId={packageId}
			onBack={handleBack}
			onInstallSuccess={() => toast.success("Package installed successfully")}
			onUninstallSuccess={() =>
				toast.success("Package uninstalled successfully")
			}
			onInstallError={(error) =>
				toast.error(`Failed to install package: ${error.message}`)
			}
			onUninstallError={(error) =>
				toast.error(`Failed to uninstall package: ${error.message}`)
			}
			fetcher={fetcher}
			auth={auth}
			compileStatus={compileStatus}
		/>
	);
}

function PackageListContent({
	fetcher,
	auth,
}: { fetcher: GenericFetcher; auth: AuthContextProps }) {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const [searchQuery, setSearchQuery] = useState("");
	const [sortBy, setSortBy] = useState<SortOption>("downloads");
	const [verifiedOnly, setVerifiedOnly] = useState(false);
	const [offset, setOffset] = useState(0);
	const limit = 12;

	const debouncedQuery = useDebounce(searchQuery, 300);

	const buildFilters = useCallback((): SearchFilters => {
		return {
			query: debouncedQuery || undefined,
			sortBy,
			sortDesc: true,
			verifiedOnly,
			offset,
			limit,
			language: navigator.language?.split("-")[0] ?? "en",
		};
	}, [debouncedQuery, sortBy, verifiedOnly, offset, limit]);

	const searchResults = useQuery({
		queryKey: ["registry-search", debouncedQuery, sortBy, verifiedOnly, offset],
		queryFn: async () => {
			if (!profile.data) return null;
			const params = new URLSearchParams();
			if (debouncedQuery) params.set("query", debouncedQuery);
			params.set("sort_by", sortBy);
			params.set("sort_desc", "true");
			params.set("verified_only", String(verifiedOnly));
			params.set("offset", String(offset));
			params.set("limit", String(limit));
			params.set("language", navigator.language?.split("-")[0] ?? "en");

			return fetcher<SearchResults>(
				profile.data.hub_profile,
				`registry/search?${params.toString()}`,
				{ method: "GET" },
				auth,
			);
		},
		enabled: !!profile.data,
	});

	const totalPages = Math.ceil((searchResults.data?.totalCount ?? 0) / limit);
	const currentPage = Math.floor(offset / limit) + 1;

	return (
		<main className="flex-col flex grow max-h-full p-6 overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-7xl space-y-8">
				<div className="space-y-2">
					<h1 className="text-2xl font-semibold tracking-tight">Packages</h1>
					<p className="text-sm text-muted-foreground">
						Discover and install WASM node packages
					</p>
				</div>

				<div className="flex flex-col sm:flex-row gap-4">
					<div className="relative flex-1">
						<Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
						<Input
							placeholder="Search packages..."
							value={searchQuery}
							onChange={(e) => {
								setSearchQuery(e.target.value);
								setOffset(0);
							}}
							className="rounded-full bg-muted/30 border-border/20 pl-10"
						/>
					</div>

					<div className="flex gap-2">
						<Select
							value={sortBy}
							onValueChange={(val) => {
								setSortBy(val as SortOption);
								setOffset(0);
							}}
						>
							<SelectTrigger className="w-[150px]">
								<SlidersHorizontal className="mr-2 h-4 w-4" />
								<SelectValue placeholder="Sort by" />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="downloads">Most Downloads</SelectItem>
								<SelectItem value="relevance">Relevance</SelectItem>
								<SelectItem value="name">Name</SelectItem>
								<SelectItem value="updated_at">Recently Updated</SelectItem>
								<SelectItem value="created_at">Newest</SelectItem>
							</SelectContent>
						</Select>

						<button
							type="button"
							onClick={() => {
								setVerifiedOnly(!verifiedOnly);
								setOffset(0);
							}}
							className={`rounded-full text-sm border gap-2 px-4 py-2 flex items-center transition-colors ${
								verifiedOnly
									? "bg-primary text-primary-foreground border-primary"
									: "bg-transparent text-muted-foreground border-border/30 hover:bg-muted/30"
							}`}
						>
							<Shield className="h-4 w-4" />
							Verified
						</button>
					</div>
				</div>

				{searchResults.data && (
					<p className="text-xs text-muted-foreground/60">
						{searchResults.data.totalCount.toLocaleString()} packages found
					</p>
				)}

				{searchResults.isLoading ? (
					<div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
						{Array.from({ length: 8 }).map((_, i) => (
							<PackageCardSkeleton key={i} />
						))}
					</div>
				) : searchResults.data?.packages.length === 0 ? (
					<div className="flex flex-col items-center justify-center py-20 text-center">
						<Package className="w-12 h-12 text-muted-foreground/30 mb-3" />
						<h3 className="text-lg font-semibold">No packages found</h3>
						<p className="text-sm text-muted-foreground mt-1">
							Try adjusting your search or filters
						</p>
					</div>
				) : (
					<div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
						{searchResults.data?.packages.map((pkg) => (
							<PackageCard key={pkg.id} pkg={pkg} />
						))}
					</div>
				)}

				{totalPages > 1 && (
					<div className="flex items-center justify-center gap-3">
						<button
							type="button"
							onClick={() => setOffset(Math.max(0, offset - limit))}
							disabled={offset === 0}
							className="rounded-full text-sm text-muted-foreground/60 border border-border/30 hover:bg-muted/30 px-5 py-2 transition-colors disabled:opacity-40"
						>
							Previous
						</button>
						<span className="text-xs text-muted-foreground/60">
							{currentPage} / {totalPages}
						</span>
						<button
							type="button"
							onClick={() => setOffset(offset + limit)}
							disabled={currentPage >= totalPages}
							className="rounded-full text-sm text-muted-foreground/60 border border-border/30 hover:bg-muted/30 px-5 py-2 transition-colors disabled:opacity-40"
						>
							Next
						</button>
					</div>
				)}
			</div>
		</main>
	);
}

function PageContent({
	fetcher,
	auth,
	getPackageStatus,
}: PackagesStorePageProps) {
	const searchParams = useSearchParams();
	const packageId = searchParams.get("id");

	if (packageId) {
		return (
			<Suspense fallback={<Skeleton className="h-full w-full" />}>
				<PackageDetailWrapper
					fetcher={fetcher}
					auth={auth}
					getPackageStatus={getPackageStatus}
				/>
			</Suspense>
		);
	}

	return <PackageListContent fetcher={fetcher} auth={auth} />;
}

export function PackagesStorePage({
	fetcher,
	auth,
	getPackageStatus,
}: PackagesStorePageProps) {
	return (
		<Suspense fallback={<Skeleton className="h-full w-full" />}>
			<PageContent
				fetcher={fetcher}
				auth={auth}
				getPackageStatus={getPackageStatus}
			/>
		</Suspense>
	);
}

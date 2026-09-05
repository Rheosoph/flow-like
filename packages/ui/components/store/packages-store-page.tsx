"use client";

import { useTranslation } from "@flow-like/locales";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useDebounce } from "@uidotdev/usehooks";
import {
	AlertCircle,
	Loader2,
	Package,
	RotateCw,
	Search,
	Shield,
	X,
} from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import {
	type ReactNode,
	Suspense,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import { getErrorMessage } from "../../lib/error-message";
import type { SearchResults } from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import {
	type GenericFetcher,
	StorePackageDetail,
} from "../pages/store/store-package-detail";
import { Alert, AlertDescription } from "../ui/alert";
import { Button } from "../ui/button";
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
import { ExploreHubLayout } from "./explore-hub-layout";
import { PackageCard } from "./package-card";
import { getPackageOverviewHref } from "./package-navigation";

export { PackageCard } from "./package-card";

type SortOption =
	| "relevance"
	| "name"
	| "downloads"
	| "updated_at"
	| "created_at";

type PackagesStoreAuth =
	| {
			readonly user?: { readonly access_token?: string } | null;
	  }
	| null
	| undefined;

interface PackagesStorePageProps {
	fetcher: GenericFetcher;
	auth: PackagesStoreAuth;
	getPackageStatus?: (packageId: string) => CompileStatus | undefined;
}

const PACKAGE_CARD_SKELETON_KEYS = Array.from(
	{ length: 12 },
	(_, index) => `package-skeleton-${index}`,
);

function PackageCardSkeleton() {
	return (
		<div className="flex min-h-96 flex-col rounded-xl border border-border/60 bg-card p-2.5">
			<Skeleton className="aspect-video w-full rounded-lg" />
			<div className="mt-2.5 flex items-center gap-2">
				<Skeleton className="h-9 w-9 shrink-0 rounded-lg" />
				<div className="min-w-0 flex-1 space-y-1.5">
					<Skeleton className="h-3.5 w-28 rounded" />
					<Skeleton className="h-2.5 w-16 rounded" />
				</div>
			</div>
			<div className="mt-2 min-h-9 space-y-1.5">
				<Skeleton className="h-3 w-full rounded" />
				<Skeleton className="h-3 w-3/4 rounded" />
			</div>
			<div className="mb-2.5 mt-2.5 flex gap-1">
				<Skeleton className="h-5 w-16 rounded" />
				<Skeleton className="h-5 w-20 rounded" />
			</div>
			<div className="mt-auto grid grid-cols-3 gap-2.5 border-t border-border/60 pt-2.5">
				<Skeleton className="h-7 rounded" />
				<Skeleton className="h-7 rounded" />
				<Skeleton className="h-7 rounded" />
			</div>
		</div>
	);
}

export function PackageDetailWrapper({
	fetcher,
	auth,
	getPackageStatus,
}: {
	fetcher: GenericFetcher;
	auth: PackagesStoreAuth;
	getPackageStatus?: (packageId: string) => CompileStatus | undefined;
}) {
	const { t } = useTranslation("store");
	const searchParams = useSearchParams();
	const router = useRouter();
	const packageId = searchParams.get("id") ?? "";
	const purchaseStatus = searchParams.get("purchase");

	useEffect(() => {
		if (!purchaseStatus) return;
		if (purchaseStatus === "success") {
			toast.success(
				t(
					"purchaseSuccessfulYouNowHaveAccessToThisPackage",
					"Purchase successful! You now have access to this package.",
				),
				{ duration: 5000 },
			);
		} else if (purchaseStatus === "canceled") {
			toast.info(
				t(
					"purchaseCanceledTryAgainAnytime",
					"Purchase was canceled. You can try again anytime.",
				),
			);
		}
		const url = new URL(window.location.href);
		url.searchParams.delete("purchase");
		router.replace(url.pathname + url.search, { scroll: false });
	}, [purchaseStatus, router, t]);

	const handleBack = useCallback(() => {
		router.replace(getPackageOverviewHref(searchParams), {
			scroll: false,
		});
	}, [router, searchParams]);
	const compileStatus = getPackageStatus?.(packageId);

	return (
		<StorePackageDetail
			packageId={packageId}
			onBack={handleBack}
			onInstallSuccess={() =>
				toast.success(
					t("packageInstalledSuccessfully", "Package installed successfully"),
				)
			}
			onUninstallSuccess={() =>
				toast.success(
					t(
						"packageUninstalledSuccessfully",
						"Package uninstalled successfully",
					),
				)
			}
			onInstallError={(error) =>
				toast.error(
					t(
						"failedToInstallPackageMessage",
						"Failed to install package: {{message}}",
						{ message: getErrorMessage(error) },
					),
				)
			}
			onUninstallError={(error) =>
				toast.error(
					t(
						"failedToUninstallPackageMessage",
						"Failed to uninstall package: {{message}}",
						{ message: getErrorMessage(error) },
					),
				)
			}
			onDeleteSuccess={handleBack}
			fetcher={fetcher}
			auth={auth}
			compileStatus={compileStatus}
		/>
	);
}

const PACKAGE_GRID_CLASS_NAME =
	"grid grid-cols-[repeat(auto-fill,minmax(min(100%,280px),1fr))] gap-4";

export function PackageListContent({
	fetcher,
	auth,
	navigation,
}: {
	fetcher: GenericFetcher;
	auth: PackagesStoreAuth;
	navigation?: ReactNode;
}) {
	const { t } = useTranslation("store");
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const searchRef = useRef<HTMLInputElement>(null);
	const [searchQuery, setSearchQuery] = useState("");
	const [sortBy, setSortBy] = useState<SortOption>("downloads");
	const [verifiedOnly, setVerifiedOnly] = useState(false);
	const [offset, setOffset] = useState(0);
	const limit = 12;
	const debouncedQuery = useDebounce(searchQuery.trim(), 300);
	const isAuthenticated = !!auth?.user?.access_token;

	const searchResults = useQuery({
		queryKey: [
			"registry-search",
			debouncedQuery,
			sortBy,
			verifiedOnly,
			offset,
			isAuthenticated,
			profile.data?.hub_profile,
		],
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
			params.set("include_own", "true");
			return fetcher<SearchResults>(
				profile.data.hub_profile,
				`registry/search?${params.toString()}`,
				{ method: "GET" },
				auth,
			);
		},
		enabled: !!profile.data,
		placeholderData: keepPreviousData,
	});

	const error = profile.error ?? searchResults.error;
	const isInitialLoading =
		!error && (profile.isPending || searchResults.isPending);
	const isBusy =
		isInitialLoading ||
		profile.isFetching ||
		searchResults.isFetching ||
		searchQuery.trim() !== debouncedQuery;
	const hasFilters = !!searchQuery || sortBy !== "downloads" || verifiedOnly;
	const totalPages = Math.ceil((searchResults.data?.totalCount ?? 0) / limit);
	const currentPage = Math.floor(offset / limit) + 1;
	const clearFilters = () => {
		setSearchQuery("");
		setSortBy("downloads");
		setVerifiedOnly(false);
		setOffset(0);
		searchRef.current?.focus();
	};

	return (
		<ExploreHubLayout
			active="packages"
			subtitle={t(
				"discoverAndInstallWasmNodePackages",
				"Discover and install WASM node packages.",
			)}
			toolbar={
				<div className="flex flex-col gap-3 sm:flex-row sm:items-center">
					<div className="relative min-w-0 flex-1">
						{isBusy ? (
							<Loader2
								aria-hidden="true"
								className="pointer-events-none absolute left-4 top-1/2 h-5 w-5 -translate-y-1/2 animate-spin text-primary"
							/>
						) : (
							<Search
								aria-hidden="true"
								className="pointer-events-none absolute left-4 top-1/2 h-5 w-5 -translate-y-1/2 text-muted-foreground"
							/>
						)}
						<Input
							ref={searchRef}
							type="search"
							aria-label={t("searchPackages", "Search packages...")}
							aria-controls="explore-package-results"
							placeholder={t("searchPackages", "Search packages...")}
							value={searchQuery}
							onChange={(e) => {
								setSearchQuery(e.target.value);
								setOffset(0);
							}}
							className="h-12 rounded-xl border-border/60 bg-muted/30 pr-12 pl-12 text-sm shadow-none transition-colors focus-visible:bg-background [&::-webkit-search-cancel-button]:appearance-none"
						/>
						{searchQuery && (
							<button
								type="button"
								aria-label={t("clearSearch", "Clear search")}
								onClick={() => {
									setSearchQuery("");
									setOffset(0);
									searchRef.current?.focus();
								}}
								className="absolute right-1 top-1/2 flex h-10 w-10 -translate-y-1/2 items-center justify-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
							>
								<X aria-hidden="true" className="h-4 w-4" />
							</button>
						)}
					</div>
					<div className="flex shrink-0 items-center justify-between gap-2 sm:justify-start">
						<Select
							value={sortBy}
							onValueChange={(value) => {
								setSortBy(value as SortOption);
								setOffset(0);
							}}
						>
							<SelectTrigger
								aria-label={t("sortResults", "Sort results")}
								className="min-h-12 min-w-0 max-w-48 flex-1 gap-2 rounded-xl sm:w-48 sm:flex-none border-border/60 bg-background text-sm"
							>
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="downloads">
									{t("mostDownloads", "Most Downloads")}
								</SelectItem>
								<SelectItem value="relevance">
									{t("relevance", "Relevance")}
								</SelectItem>
								<SelectItem value="name">{t("name", "Name")}</SelectItem>
								<SelectItem value="updated_at">
									{t("recentlyUpdated", "Recently updated")}
								</SelectItem>
								<SelectItem value="created_at">
									{t("newest", "Newest")}
								</SelectItem>
							</SelectContent>
						</Select>
						<div className="w-28 shrink-0">
							<Button
								variant="ghost"
								size="sm"
								disabled={!hasFilters}
								onClick={clearFilters}
								className={`h-12 w-full rounded-xl text-muted-foreground ${hasFilters ? "" : "invisible"}`}
							>
								<X aria-hidden="true" className="mr-1 h-3.5 w-3.5" />
								{t("clearFilters", "Clear filters")}
							</Button>
						</div>
					</div>
				</div>
			}
			filters={
				<div className="flex min-h-10 flex-wrap items-center gap-3">
					{navigation}
					<button
						type="button"
						aria-pressed={verifiedOnly}
						onClick={() => {
							setVerifiedOnly(!verifiedOnly);
							setOffset(0);
						}}
						className={`inline-flex min-h-10 shrink-0 items-center gap-2 rounded-xl border px-3 py-2 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${verifiedOnly ? "border-primary/25 bg-primary/10 text-primary" : "border-border/60 bg-background text-muted-foreground hover:bg-muted hover:text-foreground"}`}
					>
						<Shield aria-hidden="true" className="h-4 w-4" />
						{t("verified", "Verified")}
					</button>
				</div>
			}
		>
			<section
				id="explore-package-results"
				aria-busy={isBusy}
				className="space-y-5"
			>
				<output
					className="block min-h-5 text-sm text-muted-foreground"
					aria-live="polite"
				>
					{isBusy
						? t("loadingPackages", "Loading packages…")
						: error
							? t("packagesCouldNotBeLoaded", "Packages could not be loaded.")
							: `${(searchResults.data?.totalCount ?? 0).toLocaleString()} ${t("packagesFound", "packages found")}`}
				</output>
				{error && (
					<Alert variant="destructive" className="rounded-xl">
						<AlertCircle className="h-4 w-4" />
						<AlertDescription className="flex flex-wrap items-center justify-between gap-3">
							{t(
								"packagesLoadError",
								"Packages could not be loaded. Please try again.",
							)}
							<Button
								variant="outline"
								size="sm"
								disabled={isBusy}
								onClick={() =>
									profile.error ? profile.refetch() : searchResults.refetch()
								}
							>
								<RotateCw aria-hidden="true" className="mr-1.5 h-3.5 w-3.5" />
								{t("retry", "Retry")}
							</Button>
						</AlertDescription>
					</Alert>
				)}
				{isInitialLoading ? (
					<div className={PACKAGE_GRID_CLASS_NAME}>
						{PACKAGE_CARD_SKELETON_KEYS.map((key) => (
							<PackageCardSkeleton key={key} />
						))}
					</div>
				) : searchResults.data?.packages.length === 0 ? (
					<div className="flex flex-col items-center justify-center rounded-2xl border border-dashed border-border bg-muted/10 px-6 py-16 text-center">
						<Package
							aria-hidden="true"
							className="mb-3 h-10 w-10 text-muted-foreground"
						/>
						<h2 className="text-lg font-semibold">
							{t("noPackagesFound", "No packages found")}
						</h2>
						<p className="mt-2 text-sm text-muted-foreground">
							{t(
								"tryAdjustingYourSearchOrFilters",
								"Try adjusting your search or filters",
							)}
						</p>
						{hasFilters && (
							<Button
								variant="outline"
								onClick={clearFilters}
								className="mt-5 rounded-xl"
							>
								{t("clearFilters", "Clear filters")}
							</Button>
						)}
					</div>
				) : (
					<div className={PACKAGE_GRID_CLASS_NAME}>
						{searchResults.data?.packages.map((pkg) => (
							<PackageCard
								key={pkg.id}
								pkg={pkg}
								className="min-h-96 [&>p]:min-h-9"
							/>
						))}
					</div>
				)}
				{totalPages > 1 && (
					<div className="flex min-h-11 items-center justify-center gap-3">
						<Button
							variant="outline"
							onClick={() => setOffset(Math.max(0, offset - limit))}
							disabled={offset === 0 || isBusy}
							className="min-h-11 rounded-xl"
						>
							{t("previous", "Previous")}
						</Button>
						<span className="min-w-20 text-center text-sm text-muted-foreground tabular-nums">
							{isBusy
								? t("loading", "Loading…")
								: `${currentPage} / ${totalPages}`}
						</span>
						<Button
							variant="outline"
							onClick={() => setOffset(offset + limit)}
							disabled={
								currentPage >= totalPages ||
								isBusy ||
								searchResults.isPlaceholderData
							}
							className="min-h-11 rounded-xl"
						>
							{t("next", "Next")}
						</Button>
					</div>
				)}
			</section>
		</ExploreHubLayout>
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

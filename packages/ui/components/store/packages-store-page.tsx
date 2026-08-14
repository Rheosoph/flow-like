"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { useDebounce } from "@uidotdev/usehooks";
import {
	ChevronRight,
	KeyRound,
	Lock,
	Package,
	Search,
	Shield,
	SlidersHorizontal,
	Star,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import {
	Suspense,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import { hashToGradient, useThemeInfo } from "../../hooks/use-theme-gradient";
import { getErrorMessage } from "../../lib/error-message";
import type { PackageSummary, SearchResults } from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import {
	type GenericFetcher,
	StorePackageDetail,
} from "../pages/store/store-package-detail";
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
import { ExploreHubHeader } from "./explore-hub-header";
import { getPackageOverviewHref } from "./package-navigation";

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
	{ length: 9 },
	(_, index) => `package-skeleton-${index}`,
);

function getPackageInitials(name: string): string {
	const words = name
		.replace(/[()]/g, " ")
		.split(/\s+/)
		.filter((word) => {
			const normalized = word.toLowerCase();
			return normalized !== "custom" && normalized !== "node";
		});
	if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
	const initials = (words.length ? words : [name])
		.slice(0, 2)
		.map((word) => word[0]?.toUpperCase() ?? "")
		.join("");

	return initials || "PK";
}

function formatCompact(n: number): string {
	if (n >= 1_000_000)
		return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
	if (n >= 1_000) return `${(n / 1_000).toFixed(1).replace(/\.0$/, "")}k`;
	return `${n}`;
}

function prettyCategory(category: string): string {
	return category
		.toLowerCase()
		.split("_")
		.map((word) => word.charAt(0).toUpperCase() + word.slice(1))
		.join(" ");
}

export function PackageCard({ pkg }: { pkg: PackageSummary }) {
	const { t } = useTranslation("store");
	const { primaryHue, isDark } = useThemeInfo();
	const gradient = useMemo(
		() => hashToGradient(pkg.id, primaryHue, isDark),
		[pkg.id, primaryHue, isDark],
	);
	const displayName = pkg.metadata?.name ?? pkg.name;
	const displayDesc = pkg.metadata?.description ?? pkg.description;
	const icon = pkg.metadata?.icon;
	const thumbnail = pkg.metadata?.thumbnail;
	const cover = icon ?? thumbnail;
	const rated = (pkg.ratingCount ?? 0) > 0;
	const category = pkg.primaryCategory ?? pkg.secondaryCategory;

	return (
		<Link
			href={`/store/packages?id=${pkg.id}`}
			className="group relative flex h-56 w-full flex-col overflow-hidden rounded-2xl border border-border/50 shadow-sm transition-all hover:-translate-y-1 hover:border-primary/40 hover:shadow-xl"
		>
			{/* cover image (or the deterministic aurora when there's none) */}
			<div className="absolute inset-0 transition-transform duration-500 ease-out group-hover:scale-105">
				{thumbnail ? (
					<img
						src={thumbnail}
						alt=""
						className="absolute inset-0 h-full w-full object-cover"
					/>
				) : (
					<>
						<div
							className="absolute inset-0"
							style={{
								background: `linear-gradient(${gradient.angle}deg, ${gradient.from}, ${gradient.to})`,
							}}
						/>
						{icon && (
							<img
								src={icon}
								alt=""
								aria-hidden="true"
								className="absolute left-1/2 top-1/3 h-[160%] w-[160%] -translate-x-1/2 -translate-y-1/2 object-contain opacity-40 blur-2xl saturate-150"
							/>
						)}
					</>
				)}
			</div>

			{/* frosted glass for legibility — kept dark in both themes so any cover reads */}
			<div className="absolute inset-0 bg-linear-to-b from-black/25 via-black/55 to-black/85 backdrop-blur-[2px]" />
			<div
				className="pointer-events-none absolute inset-0 rounded-2xl opacity-0 transition-opacity duration-300 group-hover:opacity-100"
				style={{ boxShadow: `inset 0 0 0 1px ${gradient.from}66` }}
			/>

			<div className="relative z-10 flex h-full flex-col p-4">
				<div className="flex items-center gap-3">
					<div className="relative h-11 w-11 shrink-0 overflow-hidden rounded-xl border border-white/20 bg-white/10 shadow-lg backdrop-blur-md">
						{cover ? (
							<img src={cover} alt="" className="h-full w-full object-cover" />
						) : (
							<div className="flex h-full w-full items-center justify-center font-mono text-xs font-semibold text-white">
								{getPackageInitials(displayName)}
							</div>
						)}
					</div>
					<div className="min-w-0 flex-1">
						{category && (
							<div className="truncate text-[10px] font-semibold uppercase tracking-wider text-white/60">
								{prettyCategory(category)}
							</div>
						)}
						<div className="flex items-center gap-1.5">
							<h3 className="truncate font-mono text-sm font-semibold text-white">
								{displayName}
							</h3>
							{pkg.verified && (
								<Shield className="h-3.5 w-3.5 shrink-0 text-sky-400" />
							)}
						</div>
					</div>
					<div className="flex shrink-0 items-center gap-1.5 self-start">
						{pkg.visibility !== "public" && (
							<span
								className="rounded-md border border-white/20 bg-white/12 p-1 text-white/80"
								title={pkg.visibility}
							>
								{pkg.visibility === "private" ? (
									<Lock className="h-3 w-3" />
								) : (
									<KeyRound className="h-3 w-3" />
								)}
							</span>
						)}
						<span className="rounded-md border border-white/20 bg-white/12 px-2 py-0.5 font-mono text-[10px] text-white/85">{`v${pkg.latestVersion}`}</span>
					</div>
				</div>

				<p className="mt-3 line-clamp-2 text-xs leading-relaxed text-white/70">
					{displayDesc}
				</p>

				<div className="mt-auto grid grid-cols-3 gap-2">
					<div className="rounded-xl border border-white/12 bg-white/8 px-2 py-2 text-center backdrop-blur-sm">
						<div className="font-mono text-sm font-bold tabular-nums text-white">
							{formatCompact(pkg.downloadCount)}
						</div>
						<div className="mt-0.5 text-[9px] uppercase tracking-wider text-white/55">
							{t("installs", "Installs")}
						</div>
					</div>
					<div className="rounded-xl border border-white/12 bg-white/8 px-2 py-2 text-center backdrop-blur-sm">
						<div className="flex items-center justify-center gap-1 font-mono text-sm font-bold tabular-nums text-white">
							{rated ? (
								<>
									<Star className="h-3 w-3 fill-yellow-400 text-yellow-400" />
									{(pkg.avgRating ?? 0).toFixed(1)}
								</>
							) : (
								"New"
							)}
						</div>
						<div className="mt-0.5 text-[9px] uppercase tracking-wider text-white/55">
							{t("rating", "Rating")}
						</div>
					</div>
					<div className="rounded-xl border border-white/12 bg-white/8 px-2 py-2 text-center backdrop-blur-sm">
						<div className="font-mono text-sm font-bold text-white">
							{pkg.price > 0 ? `€${(pkg.price / 100).toFixed(2)}` : "Free"}
						</div>
						<div className="mt-0.5 text-[9px] uppercase tracking-wider text-white/55">
							{t("price", "Price")}
						</div>
					</div>
				</div>
			</div>
		</Link>
	);
}

function PackageCardSkeleton() {
	return (
		<div className="flex h-56 flex-col rounded-2xl border border-border/40 bg-card/60 p-4">
			<div className="flex items-start gap-3">
				<Skeleton className="h-11 w-11 shrink-0 rounded-md" />
				<div className="min-w-0 flex-1 space-y-2">
					<div className="flex items-center gap-2">
						<Skeleton className="h-4 w-28 rounded" />
						<Skeleton className="h-3 w-10 rounded" />
					</div>
					<Skeleton className="h-3 w-full rounded" />
					<Skeleton className="h-3 w-3/4 rounded" />
				</div>
			</div>
			<div className="mt-auto flex gap-2 border-t border-border/20 pt-3">
				<Skeleton className="h-3.5 w-10 rounded" />
				<Skeleton className="h-3.5 w-16 rounded" />
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

function categoryLabel(cat: string): string {
	return cat
		.split("_")
		.map((w) => w.charAt(0) + w.slice(1).toLowerCase())
		.join(" ");
}

function groupByCategory(
	packages: PackageSummary[],
): Map<string, PackageSummary[]> {
	const groups = new Map<string, PackageSummary[]>();
	const seen = new Set<string>();

	for (const pkg of packages) {
		const category = pkg.primaryCategory ?? pkg.secondaryCategory ?? "OTHER";
		const label = categoryLabel(category);
		let group = groups.get(label);
		if (!group) {
			group = [];
			groups.set(label, group);
		}
		if (!seen.has(pkg.id)) {
			group.push(pkg);
			seen.add(pkg.id);
		}
	}
	return groups;
}

function Swimlane({
	title,
	packages,
}: { title: string; packages: PackageSummary[] }) {
	const scrollRef = useRef<HTMLDivElement>(null);
	const [canScrollLeft, setCanScrollLeft] = useState(false);
	const [canScrollRight, setCanScrollRight] = useState(false);

	const checkScroll = useCallback(() => {
		const el = scrollRef.current;
		if (!el) return;
		setCanScrollLeft(el.scrollLeft > 0);
		setCanScrollRight(el.scrollLeft + el.clientWidth < el.scrollWidth - 1);
	}, []);

	useEffect(() => {
		const el = scrollRef.current;
		if (!el) return;
		checkScroll();
		el.addEventListener("scroll", checkScroll, { passive: true });
		const ro = new ResizeObserver(checkScroll);
		ro.observe(el);
		return () => {
			el.removeEventListener("scroll", checkScroll);
			ro.disconnect();
		};
	}, [checkScroll]);

	const scroll = (direction: "left" | "right") => {
		const el = scrollRef.current;
		if (!el) return;
		const amount = el.clientWidth * 0.8;
		el.scrollBy({
			left: direction === "left" ? -amount : amount,
			behavior: "smooth",
		});
	};

	return (
		<div className="space-y-2">
			<div className="flex items-center gap-2">
				<h3 className="text-sm font-semibold capitalize">{title}</h3>
				<span className="text-[10px] text-muted-foreground/50 font-mono">
					{packages.length}
				</span>
				<ChevronRight className="h-3.5 w-3.5 text-muted-foreground/30" />
			</div>
			<div className="relative group/swimlane">
				{canScrollLeft && (
					<button
						type="button"
						onClick={() => scroll("left")}
						className="absolute left-0 top-0 bottom-0 z-10 w-8 flex items-center justify-center bg-linear-to-r from-background/80 to-transparent opacity-100 md:opacity-0 md:group-hover/swimlane:opacity-100 transition-opacity"
					>
						<ChevronRight className="h-4 w-4 rotate-180" />
					</button>
				)}
				<div
					ref={scrollRef}
					className="flex snap-x snap-mandatory gap-3 overflow-x-auto scrollbar-none pb-1"
				>
					{packages.map((pkg) => (
						<div key={pkg.id} className="w-[85vw] max-w-72 shrink-0 snap-start">
							<PackageCard pkg={pkg} />
						</div>
					))}
				</div>
				{canScrollRight && (
					<button
						type="button"
						onClick={() => scroll("right")}
						className="absolute right-0 top-0 bottom-0 z-10 w-8 flex items-center justify-center bg-linear-to-l from-background/80 to-transparent opacity-100 md:opacity-0 md:group-hover/swimlane:opacity-100 transition-opacity"
					>
						<ChevronRight className="h-4 w-4" />
					</button>
				)}
			</div>
		</div>
	);
}

export function PackageListContent({
	fetcher,
	auth,
}: { fetcher: GenericFetcher; auth: PackagesStoreAuth }) {
	const { t } = useTranslation("store");
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
	const isAuthenticated = !!auth?.user?.access_token;

	const searchResults = useQuery({
		queryKey: [
			"registry-search",
			debouncedQuery,
			sortBy,
			verifiedOnly,
			offset,
			isAuthenticated,
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
	});

	const totalPages = Math.ceil((searchResults.data?.totalCount ?? 0) / limit);
	const currentPage = Math.floor(offset / limit) + 1;
	const isSearching = !!debouncedQuery;

	const swimlaneGroups = useMemo(() => {
		if (isSearching || !searchResults.data?.packages.length) return null;
		return groupByCategory(searchResults.data.packages);
	}, [isSearching, searchResults.data?.packages]);

	return (
		<div className="space-y-6 w-full">
			<div className="flex flex-col sm:flex-row gap-4">
				<div className="relative flex-1">
					<Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
					<Input
						placeholder={t("searchPackages", "Search packages...")}
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
						<SelectTrigger className="w-37.5">
							<SlidersHorizontal className="mr-2 h-4 w-4" />
							<SelectValue placeholder={t("sortBy", "Sort by")} />
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
								{t("recentlyUpdated", "Recently Updated")}
							</SelectItem>
							<SelectItem value="created_at">
								{t("newest", "Newest")}
							</SelectItem>
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
						{t("verified", "Verified")}
					</button>
				</div>
			</div>

			{searchResults.data && (
				<p className="text-xs text-muted-foreground/60">
					{searchResults.data.totalCount.toLocaleString()}{" "}
					{t("packagesFound", "packages found")}
				</p>
			)}

			{searchResults.isLoading ? (
				<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
					{PACKAGE_CARD_SKELETON_KEYS.map((key) => (
						<PackageCardSkeleton key={key} />
					))}
				</div>
			) : searchResults.data?.packages.length === 0 ? (
				<div className="flex flex-col items-center justify-center py-20 text-center">
					<Package className="w-12 h-12 text-muted-foreground/30 mb-3" />
					<h3 className="text-lg font-semibold">
						{t("noPackagesFound", "No packages found")}
					</h3>
					<p className="text-sm text-muted-foreground mt-1">
						{t(
							"tryAdjustingYourSearchOrFilters",
							"Try adjusting your search or filters",
						)}
					</p>
				</div>
			) : swimlaneGroups && swimlaneGroups.size > 1 ? (
				<div className="space-y-6">
					{Array.from(swimlaneGroups.entries()).map(([category, pkgs]) => (
						<Swimlane key={category} title={category} packages={pkgs} />
					))}
				</div>
			) : (
				<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
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
						{t("previous", "Previous")}
					</button>
					<span className="text-xs text-muted-foreground/60">{`${currentPage} / ${totalPages}`}</span>
					<button
						type="button"
						onClick={() => setOffset(offset + limit)}
						disabled={currentPage >= totalPages}
						className="rounded-full text-sm text-muted-foreground/60 border border-border/30 hover:bg-muted/30 px-5 py-2 transition-colors disabled:opacity-40"
					>
						{t("next", "Next")}
					</button>
				</div>
			)}
		</div>
	);
}

function PageContent({
	fetcher,
	auth,
	getPackageStatus,
}: PackagesStorePageProps) {
	const { t } = useTranslation("store");
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

	return (
		<main className="flex-col flex grow max-h-full p-6 overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-7xl space-y-8">
				<ExploreHubHeader
					active="packages"
					subtitle={t(
						"discoverAndInstallWasmNodePackages",
						"Discover and install WASM node packages.",
					)}
				/>
				<PackageListContent fetcher={fetcher} auth={auth} />
			</div>
		</main>
	);
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

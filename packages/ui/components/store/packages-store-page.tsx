"use client";

import { useQuery } from "@tanstack/react-query";
import { useDebounce } from "@uidotdev/usehooks";
import {
	ChevronRight,
	Download,
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

function PackageMark({
	displayName,
	icon,
	thumbnail,
	gradient,
}: {
	displayName: string;
	icon?: string;
	thumbnail?: string;
	gradient: ReturnType<typeof hashToGradient>;
}) {
	const image = icon ?? thumbnail;

	return (
		<div className="relative h-11 w-11 shrink-0 overflow-hidden rounded-md border border-border/40 bg-muted/40 shadow-sm">
			<div
				className="absolute inset-0"
				style={{
					background: `linear-gradient(${gradient.angle}deg, ${gradient.from}, ${gradient.to})`,
					opacity: gradient.opacity,
				}}
			/>
			{image ? (
				<img
					src={image}
					alt=""
					className="relative h-full w-full bg-background/20 object-cover"
				/>
			) : (
				<div className="relative flex h-full w-full items-center justify-center font-mono text-xs font-semibold text-foreground/80">
					{getPackageInitials(displayName)}
				</div>
			)}
		</div>
	);
}

export function PackageCard({ pkg }: { pkg: PackageSummary }) {
	const { primaryHue, isDark } = useThemeInfo();
	const gradient = useMemo(
		() => hashToGradient(pkg.id, primaryHue, isDark),
		[pkg.id, primaryHue, isDark],
	);
	const displayName = pkg.metadata?.name ?? pkg.name;
	const displayDesc = pkg.metadata?.description ?? pkg.description;
	const icon = pkg.metadata?.icon;
	const thumbnail = pkg.metadata?.thumbnail;

	return (
		<Link
			href={`/store/packages?id=${pkg.id}`}
			className="group relative flex min-h-35 flex-col overflow-hidden rounded-lg border border-border/40 bg-card/70 p-4 backdrop-blur-sm transition-all hover:border-primary/40 hover:bg-card/90 hover:shadow-lg"
		>
			<div
				className="absolute inset-0"
				style={{
					background: `linear-gradient(${gradient.angle}deg, ${gradient.from}, ${gradient.to})`,
					opacity: isDark ? 0.1 : 0.08,
				}}
			/>
			{thumbnail && (
				<>
					<img
						src={thumbnail}
						alt=""
						className="absolute inset-0 h-full w-full scale-[1.02] object-cover opacity-[0.14] saturate-125 transition-opacity group-hover:opacity-[0.18] dark:opacity-[0.18] dark:group-hover:opacity-[0.24]"
					/>
					<div className="absolute inset-0 bg-linear-to-r from-card/90 via-card/78 to-card/70" />
				</>
			)}
			<div
				className="absolute inset-y-0 left-0 z-10 w-1"
				style={{
					background: `linear-gradient(180deg, ${gradient.from}, ${gradient.to})`,
					opacity: gradient.opacity,
				}}
			/>

			<div className="relative z-10 flex flex-1 min-w-0 flex-col gap-3">
				<div className="flex min-w-0 items-start gap-3">
					<PackageMark
						displayName={displayName}
						icon={icon}
						thumbnail={thumbnail}
						gradient={gradient}
					/>
					<div className="min-w-0 flex-1">
						<div className="flex items-center gap-1.5 min-w-0">
							<h3 className="text-sm font-semibold font-mono truncate group-hover:text-primary transition-colors">
								{displayName}
							</h3>
							<span className="text-[10px] font-mono text-muted-foreground/50 shrink-0">
								v{pkg.latestVersion}
							</span>
							<div className="flex items-center gap-1 ml-auto shrink-0">
								{pkg.verified && (
									<span className="inline-flex items-center rounded bg-background/80 border border-border/40 p-1">
										<Shield className="h-3 w-3 text-blue-500" />
									</span>
								)}
								{pkg.visibility !== "public" && (
									<span className="inline-flex items-center gap-0.5 rounded bg-background/80 border border-border/40 px-1.5 py-0.5 text-[10px] text-muted-foreground font-mono">
										{pkg.visibility === "private" ? (
											<>
												<Lock className="h-2.5 w-2.5" /> private
											</>
										) : (
											<>
												<KeyRound className="h-2.5 w-2.5" /> gated
											</>
										)}
									</span>
								)}
							</div>
						</div>

						<p className="text-xs text-muted-foreground/80 line-clamp-2 leading-relaxed">
							{displayDesc}
						</p>
					</div>
				</div>

				<div className="flex items-center gap-2 mt-auto pt-3 border-t border-border/20 text-[10px] text-muted-foreground/60 font-mono">
					<span className="flex items-center gap-1">
						<Download className="h-3 w-3" />
						{pkg.downloadCount.toLocaleString()}
					</span>
					{(pkg.ratingCount ?? 0) > 0 && (
						<span className="flex items-center gap-0.5 border-l border-border/30 pl-2">
							<Star className="h-3 w-3 text-yellow-500 fill-yellow-500" />
							{(pkg.avgRating ?? 0).toFixed(1)}
						</span>
					)}
					{pkg.price > 0 && (
						<span className="font-semibold text-primary">
							€{(pkg.price / 100).toFixed(2)}
						</span>
					)}
					{pkg.keywords.length > 0 && (
						<span className="inline-flex min-w-0 items-center gap-1 border-l border-border/30 pl-2">
							{pkg.keywords.slice(0, 2).map((kw) => (
								<span
									key={kw}
									className="max-w-24 truncate rounded bg-muted/30 px-1.5 py-0.5"
								>
									{kw}
								</span>
							))}
							{pkg.keywords.length > 2 && (
								<span>+{pkg.keywords.length - 2}</span>
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
		<div className="flex min-h-35 flex-col rounded-lg border border-border/40 bg-card/60 p-4">
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
	const searchParams = useSearchParams();
	const router = useRouter();
	const packageId = searchParams.get("id") ?? "";
	const purchaseStatus = searchParams.get("purchase");

	useEffect(() => {
		if (!purchaseStatus) return;
		if (purchaseStatus === "success") {
			toast.success(
				"Purchase successful! You now have access to this package.",
				{ duration: 5000 },
			);
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
				toast.error(`Failed to install package: ${getErrorMessage(error)}`)
			}
			onUninstallError={(error) =>
				toast.error(`Failed to uninstall package: ${getErrorMessage(error)}`)
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
						className="absolute left-0 top-0 bottom-0 z-10 w-8 flex items-center justify-center bg-linear-to-r from-background/80 to-transparent opacity-0 group-hover/swimlane:opacity-100 transition-opacity"
					>
						<ChevronRight className="h-4 w-4 rotate-180" />
					</button>
				)}
				<div
					ref={scrollRef}
					className="flex gap-3 overflow-x-auto scrollbar-none pb-1"
				>
					{packages.map((pkg) => (
						<div key={pkg.id} className="w-70 shrink-0">
							<PackageCard pkg={pkg} />
						</div>
					))}
				</div>
				{canScrollRight && (
					<button
						type="button"
						onClick={() => scroll("right")}
						className="absolute right-0 top-0 bottom-0 z-10 w-8 flex items-center justify-center bg-linear-to-l from-background/80 to-transparent opacity-0 group-hover/swimlane:opacity-100 transition-opacity"
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
						<SelectTrigger className="w-37.5">
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
				<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
					{PACKAGE_CARD_SKELETON_KEYS.map((key) => (
						<PackageCardSkeleton key={key} />
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

	return (
		<main className="flex-col flex grow max-h-full p-6 overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-7xl space-y-8">
				<div className="space-y-2">
					<h1 className="text-2xl font-semibold tracking-tight">Packages</h1>
					<p className="text-sm text-muted-foreground">
						Discover and install WASM node packages
					</p>
				</div>
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

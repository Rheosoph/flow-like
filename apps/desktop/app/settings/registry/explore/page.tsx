"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Badge,
	Button,
	Input,
	PackageStatusBadge,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
	StorePackageDetail,
	useBackend,
} from "@flow-like/flow-like-ui";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import type {
	InstalledPackage,
	PackageSummary,
	SearchFilters,
	SearchResults,
} from "@flow-like/flow-like-ui/lib/schema/wasm";
import {
	Check,
	Download,
	Loader2,
	Package,
	RefreshCw,
	Search,
	Shield,
	Trash2,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { usePackageStatusMap } from "../../../../hooks/use-package-status";
import { fetcher } from "../../../../lib/api";

type SortOption =
	| "relevance"
	| "name"
	| "downloads"
	| "updated_at"
	| "created_at";

const SORT_OPTIONS: { value: SortOption; label: string }[] = [
	{ value: "relevance", label: "Relevance" },
	{ value: "downloads", label: "Most Downloads" },
	{ value: "updated_at", label: "Recently Updated" },
	{ value: "created_at", label: "Newest" },
	{ value: "name", label: "Name" },
];

function PackageItem({
	pkg,
	isInstalled,
	installedVersion,
	onInstall,
	onUninstall,
	onSelect,
	isLoading,
	compileStatus,
}: {
	pkg: PackageSummary;
	isInstalled: boolean;
	installedVersion?: string;
	onInstall: (id: string) => void;
	onUninstall: (id: string) => void;
	onSelect?: (id: string) => void;
	isLoading: boolean;
	compileStatus?:
		| "idle"
		| "downloading"
		| "compiling"
		| "ready"
		| "error"
		| "stale";
}) {
	const { t } = useTranslation("common");
	return (
		<div
			className="rounded-xl border border-border/20 bg-card/50 hover:bg-muted/10 p-4 transition-all cursor-pointer"
			onClick={() => onSelect?.(pkg.id)}
			onKeyDown={(e) => {
				if (e.key === "Enter" || e.key === " ") {
					e.preventDefault();
					onSelect?.(pkg.id);
				}
			}}
			role="button"
			tabIndex={0}
		>
			<div className="flex items-start justify-between gap-3 mb-2">
				<div className="flex items-center gap-2 min-w-0 flex-1">
					<span className="truncate text-sm font-medium">{pkg.name}</span>
					{pkg.verified && (
						<Shield className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
					)}
					<span className="text-[11px] text-muted-foreground/50 shrink-0">{`v${pkg.latestVersion}`}</span>
					{compileStatus && compileStatus !== "idle" && (
						<PackageStatusBadge status={compileStatus} />
					)}
				</div>
			</div>

			{pkg.description && (
				<p className="text-xs text-muted-foreground/70 line-clamp-2 mb-3 leading-relaxed">
					{pkg.description}
				</p>
			)}

			{pkg.keywords.length > 0 && (
				<div className="flex flex-wrap gap-1.5 mb-3">
					{pkg.keywords.slice(0, 4).map((keyword) => (
						<Badge
							key={keyword}
							variant="outline"
							className="text-[10px] px-1.5 py-0 h-5 font-normal text-muted-foreground/60 border-border/30"
						>
							{keyword}
						</Badge>
					))}
					{pkg.keywords.length > 4 && (
						<span className="text-[10px] text-muted-foreground/40 self-center">
							+{pkg.keywords.length - 4}
						</span>
					)}
				</div>
			)}

			<div className="flex items-center justify-between">
				<span className="text-[11px] text-muted-foreground/50">
					{pkg.downloadCount.toLocaleString()} downloads
				</span>
				{isInstalled ? (
					<div className="flex items-center gap-2">
						<span className="text-[11px] text-muted-foreground/60 flex items-center gap-1">
							<Check className="h-3 w-3 text-green-500/70" />{`v${installedVersion}`}</span>
						<Button
							size="sm"
							variant="ghost"
							className="h-7 w-7 rounded-full text-muted-foreground/60 hover:text-destructive hover:bg-destructive/10 p-0"
							onClick={(e) => {
								e.stopPropagation();
								onUninstall(pkg.id);
							}}
							disabled={isLoading}
						>
							{isLoading ? (
								<Loader2 className="h-3.5 w-3.5 animate-spin" />
							) : (
								<Trash2 className="h-3.5 w-3.5" />
							)}
						</Button>
					</div>
				) : (
					<Button
						size="sm"
						variant="ghost"
						className="h-7 gap-1.5 rounded-full text-xs text-muted-foreground/70 hover:text-foreground/80 hover:bg-muted/30 px-3"
						onClick={(e) => {
							e.stopPropagation();
							onInstall(pkg.id);
						}}
						disabled={isLoading}
					>
						{isLoading ? (
							<Loader2 className="h-3.5 w-3.5 animate-spin" />
						) : (
							<>
								<Download className="h-3.5 w-3.5" />
								{t('install', 'Install')}
							</>
						)}
					</Button>
				)}
			</div>
		</div>
	);
}

function PackageItemSkeleton() {
	return (
		<div className="rounded-xl border border-border/20 bg-card/50 p-4">
			<div className="flex items-center gap-2 mb-2">
				<Skeleton className="h-4 w-28" />
				<Skeleton className="h-3 w-12" />
			</div>
			<Skeleton className="h-3 w-full mb-1" />
			<Skeleton className="h-3 w-3/4 mb-3" />
			<div className="flex gap-1.5 mb-3">
				<Skeleton className="h-5 w-12 rounded-full" />
				<Skeleton className="h-5 w-16 rounded-full" />
				<Skeleton className="h-5 w-10 rounded-full" />
			</div>
			<div className="flex items-center justify-between">
				<Skeleton className="h-3 w-20" />
				<Skeleton className="h-7 w-16 rounded-full" />
			</div>
		</div>
	);
}

export default function ExplorePackagesPage() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const auth = useAuth();
	const [isInitialized, setIsInitialized] = useState(false);
	const [isInitializing, setIsInitializing] = useState(false);
	const [searchQuery, setSearchQuery] = useState("");
	const [sortBy, setSortBy] = useState<SortOption>("relevance");
	const [selectedPackageId, setSelectedPackageId] = useState<string | null>(
		null,
	);
	const [searchResults, setSearchResults] = useState<SearchResults | null>(
		null,
	);
	const [installedPackages, setInstalledPackages] = useState<
		InstalledPackage[]
	>([]);
	const [isLoading, setIsLoading] = useState(false);
	const [loadingPackage, setLoadingPackage] = useState<string | null>(null);
	const packageStatusMap = usePackageStatusMap();

	const initRegistry = useCallback(async () => {
		if (!backend?.registryState || isInitialized || isInitializing) return;
		setIsInitializing(true);
		try {
			await backend.registryState.init();
			setIsInitialized(true);
		} catch (err) {
			console.error("Failed to initialize registry:", err);
		} finally {
			setIsInitializing(false);
		}
	}, [backend?.registryState, isInitialized, isInitializing]);

	useEffect(() => {
		initRegistry();
	}, [initRegistry]);

	const isAuthenticated = !!auth?.user?.access_token;

	const fetchPackages = useCallback(async () => {
		if (!backend?.registryState || !isInitialized) return;
		setIsLoading(true);
		try {
			const filters: SearchFilters = {
				query: searchQuery || undefined,
				sortBy,
				sortDesc: true,
				limit: 20,
			};
			const results = await backend.registryState.searchPackages(filters);
			setSearchResults(results);
		} catch (err) {
			console.error("Failed to search packages:", err);
		} finally {
			setIsLoading(false);
		}
	}, [
		backend?.registryState,
		isInitialized,
		searchQuery,
		sortBy,
		isAuthenticated,
	]);

	const fetchInstalled = useCallback(async () => {
		if (!backend?.registryState || !isInitialized) return;
		try {
			const packages = await backend.registryState.getInstalledPackages();
			setInstalledPackages(packages);
		} catch (err) {
			console.error("Failed to fetch installed packages:", err);
		}
	}, [backend?.registryState, isInitialized]);

	useEffect(() => {
		if (isInitialized) {
			fetchPackages();
			fetchInstalled();
		}
	}, [isInitialized, fetchPackages, fetchInstalled]);

	const handleInstall = async (packageId: string) => {
		if (!backend?.registryState) return;
		setLoadingPackage(packageId);
		try {
			await backend.registryState.installPackage(packageId);
			toast.success("Package installed successfully");
			await fetchInstalled();
			await fetchPackages();
		} catch (err) {
			console.error("Failed to install package:", err);
			toast.error(`Failed to install: ${getErrorMessage(err)}`);
		} finally {
			setLoadingPackage(null);
		}
	};

	const handleUninstall = async (packageId: string) => {
		if (!backend?.registryState) return;
		setLoadingPackage(packageId);
		try {
			await backend.registryState.uninstallPackage(packageId);
			toast.success("Package uninstalled");
			await fetchInstalled();
			await fetchPackages();
		} catch (err) {
			console.error("Failed to uninstall package:", err);
			toast.error(`Failed to uninstall: ${getErrorMessage(err)}`);
		} finally {
			setLoadingPackage(null);
		}
	};

	const installedIds = new Set(installedPackages.map((p) => p.id));
	const installedVersionMap = new Map(
		installedPackages.map((p) => [p.id, p.version]),
	);

	if (isInitializing) {
		return (
			<div className="flex items-center justify-center h-full">
				<div className="flex flex-col items-center gap-3">
					<Loader2 className="h-5 w-5 animate-spin text-muted-foreground/50" />
					<p className="text-xs text-muted-foreground/50">
						{t('initializingRegistry', 'Initializing registry…')}
					</p>
				</div>
			</div>
		);
	}

	if (selectedPackageId) {
		return (
			<StorePackageDetail
				packageId={selectedPackageId}
				onBack={() => setSelectedPackageId(null)}
				onInstallSuccess={() => {
					toast.success("Package installed");
					fetchInstalled();
				}}
				onUninstallSuccess={() => {
					toast.success("Package uninstalled");
					fetchInstalled();
				}}
				onInstallError={(error) =>
					toast.error(`Failed to install: ${getErrorMessage(error)}`)
				}
				onUninstallError={(error) =>
					toast.error(`Failed to uninstall: ${getErrorMessage(error)}`)
				}
				onDeleteSuccess={() => {
					setSelectedPackageId(null);
					fetchInstalled();
				}}
				fetcher={fetcher}
				auth={auth}
				compileStatus={packageStatusMap.get(selectedPackageId)}
			/>
		);
	}

	return (
		<div className="flex flex-col h-full gap-4">
			<div className="flex items-center gap-2">
				<div className="relative flex-1 max-w-lg">
					<Search className="absolute left-3.5 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground/40" />
					<Input
						placeholder={t('searchPackages2', 'Search packages…')}
						value={searchQuery}
						onChange={(e) => setSearchQuery(e.target.value)}
						onKeyDown={(e) => e.key === "Enter" && fetchPackages()}
						className="pl-11 h-10 rounded-full bg-muted/30 border-transparent focus:border-border/40 focus:bg-muted/50 transition-all text-sm"
					/>
				</div>

				<Select
					value={sortBy}
					onValueChange={(v) => setSortBy(v as SortOption)}
				>
					<SelectTrigger className="w-40 h-10 rounded-full bg-muted/30 border-transparent text-sm text-muted-foreground/70">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{SORT_OPTIONS.map((opt) => (
							<SelectItem key={opt.value} value={opt.value}>
								{opt.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>

				<Button
					variant="ghost"
					size="icon"
					className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
					onClick={() => {
						fetchPackages();
						fetchInstalled();
					}}
					disabled={isLoading}
				>
					<RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
				</Button>
			</div>

			{searchResults && (
				<p className="text-xs text-muted-foreground/50">{t('countPackagesFound', { defaultValue_one: '{{count}} package found', defaultValue_other: '{{count}} packages found', count: searchResults.totalCount })}
				</p>
			)}

			<div className="flex-1 overflow-y-auto">
				{isLoading ? (
					<div
						className="grid gap-3"
						style={{
							gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))",
						}}
					>
						{Array.from({ length: 6 }).map((_, i) => (
							<PackageItemSkeleton key={i} />
						))}
					</div>
				) : searchResults?.packages.length ? (
					<div
						className="grid gap-3"
						style={{
							gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))",
						}}
					>
						{searchResults.packages.map((pkg) => (
							<PackageItem
								key={pkg.id}
								pkg={pkg}
								isInstalled={installedIds.has(pkg.id)}
								installedVersion={installedVersionMap.get(pkg.id)}
								onInstall={handleInstall}
								onUninstall={handleUninstall}
								onSelect={setSelectedPackageId}
								isLoading={loadingPackage === pkg.id}
								compileStatus={packageStatusMap.get(pkg.id)}
							/>
						))}
					</div>
				) : (
					<div className="flex flex-col items-center justify-center py-20 text-center">
						<Package className="h-8 w-8 text-muted-foreground/30 mb-3" />
						<p className="text-sm text-muted-foreground/50">
							{t('noPackagesFound', 'No packages found')}
						</p>
						<p className="text-xs text-muted-foreground/30 mt-1">
							{t('tryADifferentSearchTerm', 'Try a different search term')}
						</p>
					</div>
				)}
			</div>
		</div>
	);
}

"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Input,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
	StorePackageDetail,
	useBackend,
} from "@flow-like/flow-like-ui";
import type {
	InstalledPackage,
	PackageSummary,
	SearchFilters,
	SearchResults,
} from "@flow-like/flow-like-ui/lib/schema/wasm";
import { motion } from "framer-motion";
import {
	ArrowUpDown,
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
import { fetcher } from "../../../../lib/api";
import { getErrorMessage } from "../error-utils";

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

function PackageCard({
	pkg,
	isInstalled,
	installedVersion,
	onInstall,
	onUninstall,
	onSelect,
	isLoading,
}: {
	pkg: PackageSummary;
	isInstalled: boolean;
	installedVersion?: string;
	onInstall: (id: string) => void;
	onUninstall: (id: string) => void;
	onSelect?: (id: string) => void;
	isLoading: boolean;
}) {
	const { t } = useTranslation("common");
	return (
		<motion.div
			initial={{ opacity: 0, y: 10 }}
			animate={{ opacity: 1, y: 0 }}
			transition={{ duration: 0.2 }}
			className="cursor-pointer"
			onClick={() => onSelect?.(pkg.id)}
		>
			<Card className="hover:shadow-md transition-shadow">
				<CardHeader className="pb-2">
					<div className="flex items-start justify-between gap-2">
						<div className="flex items-center gap-2 min-w-0">
							<Package className="h-5 w-5 shrink-0 text-muted-foreground" />
							<CardTitle className="text-base truncate">{pkg.name}</CardTitle>
						</div>
						<div className="flex items-center gap-1 shrink-0">
							{pkg.verified && (
								<Badge variant="secondary" className="gap-1">
									<Shield className="h-3 w-3" />
									{t('verified', 'Verified')}
								</Badge>
							)}
							<Badge variant="outline">{`v${pkg.latestVersion}`}</Badge>
						</div>
					</div>
					<CardDescription className="line-clamp-2">
						{pkg.description}
					</CardDescription>
				</CardHeader>
				<CardContent>
					<div className="flex flex-wrap gap-1 mb-3">
						{pkg.keywords.slice(0, 3).map((keyword) => (
							<Badge key={keyword} variant="outline" className="text-xs">
								{keyword}
							</Badge>
						))}
						{pkg.keywords.length > 3 && (
							<Badge variant="outline" className="text-xs">
								+{pkg.keywords.length - 3}
							</Badge>
						)}
					</div>
					<div className="flex items-center justify-between">
						<span className="text-xs text-muted-foreground">
							{pkg.downloadCount.toLocaleString()} downloads
						</span>
						{isInstalled ? (
							<div className="flex items-center gap-2">
								<span className="text-xs text-green-600 flex items-center gap-1">
									<Check className="h-3 w-3" />{`v${installedVersion}`}</span>
								<Button
									size="sm"
									variant="destructive"
									onClick={(e) => {
										e.stopPropagation();
										onUninstall(pkg.id);
									}}
									disabled={isLoading}
								>
									{isLoading ? (
										<Loader2 className="h-4 w-4 animate-spin" />
									) : (
										<Trash2 className="h-4 w-4" />
									)}
								</Button>
							</div>
						) : (
							<Button
								size="sm"
								onClick={(e) => {
									e.stopPropagation();
									onInstall(pkg.id);
								}}
								disabled={isLoading}
							>
								{isLoading ? (
									<Loader2 className="h-4 w-4 animate-spin" />
								) : (
									<>
										<Download className="h-4 w-4 mr-1" />
										{t('install', 'Install')}
									</>
								)}
							</Button>
						)}
					</div>
				</CardContent>
			</Card>
		</motion.div>
	);
}

function PackageCardSkeleton() {
	return (
		<Card>
			<CardHeader className="pb-2">
				<div className="flex items-start justify-between gap-2">
					<Skeleton className="h-5 w-32" />
					<Skeleton className="h-5 w-16" />
				</div>
				<Skeleton className="h-4 w-full mt-2" />
				<Skeleton className="h-4 w-3/4" />
			</CardHeader>
			<CardContent>
				<div className="flex gap-1 mb-3">
					<Skeleton className="h-5 w-12" />
					<Skeleton className="h-5 w-16" />
					<Skeleton className="h-5 w-10" />
				</div>
				<div className="flex items-center justify-between">
					<Skeleton className="h-4 w-24" />
					<Skeleton className="h-8 w-20" />
				</div>
			</CardContent>
		</Card>
	);
}

export default function ExplorePackagesPage() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const auth = useAuth();
	const [isInitialized, setIsInitialized] = useState(false);
	const [isInitializing, setIsInitializing] = useState(false);
	const [selectedPackageId, setSelectedPackageId] = useState<string | null>(
		null,
	);

	const [searchQuery, setSearchQuery] = useState("");
	const [sortBy, setSortBy] = useState<SortOption>("relevance");
	const [searchResults, setSearchResults] = useState<SearchResults | null>(
		null,
	);
	const [installedPackages, setInstalledPackages] = useState<
		InstalledPackage[]
	>([]);
	const [isLoading, setIsLoading] = useState(false);
	const [loadingPackage, setLoadingPackage] = useState<string | null>(null);

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
	}, [backend?.registryState, isInitialized, searchQuery, sortBy]);

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
				<div className="text-center">
					<Loader2 className="h-8 w-8 animate-spin mx-auto mb-4" />
					<p className="text-muted-foreground">{t('initializingRegistry', 'Initializing registry...')}</p>
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
			/>
		);
	}

	return (
		<div className="flex flex-col h-full space-y-4">
			<div className="flex items-center justify-between">
				<div>
					<h1 className="text-2xl font-bold">{t('explorePackages', 'Explore Packages')}</h1>
					<p className="text-sm text-muted-foreground">
						{`Discover and install WASM node packages from the registry`}
					</p>
				</div>
				<Button
					variant="outline"
					size="sm"
					onClick={() => {
						fetchPackages();
						fetchInstalled();
					}}
					disabled={isLoading}
				>
					<RefreshCw
						className={`h-4 w-4 mr-1 ${isLoading ? "animate-spin" : ""}`}
					/>
					{t('refresh', 'Refresh')}
				</Button>
			</div>

			<div className="flex items-center gap-2">
				<div className="relative flex-1 max-w-md">
					<Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
					<Input
						placeholder={t('searchPackages', 'Search packages...')}
						value={searchQuery}
						onChange={(e) => setSearchQuery(e.target.value)}
						onKeyDown={(e) => e.key === "Enter" && fetchPackages()}
						className="pl-9"
					/>
				</div>
				<Select
					value={sortBy}
					onValueChange={(v) => setSortBy(v as SortOption)}
				>
					<SelectTrigger className="w-45">
						<ArrowUpDown className="h-4 w-4 mr-2" />
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
				<Button onClick={fetchPackages} disabled={isLoading}>
					{isLoading ? (
						<Loader2 className="h-4 w-4 animate-spin" />
					) : (
						<Search className="h-4 w-4" />
					)}
				</Button>
			</div>

			{searchResults && (
				<p className="text-sm text-muted-foreground">{t('countPackagesFound', { defaultValue_one: '{{count}} package found', defaultValue_other: '{{count}} packages found', count: searchResults.totalCount })}</p>
			)}

			<div className="flex-1 overflow-y-auto">
				{isLoading ? (
					<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
						{Array.from({ length: 6 }).map((_, i) => (
							<PackageCardSkeleton key={i} />
						))}
					</div>
				) : searchResults?.packages.length ? (
					<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
						{searchResults.packages.map((pkg) => (
							<PackageCard
								key={pkg.id}
								pkg={pkg}
								isInstalled={installedIds.has(pkg.id)}
								installedVersion={installedVersionMap.get(pkg.id)}
								onInstall={handleInstall}
								onUninstall={handleUninstall}
								onSelect={setSelectedPackageId}
								isLoading={loadingPackage === pkg.id}
							/>
						))}
					</div>
				) : (
					<div className="flex flex-col items-center justify-center py-12 text-center">
						<Package className="h-12 w-12 text-muted-foreground mb-4" />
						<p className="text-muted-foreground">{t('noPackagesFound', 'No packages found')}</p>
						<p className="text-sm text-muted-foreground mt-2">
							{t('tryADifferentSearchTermOrBrowseAllPackages', 'Try a different search term or browse all packages')}
						</p>
					</div>
				)}
			</div>
		</div>
	);
}

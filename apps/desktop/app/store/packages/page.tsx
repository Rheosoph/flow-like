"use client";

import {
	Alert,
	AlertDescription,
	Badge,
	Button,
	EmptyState,
	Input,
	type InstalledPackage,
	PackageDetailWrapper,
	PackageListContent,
	PackageStatusBadge,
	type PackageUpdate,
	Skeleton,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	useSearch,
} from "@flow-like/flow-like-ui";
import { ExploreHubLayout } from "@flow-like/flow-like-ui/components/store/explore-hub-layout";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
} from "@flow-like/flow-like-ui/components/ui/avatar";
import {
	hashToGradient,
	useThemeInfo,
} from "@flow-like/flow-like-ui/hooks/use-theme-gradient";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import {
	AlertCircle,
	CheckCircle,
	Download,
	Globe,
	Loader2,
	Package,
	RefreshCw,
	Search,
	Sparkles,
	Trash2,
	X,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import {
	type ReactNode,
	Suspense,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { usePackageStatusMap } from "../../../hooks/use-package-status";
import { fetcher } from "../../../lib/api";

const INSTALLED_PACKAGE_SKELETON_KEYS = Array.from(
	{ length: 6 },
	(_, index) => `installed-package-skeleton-${index}`,
);

// ─── Installed Tab ───────────────────────────────────────────────────────────

function InstalledPackageCard({
	pkg,
	updateAvailable,
	onUninstall,
	onUpdate,
	isUpdating,
	isUninstalling,
	compileStatus,
}: {
	pkg: InstalledPackage;
	updateAvailable?: string;
	onUninstall: () => void;
	onUpdate: () => void;
	isUpdating: boolean;
	isUninstalling: boolean;
	compileStatus?:
		| "idle"
		| "downloading"
		| "compiling"
		| "ready"
		| "error"
		| "stale";
}) {
	const { t } = useTranslation("common");
	const { primaryHue, isDark } = useThemeInfo();
	const gradient = useMemo(
		() => hashToGradient(pkg.id, primaryHue, isDark),
		[pkg.id, primaryHue, isDark],
	);
	const displayName = pkg.metadata?.name ?? pkg.manifest.name;
	const displayDesc = pkg.metadata?.description ?? pkg.manifest.description;
	const icon = pkg.metadata?.icon;
	const thumbnail = pkg.metadata?.thumbnail;

	return (
		<Link
			href={`/store/packages?tab=installed&id=${pkg.id}`}
			className="group relative flex min-h-32 flex-row rounded-lg border border-border/40 border-dashed bg-card/60 backdrop-blur-sm overflow-hidden transition-all hover:border-primary/40 hover:bg-card/80 hover:shadow-lg cursor-pointer"
		>
			{/* Left gradient accent */}
			<div className="relative w-28 shrink-0 overflow-hidden">
				{thumbnail ? (
					<img
						src={thumbnail}
						alt=""
						className="absolute inset-0 w-full h-full object-cover"
					/>
				) : (
					<div
						className="absolute inset-0"
						style={{
							background: `linear-gradient(${gradient.angle}deg, ${gradient.from}, ${gradient.to})`,
							opacity: gradient.opacity,
						}}
					/>
				)}
				<div className="absolute inset-0 bg-linear-to-r from-transparent to-card/80" />
				<div className="absolute inset-0 flex items-center justify-center">
					<Avatar className="w-10 h-10 rounded-lg shadow-md border-2 border-background/20 bg-background/30 backdrop-blur-sm">
						{icon ? (
							<AvatarImage
								src={icon}
								alt={displayName}
								className="rounded-lg"
							/>
						) : null}
						<AvatarFallback className="rounded-lg text-xs font-mono font-bold bg-background/20 text-white/80">
							<Package className="h-5 w-5" />
						</AvatarFallback>
					</Avatar>
				</div>
			</div>

			{/* Content */}
			<div className="flex-1 min-w-0 px-3.5 py-3 flex flex-col gap-1">
				{/* Top row: name + badges */}
				<div className="flex flex-wrap items-center gap-1.5 min-w-0">
					<h3 className="w-full min-w-0 text-sm font-semibold font-mono truncate group-hover:text-primary transition-colors sm:w-auto sm:flex-1">
						{displayName}
					</h3>
					<span className="text-[10px] font-mono text-muted-foreground/50 shrink-0">
						{`v${pkg.version}`}
						{updateAvailable && (
							<span className="text-primary">{`→ v${updateAvailable}`}</span>
						)}
					</span>
					<div className="flex items-center gap-1 ml-auto shrink-0">
						<span className="inline-flex items-center gap-1 rounded bg-background/80 border border-border/40 px-1.5 py-0.5 text-[10px] text-muted-foreground font-mono">
							<Globe className="h-2.5 w-2.5" /> installed
						</span>
						{compileStatus && compileStatus !== "idle" && (
							<PackageStatusBadge status={compileStatus} />
						)}
						{updateAvailable && (
							<Badge
								variant="secondary"
								className="gap-0.5 text-[10px] px-1.5 py-0.5"
							>
								<AlertCircle className="h-2.5 w-2.5" />
								{t("update", "Update")}
							</Badge>
						)}
					</div>
				</div>

				{/* Description */}
				<p className="text-xs text-muted-foreground/80 line-clamp-1 leading-relaxed">
					{displayDesc}
				</p>

				{/* Footer with keywords + actions */}
				<div className="flex items-center gap-1 mt-auto pt-1.5 border-t border-border/20">
					{pkg.manifest.keywords.length > 0 && (
						<div className="flex items-center gap-1 flex-1 min-w-0 overflow-hidden">
							{pkg.manifest.keywords.slice(0, 2).map((kw) => (
								<span
									key={kw}
									className="rounded bg-muted/30 px-1.5 py-0.5 text-[10px] text-muted-foreground/60 font-mono truncate"
								>
									{kw}
								</span>
							))}
						</div>
					)}
					<div className="flex items-center gap-0.5 shrink-0 ml-auto">
						{updateAvailable && (
							<Button
								size="icon"
								variant="ghost"
								className="h-6 w-6 rounded-full text-primary hover:text-primary hover:bg-primary/10"
								onClick={(e) => {
									e.preventDefault();
									e.stopPropagation();
									onUpdate();
								}}
								disabled={isUpdating}
							>
								{isUpdating ? (
									<RefreshCw className="h-3 w-3 animate-spin" />
								) : (
									<RefreshCw className="h-3 w-3" />
								)}
							</Button>
						)}
						<Button
							size="icon"
							variant="ghost"
							className="h-6 w-6 rounded-full text-destructive hover:text-destructive hover:bg-destructive/10"
							onClick={(e) => {
								e.preventDefault();
								e.stopPropagation();
								onUninstall();
							}}
							disabled={isUninstalling}
						>
							{isUninstalling ? (
								<Loader2 className="h-3 w-3 animate-spin" />
							) : (
								<Trash2 className="h-3 w-3" />
							)}
						</Button>
					</div>
				</div>
			</div>
		</Link>
	);
}

function InstalledContent({ navigation }: { navigation: ReactNode }) {
	const { t } = useTranslation("common");
	const { t: storeT } = useTranslation("store");
	const searchRef = useRef<HTMLInputElement>(null);
	const queryClient = useQueryClient();
	const auth = useAuth();
	const [searchQuery, setSearchQuery] = useState("");
	const packageStatusMap = usePackageStatusMap();
	const [updatingPackages, setUpdatingPackages] = useState<Set<string>>(
		new Set(),
	);
	const [uninstallingPackages, setUninstallingPackages] = useState<Set<string>>(
		new Set(),
	);

	const registryReady = useQuery({
		queryKey: ["registry-init"],
		queryFn: async () => {
			await invoke("registry_init", { config: null });
			return true;
		},
		staleTime: Number.POSITIVE_INFINITY,
	});

	const installedPackages = useQuery({
		queryKey: ["installed-packages"],
		queryFn: async () => {
			return invoke<InstalledPackage[]>("registry_get_installed_packages");
		},
		enabled: registryReady.data === true,
	});

	const availableUpdates = useQuery({
		queryKey: ["available-updates"],
		queryFn: async () => {
			return invoke<PackageUpdate[]>("registry_check_for_updates", {
				token: auth.user?.access_token,
			});
		},
		enabled: registryReady.data === true,
	});

	const updateMutation = useMutation({
		mutationFn: async ({
			packageId,
			version,
		}: { packageId: string; version?: string }) => {
			setUpdatingPackages((prev) => new Set(prev).add(packageId));
			await invoke("registry_update_package", {
				packageId,
				version,
				token: auth.user?.access_token,
			});
		},
		onSuccess: (_, { packageId }) => {
			toast.success("Package updated successfully");
			queryClient.invalidateQueries({ queryKey: ["installed-packages"] });
			queryClient.invalidateQueries({ queryKey: ["available-updates"] });
			queryClient.invalidateQueries({
				queryKey: ["installed-package", packageId],
			});
		},
		onError: (error: unknown) => {
			toast.error(`Failed to update package: ${getErrorMessage(error)}`);
		},
		onSettled: (_, __, { packageId }) => {
			setUpdatingPackages((prev) => {
				const next = new Set(prev);
				next.delete(packageId);
				return next;
			});
		},
	});

	const uninstallMutation = useMutation({
		mutationFn: async (packageId: string) => {
			setUninstallingPackages((prev) => new Set(prev).add(packageId));
			await invoke("registry_uninstall_package", { packageId });
		},
		onSuccess: (_, packageId) => {
			toast.success("Package uninstalled");
			queryClient.invalidateQueries({ queryKey: ["installed-packages"] });
			queryClient.invalidateQueries({ queryKey: ["available-updates"] });
			queryClient.invalidateQueries({
				queryKey: ["installed-package", packageId],
			});
		},
		onError: (error: unknown) => {
			toast.error(`Failed to uninstall package: ${getErrorMessage(error)}`);
		},
		onSettled: (_, __, packageId) => {
			setUninstallingPackages((prev) => {
				const next = new Set(prev);
				next.delete(packageId);
				return next;
			});
		},
	});

	const updateAllMutation = useMutation({
		mutationFn: async () => {
			const updates = availableUpdates.data ?? [];
			for (const update of updates) {
				await invoke("registry_update_package", {
					packageId: update.packageId,
					version: update.latestVersion,
					token: auth.user?.access_token,
				});
			}
		},
		onSuccess: () => {
			toast.success("All packages updated");
			queryClient.invalidateQueries({ queryKey: ["installed-packages"] });
			queryClient.invalidateQueries({ queryKey: ["available-updates"] });
		},
		onError: (error: unknown) => {
			toast.error(`Failed to update packages: ${getErrorMessage(error)}`);
		},
	});

	const updatesMap = new Map(
		(availableUpdates.data ?? []).map((u) => [u.packageId, u.latestVersion]),
	);

	const registryPackages = useMemo(
		() =>
			(installedPackages.data ?? []).filter(
				(pkg) => pkg.source.type === "remote",
			),
		[installedPackages.data],
	);

	const filteredPackages = useSearch(registryPackages, searchQuery, {
		fields: [
			"manifest.name",
			"manifest.id",
			"manifest.description",
			"manifest.keywords",
			"manifest.authors",
		],
		boost: { "manifest.name": 3, "manifest.id": 2, "manifest.keywords": 1.5 },
	});

	const hasUpdates = (availableUpdates.data?.length ?? 0) > 0;

	const error = registryReady.error ?? installedPackages.error;
	const isInitialLoading =
		!error && (registryReady.isPending || installedPackages.isPending);
	const isBusy =
		isInitialLoading ||
		registryReady.isFetching ||
		installedPackages.isFetching;

	return (
		<ExploreHubLayout
			active="packages"
			subtitle={storeT(
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
							aria-label={t(
								"searchInstalledPackages",
								"Search installed packages…",
							)}
							aria-controls="installed-package-results"
							placeholder={t(
								"searchInstalledPackages",
								"Search installed packages…",
							)}
							value={searchQuery}
							onChange={(e) => setSearchQuery(e.target.value)}
							className="h-12 rounded-xl border-border/60 bg-muted/30 pr-12 pl-12 text-sm shadow-none transition-colors focus-visible:bg-background [&::-webkit-search-cancel-button]:appearance-none"
						/>
					</div>
					<div className="flex shrink-0 items-center justify-between gap-2 sm:justify-start">
						<div className="min-w-0 max-w-48 flex-1 sm:w-48 sm:flex-none">
							<Button
								size="sm"
								onClick={() => updateAllMutation.mutate()}
								disabled={!hasUpdates || updateAllMutation.isPending}
								className={`h-12 w-full gap-1.5 rounded-xl ${hasUpdates ? "" : "invisible"}`}
							>
								<RefreshCw
									aria-hidden="true"
									className={`h-4 w-4 ${updateAllMutation.isPending ? "animate-spin" : ""}`}
								/>
								{t("updateAllLength", "Update All ({{length}})", {
									length: availableUpdates.data?.length ?? 0,
								})}
							</Button>
						</div>
						<div className="w-28 shrink-0">
							<Button
								variant="ghost"
								size="sm"
								disabled={!searchQuery}
								onClick={() => {
									setSearchQuery("");
									searchRef.current?.focus();
								}}
								className={`h-12 w-full rounded-xl text-muted-foreground ${searchQuery ? "" : "invisible"}`}
							>
								<X aria-hidden="true" className="mr-1 h-3.5 w-3.5" />
								{storeT("clearFilters", "Clear filters")}
							</Button>
						</div>
					</div>
				</div>
			}
			filters={
				<div className="flex min-h-10 items-center gap-3">{navigation}</div>
			}
		>
			<section
				id="installed-package-results"
				className="space-y-5"
				aria-busy={isBusy}
			>
				<output
					className="flex min-h-5 gap-4 text-sm text-muted-foreground"
					aria-live="polite"
				>
					{isBusy ? (
						storeT("loadingPackages", "Loading packages…")
					) : error ? (
						storeT("packagesCouldNotBeLoaded", "Packages could not be loaded.")
					) : (
						<>
							<span className="flex items-center gap-1.5">
								<CheckCircle
									aria-hidden="true"
									className="h-3.5 w-3.5 text-green-500"
								/>
								{searchQuery
									? t(
											"lengthMatchingPackages",
											"{{length}} matching packages",
											{ length: filteredPackages.length },
										)
									: t("lengthInstalled", "{{length}} installed", {
											length: registryPackages.length,
										})}
							</span>
							{hasUpdates && (
								<span className="flex items-center gap-1.5">
									<AlertCircle
										aria-hidden="true"
										className="h-3.5 w-3.5 text-yellow-500"
									/>
									{t("lengthUpdates", "{{length}} updates", {
										length: availableUpdates.data?.length,
									})}
								</span>
							)}
						</>
					)}
				</output>
				{error && (
					<Alert variant="destructive" className="rounded-xl">
						<AlertCircle className="h-4 w-4" />
						<AlertDescription className="flex flex-wrap items-center justify-between gap-3">
							{storeT(
								"packagesLoadError",
								"Packages could not be loaded. Please try again.",
							)}
							<Button
								variant="outline"
								size="sm"
								disabled={isBusy}
								onClick={() =>
									registryReady.error
										? registryReady.refetch()
										: installedPackages.refetch()
								}
							>
								{t("retry", "Retry")}
							</Button>
						</AlertDescription>
					</Alert>
				)}
				{isInitialLoading ? (
					<div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
						{INSTALLED_PACKAGE_SKELETON_KEYS.map((key) => (
							<div
								key={key}
								className="flex min-h-32 overflow-hidden rounded-lg border border-dashed border-border/40 bg-card/60"
							>
								<Skeleton className="w-28 shrink-0 rounded-none" />
								<div className="flex-1 space-y-2 px-3.5 py-3">
									<Skeleton className="h-4 w-28 rounded" />
									<Skeleton className="h-3 w-full rounded" />
									<Skeleton className="h-3 w-3/4 rounded" />
									<Skeleton className="mt-3 h-6 w-full rounded" />
								</div>
							</div>
						))}
					</div>
				) : filteredPackages.length === 0 ? (
					!error && (
						<EmptyState
							icons={[Download, Package, Sparkles]}
							title={
								searchQuery
									? t("noMatchingPackages", "No matching packages")
									: t(
											"noRegistryPackagesInstalled",
											"No registry packages installed",
										)
							}
							description={
								searchQuery
									? t("tryADifferentSearchTerm", "Try a different search term")
									: t(
											"browseTheExploreTabToFindAndInstallPackages",
											"Browse the Explore tab to find and install packages",
										)
							}
							className="rounded-2xl border border-dashed border-border/30 bg-muted/5"
						/>
					)
				) : (
					<div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
						{filteredPackages.map((pkg) => (
							<InstalledPackageCard
								key={pkg.id}
								pkg={pkg}
								updateAvailable={updatesMap.get(pkg.id)}
								onUninstall={() => uninstallMutation.mutate(pkg.id)}
								onUpdate={() =>
									updateMutation.mutate({
										packageId: pkg.id,
										version: updatesMap.get(pkg.id),
									})
								}
								isUpdating={updatingPackages.has(pkg.id)}
								isUninstalling={uninstallingPackages.has(pkg.id)}
								compileStatus={packageStatusMap.get(pkg.id)}
							/>
						))}
					</div>
				)}
			</section>
		</ExploreHubLayout>
	);
}

// ─── Page Shell ──────────────────────────────────────────────────────────────

function PackagesHub() {
	const { t } = useTranslation("common");
	const auth = useAuth();
	const router = useRouter();
	const searchParams = useSearchParams();

	const tabParam = searchParams.get("tab");
	const tab = tabParam === "installed" ? "installed" : "explore";

	useEffect(() => {
		if (tabParam === "projects") {
			router.replace("/developer");
		}
	}, [router, tabParam]);

	const setTab = useCallback(
		(value: string) => {
			const params = new URLSearchParams(searchParams.toString());
			if (value === "explore") {
				params.delete("tab");
			} else {
				params.set("tab", value);
			}
			const qs = params.toString();
			router.push(qs ? `/store/packages?${qs}` : "/store/packages");
		},
		[router, searchParams],
	);

	if (tabParam === "projects") {
		return <Skeleton className="h-full w-full" />;
	}

	const navigation = (
		<TabsList
			aria-label={t("packageViews", "Package views")}
			className="h-10 rounded-xl border border-border/60 bg-muted/30 p-1"
		>
			<TabsTrigger value="explore" className="gap-1 rounded-lg px-2 sm:px-3">
				<Search aria-hidden="true" className="hidden h-3.5 w-3.5 sm:block" />
				{t("explore", "Explore")}
			</TabsTrigger>
			<TabsTrigger value="installed" className="gap-1 rounded-lg px-2 sm:px-3">
				<Download aria-hidden="true" className="hidden h-3.5 w-3.5 sm:block" />
				{t("installed", "Installed")}
			</TabsTrigger>
		</TabsList>
	);

	return (
		<Tabs
			value={tab}
			onValueChange={setTab}
			className="min-h-0 w-full flex-1 gap-0"
		>
			<TabsContent
				value="explore"
				className="m-0 flex min-h-0 flex-1 flex-col data-[state=inactive]:hidden"
			>
				<PackageListContent
					fetcher={fetcher}
					auth={auth}
					navigation={navigation}
				/>
			</TabsContent>
			<TabsContent
				value="installed"
				className="m-0 flex min-h-0 flex-1 flex-col data-[state=inactive]:hidden"
			>
				<InstalledContent navigation={navigation} />
			</TabsContent>
		</Tabs>
	);
}

function PageContent() {
	const searchParams = useSearchParams();
	const packageId = searchParams.get("id");
	const auth = useAuth();
	const statusMap = usePackageStatusMap();
	const getPackageStatus = useCallback(
		(packageId: string) => statusMap.get(packageId),
		[statusMap],
	);

	if (packageId) {
		return (
			<PackageDetailWrapper
				fetcher={fetcher}
				auth={auth}
				getPackageStatus={getPackageStatus}
			/>
		);
	}

	return <PackagesHub />;
}

export default function Page() {
	return (
		<Suspense fallback={<Skeleton className="h-full w-full" />}>
			<PageContent />
		</Suspense>
	);
}

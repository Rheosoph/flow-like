"use client";

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
	Badge,
	Button,
	Input,
	PackageStatusBadge,
	Skeleton,
	StorePackageDetail,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
	useBackend,
} from "@tm9657/flow-like-ui";
import {
	type InstalledPackage,
	PackageStatus,
	type PackageSummary,
	type PackageUpdate,
} from "@tm9657/flow-like-ui/lib/schema/wasm";
import {
	AlertTriangle,
	Download,
	Eye,
	EyeOff,
	FolderOpen,
	Loader2,
	Package,
	RefreshCw,
	Search,
	Shield,
	Trash2,
	Upload,
	X,
} from "lucide-react";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { usePackageStatusMap } from "../../../../hooks/use-package-status";
import { fetcher } from "../../../../lib/api";

function PackageItem({
	pkg,
	isLocal,
	hasUpdate,
	latestVersion,
	onUninstall,
	onUpdate,
	onSelect,
	isLoading,
	compileStatus,
}: {
	pkg: InstalledPackage;
	isLocal: boolean;
	hasUpdate?: boolean;
	latestVersion?: string;
	onUninstall: (id: string) => void;
	onUpdate: (id: string) => void;
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
	return (
		<div
			className={`rounded-xl border p-4 transition-all cursor-pointer ${
				isLocal
					? "border-dashed border-primary/20 bg-card/50 hover:bg-muted/10"
					: "border-border/20 bg-card/50 hover:bg-muted/10"
			}`}
			onClick={() => onSelect?.(pkg.id)}
			onKeyDown={(e) => e.key === "Enter" && onSelect?.(pkg.id)}
			role="button"
			tabIndex={0}
		>
			<div className="flex items-start justify-between gap-3">
				<div className="min-w-0 flex-1 space-y-1.5">
					<div className="flex items-center gap-2 flex-wrap">
						<span className="text-sm font-medium truncate">
							{pkg.manifest.name}
						</span>
						<Badge
							variant="outline"
							className="text-[10px] px-1.5 py-0 h-5 rounded-full font-normal"
						>
							v{pkg.version}
						</Badge>
						{isLocal && (
							<Badge
								variant="secondary"
								className="text-[10px] px-1.5 py-0 h-5 rounded-full font-normal gap-1"
							>
								<FolderOpen className="h-2.5 w-2.5" />
								Local
							</Badge>
						)}
						{hasUpdate && (
							<Badge className="text-[10px] px-1.5 py-0 h-5 rounded-full font-normal gap-1 bg-amber-500/10 text-amber-600 border-amber-500/20 hover:bg-amber-500/20">
								Update
							</Badge>
						)}
						{compileStatus && compileStatus !== "idle" && (
							<PackageStatusBadge status={compileStatus} />
						)}
					</div>
					{pkg.manifest.description && (
						<p className="text-sm text-muted-foreground/70 line-clamp-2">
							{pkg.manifest.description}
						</p>
					)}
					{pkg.manifest.keywords.length > 0 && (
						<div className="flex flex-wrap gap-1 pt-0.5">
							{pkg.manifest.keywords.slice(0, 4).map((keyword) => (
								<span
									key={keyword}
									className="text-[10px] px-1.5 py-0.5 rounded-full bg-muted/40 text-muted-foreground/60"
								>
									{keyword}
								</span>
							))}
						</div>
					)}
				</div>
				<div className="flex items-center gap-1 shrink-0">
					{hasUpdate && latestVersion && (
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
									onClick={(e) => {
										e.stopPropagation();
										onUpdate(pkg.id);
									}}
									disabled={isLoading}
								>
									{isLoading ? (
										<Loader2 className="h-3.5 w-3.5 animate-spin" />
									) : (
										<RefreshCw className="h-3.5 w-3.5" />
									)}
								</Button>
							</TooltipTrigger>
							<TooltipContent>Update to v{latestVersion}</TooltipContent>
						</Tooltip>
					)}
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-destructive/80 hover:bg-destructive/10"
								onClick={(e) => {
									e.stopPropagation();
									onUninstall(pkg.id);
								}}
								disabled={isLoading}
							>
								{isLoading && !hasUpdate ? (
									<Loader2 className="h-3.5 w-3.5 animate-spin" />
								) : (
									<Trash2 className="h-3.5 w-3.5" />
								)}
							</Button>
						</TooltipTrigger>
						<TooltipContent>{isLocal ? "Remove" : "Uninstall"}</TooltipContent>
					</Tooltip>
				</div>
			</div>
		</div>
	);
}

function ItemSkeleton() {
	return (
		<div className="rounded-xl border border-border/20 bg-card/50 p-4 space-y-2">
			<div className="flex items-center gap-2">
				<Skeleton className="h-4 w-28" />
				<Skeleton className="h-5 w-12 rounded-full" />
			</div>
			<Skeleton className="h-3.5 w-full" />
			<Skeleton className="h-3.5 w-2/3" />
			<div className="flex gap-1 pt-0.5">
				<Skeleton className="h-4 w-10 rounded-full" />
				<Skeleton className="h-4 w-14 rounded-full" />
			</div>
		</div>
	);
}

function OwnedRegistryItem({
	pkg,
	onInstall,
	onSelect,
	isLoading,
	compileStatus,
}: {
	pkg: PackageSummary;
	onInstall: (id: string) => void;
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
	return (
		<div
			className={`rounded-xl border border-border/20 bg-card/50 hover:bg-muted/10 p-4 transition-all cursor-pointer ${pkg.status === PackageStatus.Disabled ? "opacity-60" : ""}`}
			onClick={() => onSelect?.(pkg.id)}
			onKeyDown={(e) => e.key === "Enter" && onSelect?.(pkg.id)}
			role="button"
			tabIndex={0}
		>
			<div className="flex items-start justify-between gap-3">
				<div className="min-w-0 flex-1 space-y-1.5">
					<div className="flex items-center gap-2 flex-wrap">
						<span className="text-sm font-medium truncate">{pkg.name}</span>
						<Badge
							variant="outline"
							className="text-[10px] px-1.5 py-0 h-5 rounded-full font-normal"
						>
							v{pkg.latestVersion}
						</Badge>
						{pkg.verified && (
							<Shield className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
						)}
						{pkg.status === PackageStatus.Disabled && (
							<Badge
								variant="destructive"
								className="text-[10px] px-1.5 py-0 h-5 rounded-full font-normal"
							>
								Disabled
							</Badge>
						)}
						{pkg.visibility && pkg.visibility !== "public" && (
							<Badge
								variant="secondary"
								className="text-[10px] px-1.5 py-0 h-5 rounded-full font-normal"
							>
								{pkg.visibility}
							</Badge>
						)}
						{compileStatus && compileStatus !== "idle" && (
							<PackageStatusBadge status={compileStatus} />
						)}
					</div>
					{pkg.description && (
						<p className="text-sm text-muted-foreground/70 line-clamp-2">
							{pkg.description}
						</p>
					)}
					{pkg.keywords.length > 0 && (
						<div className="flex flex-wrap gap-1 pt-0.5">
							{pkg.keywords.slice(0, 4).map((keyword) => (
								<span
									key={keyword}
									className="text-[10px] px-1.5 py-0.5 rounded-full bg-muted/40 text-muted-foreground/60"
								>
									{keyword}
								</span>
							))}
						</div>
					)}
				</div>
				<div className="flex items-center gap-1 shrink-0">
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="sm"
								className="h-8 gap-1.5 rounded-full text-xs text-muted-foreground/70 hover:text-foreground/80 hover:bg-muted/30 px-3"
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
										Install
									</>
								)}
							</Button>
						</TooltipTrigger>
						<TooltipContent>Install v{pkg.latestVersion}</TooltipContent>
					</Tooltip>
				</div>
			</div>
		</div>
	);
}

export default function InstalledPackagesPage() {
	const backend = useBackend();
	const auth = useAuth();
	const [isInitialized, setIsInitialized] = useState(false);
	const [isInitializing, setIsInitializing] = useState(false);
	const [searchQuery, setSearchQuery] = useState("");
	const [showDisabled, setShowDisabled] = useState(false);
	const [selectedPackageId, setSelectedPackageId] = useState<string | null>(
		null,
	);

	const [installedPackages, setInstalledPackages] = useState<
		InstalledPackage[]
	>([]);
	const [localPackages, setLocalPackages] = useState<InstalledPackage[]>([]);
	const [ownedPackages, setOwnedPackages] = useState<PackageSummary[]>([]);
	const [updates, setUpdates] = useState<PackageUpdate[]>([]);
	const [isLoading, setIsLoading] = useState(false);
	const [loadingPackage, setLoadingPackage] = useState<string | null>(null);
	const [isLoadingLocal, setIsLoadingLocal] = useState(false);
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

	const fetchInstalled = useCallback(async () => {
		if (!backend?.registryState || !isInitialized) return;
		setIsLoading(true);
		try {
			const [packages, updateList, ownedResults] = await Promise.all([
				backend.registryState.getInstalledPackages(),
				backend.registryState.checkForUpdates(),
				backend.registryState.getOwnedPackages({
					includeDisabled: showDisabled,
				}),
			]);
			const registry = packages.filter((p) => !p.id.startsWith("local."));
			const local = packages.filter((p) => p.id.startsWith("local."));
			setInstalledPackages(registry);
			setLocalPackages(local);
			setUpdates(updateList);

			const installedIds = new Set(packages.map((p) => p.id));
			setOwnedPackages(
				ownedResults.packages.filter((p) => !installedIds.has(p.id)),
			);
		} catch (err) {
			console.error("Failed to fetch installed packages:", err);
		} finally {
			setIsLoading(false);
		}
	}, [backend?.registryState, isInitialized, showDisabled]);

	useEffect(() => {
		if (isInitialized) {
			fetchInstalled();
		}
	}, [isInitialized, fetchInstalled]);

	const handleLoadLocal = async () => {
		try {
			const selected = await open({
				multiple: false,
				filters: [{ name: "WASM Files", extensions: ["wasm"] }],
			});

			if (!selected) return;

			setIsLoadingLocal(true);
			await invoke("registry_load_local", { path: selected });

			const baseName =
				selected.split(/[/\\]/).pop()?.replace(".wasm", "") ?? "package";
			toast.success(`Loaded ${baseName}`, {
				description: "Package loaded for development testing",
			});
			await fetchInstalled();
		} catch (err) {
			console.error("Failed to load local package:", err);
			toast.error(`Failed to load package: ${err}`);
		} finally {
			setIsLoadingLocal(false);
		}
	};

	const handleUninstall = async (packageId: string) => {
		if (!backend?.registryState) return;
		setLoadingPackage(packageId);
		try {
			await backend.registryState.uninstallPackage(packageId);
			toast.success("Package uninstalled");
			await fetchInstalled();
		} catch (err) {
			console.error("Failed to uninstall package:", err);
			toast.error(`Failed to uninstall: ${err}`);
		} finally {
			setLoadingPackage(null);
		}
	};

	const handleUpdate = async (packageId: string) => {
		if (!backend?.registryState) return;
		setLoadingPackage(packageId);
		try {
			await backend.registryState.updatePackage(packageId);
			toast.success("Package updated");
			await fetchInstalled();
		} catch (err) {
			console.error("Failed to update package:", err);
			toast.error(`Failed to update: ${err}`);
		} finally {
			setLoadingPackage(null);
		}
	};

	const handleInstall = async (packageId: string) => {
		if (!backend?.registryState) return;
		setLoadingPackage(packageId);
		try {
			await backend.registryState.installPackage(packageId);
			toast.success("Package installed");
			await fetchInstalled();
		} catch (err) {
			console.error("Failed to install package:", err);
			toast.error(`Failed to install: ${err}`);
		} finally {
			setLoadingPackage(null);
		}
	};

	const updateMap = new Map(updates.map((u) => [u.packageId, u.latestVersion]));

	const filteredPackages = installedPackages.filter((pkg) => {
		if (!searchQuery) return true;
		const query = searchQuery.toLowerCase();
		return (
			pkg.manifest.name.toLowerCase().includes(query) ||
			pkg.manifest.description.toLowerCase().includes(query)
		);
	});

	const filteredLocalPackages = localPackages.filter((pkg) => {
		if (!searchQuery) return true;
		const query = searchQuery.toLowerCase();
		return (
			pkg.manifest.name.toLowerCase().includes(query) ||
			pkg.manifest.description.toLowerCase().includes(query)
		);
	});

	const filteredOwnedPackages = ownedPackages.filter((pkg) => {
		if (!searchQuery) return true;
		const query = searchQuery.toLowerCase();
		return (
			pkg.name.toLowerCase().includes(query) ||
			pkg.description.toLowerCase().includes(query)
		);
	});

	if (isInitializing) {
		return (
			<div className="flex items-center justify-center h-full">
				<div className="text-center space-y-3">
					<Loader2 className="h-5 w-5 animate-spin mx-auto text-muted-foreground/40" />
					<p className="text-sm text-muted-foreground/60">
						Initializing registry…
					</p>
				</div>
			</div>
		);
	}

	const totalCount =
		filteredPackages.length +
		filteredLocalPackages.length +
		filteredOwnedPackages.length;

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
					toast.error(`Failed to install: ${error.message}`)
				}
				onUninstallError={(error) =>
					toast.error(`Failed to uninstall: ${error.message}`)
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
		<div className="flex flex-col h-full">
			{/* Toolbar */}
			<div className="flex items-center gap-2 pb-4">
				<div className="relative flex-1 max-w-lg">
					<Search className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground/40 pointer-events-none" />
					<Input
						placeholder="Search installed…"
						value={searchQuery}
						onChange={(e) => setSearchQuery(e.target.value)}
						className="pl-11 h-10 rounded-full bg-muted/30 border-transparent focus:border-border/40 focus:bg-muted/50 transition-all text-sm"
					/>
					{searchQuery && (
						<button
							type="button"
							onClick={() => setSearchQuery("")}
							className="absolute right-4 top-1/2 -translate-y-1/2 text-muted-foreground/40 hover:text-foreground transition-colors"
						>
							<X className="h-4 w-4" />
						</button>
					)}
				</div>

				<div className="flex items-center gap-1">
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant={showDisabled ? "secondary" : "ghost"}
								size="icon"
								className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
								onClick={() => setShowDisabled((v) => !v)}
							>
								{showDisabled ? (
									<Eye className="h-4 w-4" />
								) : (
									<EyeOff className="h-4 w-4" />
								)}
							</Button>
						</TooltipTrigger>
						<TooltipContent>
							{showDisabled ? "Hide disabled" : "Show disabled"}
						</TooltipContent>
					</Tooltip>

					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
								onClick={handleLoadLocal}
								disabled={isLoadingLocal}
							>
								{isLoadingLocal ? (
									<Loader2 className="h-4 w-4 animate-spin" />
								) : (
									<Upload className="h-4 w-4" />
								)}
							</Button>
						</TooltipTrigger>
						<TooltipContent>Load local .wasm</TooltipContent>
					</Tooltip>

					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
								onClick={fetchInstalled}
								disabled={isLoading}
							>
								<RefreshCw
									className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`}
								/>
							</Button>
						</TooltipTrigger>
						<TooltipContent>Refresh</TooltipContent>
					</Tooltip>
				</div>
			</div>

			{/* Update banner */}
			{updates.length > 0 && (
				<div className="rounded-xl bg-amber-500/5 border border-amber-500/15 p-3 mb-4 flex items-center gap-2">
					<AlertTriangle className="h-3.5 w-3.5 text-amber-500/70 shrink-0" />
					<span className="text-sm text-muted-foreground/80">
						{updates.length} update{updates.length > 1 ? "s" : ""} available
					</span>
				</div>
			)}

			{/* Content */}
			<div className="flex-1 overflow-y-auto space-y-6 min-h-0">
				{isLoading ? (
					<div className="space-y-6">
						<div className="space-y-3">
							<Skeleton className="h-3 w-16" />
							<div className="grid grid-cols-[repeat(auto-fill,minmax(320px,1fr))] gap-3">
								{Array.from({ length: 4 }).map((_, i) => (
									<ItemSkeleton key={i} />
								))}
							</div>
						</div>
					</div>
				) : totalCount === 0 ? (
					<div className="flex flex-col items-center justify-center py-20 text-center">
						<div className="rounded-2xl border border-dashed border-border/30 p-8 space-y-3 max-w-sm">
							<Package className="h-8 w-8 text-muted-foreground/30 mx-auto" />
							<p className="text-sm font-medium text-muted-foreground/60">
								{searchQuery ? "No matching packages" : "No packages installed"}
							</p>
							<p className="text-xs text-muted-foreground/40">
								{searchQuery
									? "Try a different search term"
									: "Browse the registry to install custom nodes, or load a local .wasm file"}
							</p>
							{!searchQuery && (
								<div className="flex items-center justify-center gap-2 pt-2">
									<Link href="/settings/registry/explore">
										<Button
											variant="ghost"
											size="sm"
											className="h-8 rounded-full text-xs text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
										>
											Browse Registry
										</Button>
									</Link>
									<Button
										variant="ghost"
										size="sm"
										className="h-8 rounded-full text-xs text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
										onClick={handleLoadLocal}
									>
										<Upload className="h-3 w-3 mr-1.5" />
										Load Local
									</Button>
								</div>
							)}
						</div>
					</div>
				) : (
					<>
						{/* Local Development */}
						{filteredLocalPackages.length > 0 && (
							<div className="space-y-3">
								<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
									Local Development
								</span>
								<div className="grid grid-cols-[repeat(auto-fill,minmax(320px,1fr))] gap-3">
									{filteredLocalPackages.map((pkg) => (
										<PackageItem
											key={pkg.id}
											pkg={pkg}
											isLocal
											onUninstall={handleUninstall}
											onUpdate={handleUpdate}
											onSelect={setSelectedPackageId}
											isLoading={loadingPackage === pkg.id}
											compileStatus={packageStatusMap.get(pkg.id)}
										/>
									))}
								</div>
							</div>
						)}

						{/* Registry */}
						{filteredPackages.length > 0 && (
							<div className="space-y-3">
								<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
									Installed
								</span>
								<div className="grid grid-cols-[repeat(auto-fill,minmax(320px,1fr))] gap-3">
									{filteredPackages.map((pkg) => (
										<PackageItem
											key={pkg.id}
											pkg={pkg}
											isLocal={false}
											hasUpdate={updateMap.has(pkg.id)}
											latestVersion={updateMap.get(pkg.id)}
											onUninstall={handleUninstall}
											onUpdate={handleUpdate}
											onSelect={setSelectedPackageId}
											isLoading={loadingPackage === pkg.id}
											compileStatus={packageStatusMap.get(pkg.id)}
										/>
									))}
								</div>
							</div>
						)}

						{/* Owned but not installed */}
						{filteredOwnedPackages.length > 0 && (
							<div className="space-y-3">
								<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
									Not Installed
								</span>
								<div className="grid grid-cols-[repeat(auto-fill,minmax(320px,1fr))] gap-3">
									{filteredOwnedPackages.map((pkg) => (
										<OwnedRegistryItem
											key={pkg.id}
											pkg={pkg}
											onInstall={handleInstall}
											onSelect={setSelectedPackageId}
											isLoading={loadingPackage === pkg.id}
											compileStatus={packageStatusMap.get(pkg.id)}
										/>
									))}
								</div>
							</div>
						)}
					</>
				)}
			</div>
		</div>
	);
}

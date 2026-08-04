"use client";

import {
	Badge,
	Button,
	EmptyState,
	ExploreHubHeader,
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
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
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
			className="group relative flex flex-row rounded-lg border border-border/40 border-dashed bg-card/60 backdrop-blur-sm overflow-hidden transition-all hover:border-primary/40 hover:bg-card/80 hover:shadow-lg cursor-pointer"
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
				<div className="flex items-center gap-1.5 min-w-0">
					<h3 className="text-sm font-semibold font-mono truncate group-hover:text-primary transition-colors">
						{displayName}
					</h3>
					<span className="text-[10px] font-mono text-muted-foreground/50 shrink-0">
						v{pkg.version}
						{updateAvailable && (
							<span className="text-primary"> → v{updateAvailable}</span>
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
								Update
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

function InstalledContent() {
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

	return (
		<div className="space-y-4">
			<div className="flex items-center gap-3">
				<div className="relative flex-1">
					<Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground/40" />
					<Input
						placeholder="Search installed packages…"
						value={searchQuery}
						onChange={(e) => setSearchQuery(e.target.value)}
						className="pl-10 rounded-full bg-muted/30 border-border/20"
					/>
				</div>
				{hasUpdates && (
					<Button
						size="sm"
						onClick={() => updateAllMutation.mutate()}
						disabled={updateAllMutation.isPending}
						className="gap-1.5 rounded-full"
					>
						{updateAllMutation.isPending ? (
							<RefreshCw className="h-3.5 w-3.5 animate-spin" />
						) : (
							<RefreshCw className="h-3.5 w-3.5" />
						)}
						Update All ({availableUpdates.data?.length})
					</Button>
				)}
			</div>

			{installedPackages.data && (
				<div className="flex gap-4 text-xs text-muted-foreground/60">
					<span className="flex items-center gap-1">
						<CheckCircle className="h-3.5 w-3.5 text-green-500" />
						{registryPackages.length} installed
					</span>
					{hasUpdates && (
						<span className="flex items-center gap-1">
							<AlertCircle className="h-3.5 w-3.5 text-yellow-500" />
							{availableUpdates.data?.length} updates
						</span>
					)}
				</div>
			)}

			{registryReady.isLoading ||
			installedPackages.isLoading ||
			(!registryReady.isSuccess && !registryReady.isError) ? (
				<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
					{INSTALLED_PACKAGE_SKELETON_KEYS.map((key) => (
						<div
							key={key}
							className="flex flex-col rounded-lg border border-border/40 border-dashed bg-card/60 overflow-hidden"
						>
							<Skeleton className="h-20 w-full rounded-none" />
							<div className="px-3.5 pt-5 pb-3 space-y-2">
								<Skeleton className="h-4 w-28 rounded" />
								<Skeleton className="h-3 w-full rounded" />
								<Skeleton className="h-3 w-3/4 rounded" />
							</div>
						</div>
					))}
				</div>
			) : filteredPackages.length === 0 ? (
				<EmptyState
					icons={[Download, Package, Sparkles]}
					title={
						searchQuery
							? "No matching packages"
							: "No registry packages installed"
					}
					description={
						searchQuery
							? "Try a different search term"
							: "Browse the Explore tab to find and install packages"
					}
					className="border border-dashed border-border/30 rounded-2xl bg-muted/5"
				/>
			) : (
				<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
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
		</div>
	);
}

// ─── Page Shell ──────────────────────────────────────────────────────────────

function PackagesHub() {
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

	return (
		<main className="flex-col flex grow max-h-full p-6 overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-7xl space-y-6">
				<ExploreHubHeader
					active="packages"
					subtitle="Discover, install, and manage WASM node packages."
				/>

				<Tabs value={tab} onValueChange={setTab}>
					<TabsList>
						<TabsTrigger value="explore">
							<Search className="h-3.5 w-3.5 mr-1.5" />
							Explore
						</TabsTrigger>
						<TabsTrigger value="installed">
							<Download className="h-3.5 w-3.5 mr-1.5" />
							Installed
						</TabsTrigger>
					</TabsList>

					<TabsContent value="explore" className="mt-6">
						<PackageListContent fetcher={fetcher} auth={auth} />
					</TabsContent>

					<TabsContent value="installed" className="mt-6">
						<InstalledContent />
					</TabsContent>
				</Tabs>
			</div>
		</main>
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

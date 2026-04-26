"use client";

import { invoke } from "@tauri-apps/api/core";
import {
	type InstalledPackage,
	PackageStatusBadge,
	type PackageUpdate,
	useMutation,
	useQuery,
	useQueryClient,
} from "@tm9657/flow-like-ui";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardFooter,
	CardHeader,
	CardTitle,
	Input,
	Skeleton,
} from "@tm9657/flow-like-ui/components";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
} from "@tm9657/flow-like-ui/components/ui/avatar";
import { getErrorMessage } from "@tm9657/flow-like-ui/lib/error-message";
import { formatDistanceToNow } from "date-fns";
import {
	AlertCircle,
	CheckCircle,
	Download,
	ExternalLink,
	Package,
	RefreshCw,
	Search,
	Trash2,
} from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { usePackageStatusMap } from "../../../hooks/use-package-status";

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
	const displayName = pkg.metadata?.name ?? pkg.manifest.name;
	const displayDesc = pkg.metadata?.description ?? pkg.manifest.description;
	const icon = pkg.metadata?.icon;

	return (
		<Card className="flex flex-col h-full">
			<CardHeader className="pb-2">
				<div className="flex items-start justify-between">
					<div className="flex items-center gap-2">
						<Avatar className="h-5 w-5 rounded">
							{icon ? (
								<AvatarImage src={icon} alt={displayName} className="rounded" />
							) : null}
							<AvatarFallback className="rounded">
								<Package className="h-3.5 w-3.5 text-muted-foreground" />
							</AvatarFallback>
						</Avatar>
						<CardTitle className="text-base">{displayName}</CardTitle>
					</div>
					<div className="flex items-center gap-1.5">
						{compileStatus && compileStatus !== "idle" && (
							<PackageStatusBadge status={compileStatus} />
						)}
						{updateAvailable && (
							<Badge variant="secondary" className="gap-1">
								<AlertCircle className="h-3 w-3" />
								Update
							</Badge>
						)}
					</div>
				</div>
				<CardDescription className="line-clamp-2 text-sm">
					{displayDesc}
				</CardDescription>
			</CardHeader>
			<CardContent className="flex-1 pb-2">
				<div className="flex flex-wrap gap-1 mb-3">
					{pkg.manifest.keywords.slice(0, 3).map((keyword) => (
						<Badge key={keyword} variant="outline" className="text-xs">
							{keyword}
						</Badge>
					))}
				</div>
				<div className="space-y-1 text-xs text-muted-foreground">
					<div className="flex items-center gap-2">
						<span>v{pkg.version}</span>
						{updateAvailable && (
							<>
								<span>→</span>
								<span className="text-primary">v{updateAvailable}</span>
							</>
						)}
					</div>
					<div className="flex items-center gap-1">
						<span>
							Installed{" "}
							{formatDistanceToNow(new Date(pkg.installedAt), {
								addSuffix: true,
							})}
						</span>
					</div>
				</div>
			</CardContent>
			<CardFooter className="pt-2 gap-2 flex-wrap">
				<Link href={`/store/packages?id=${pkg.id}`} className="flex-1">
					<Button variant="outline" className="w-full" size="sm">
						Details
						<ExternalLink className="ml-2 h-3 w-3" />
					</Button>
				</Link>
				{updateAvailable && (
					<Button
						size="sm"
						onClick={onUpdate}
						disabled={isUpdating}
						className="gap-1"
					>
						{isUpdating ? (
							<RefreshCw className="h-3 w-3 animate-spin" />
						) : (
							<RefreshCw className="h-3 w-3" />
						)}
						Update
					</Button>
				)}
				<Button
					variant="destructive"
					size="sm"
					onClick={onUninstall}
					disabled={isUninstalling}
					className="gap-1"
				>
					{isUninstalling ? (
						<RefreshCw className="h-3 w-3 animate-spin" />
					) : (
						<Trash2 className="h-3 w-3" />
					)}
				</Button>
			</CardFooter>
		</Card>
	);
}

function PackageCardSkeleton() {
	return (
		<Card className="flex flex-col h-full">
			<CardHeader className="pb-2">
				<Skeleton className="h-5 w-32" />
				<Skeleton className="h-8 w-full mt-2" />
			</CardHeader>
			<CardContent className="flex-1 pb-2">
				<div className="flex gap-1 mb-3">
					<Skeleton className="h-5 w-12" />
					<Skeleton className="h-5 w-16" />
				</div>
				<Skeleton className="h-4 w-24" />
			</CardContent>
			<CardFooter className="pt-2 gap-2">
				<Skeleton className="h-8 w-20" />
				<Skeleton className="h-8 w-8" />
			</CardFooter>
		</Card>
	);
}

export default function InstalledPackagesPage() {
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
		onSuccess: (
			_: void,
			{ packageId }: { packageId: string; version?: string },
		) => {
			toast.success("Package updated successfully");
			queryClient.invalidateQueries({ queryKey: ["installed-packages"] });
			queryClient.invalidateQueries({ queryKey: ["available-updates"] });
			queryClient.invalidateQueries({
				queryKey: ["installed-package", packageId],
			});
		},
		onError: (
			error: unknown,
			{ packageId }: { packageId: string; version?: string },
		) => {
			toast.error(`Failed to update package: ${getErrorMessage(error)}`);
		},
		onSettled: (
			_: void | undefined,
			__: unknown,
			{ packageId }: { packageId: string; version?: string },
		) => {
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
		onSuccess: (_: void, packageId: string) => {
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
		onSettled: (_: void | undefined, __: unknown, packageId: string) => {
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

	const filteredPackages = (installedPackages.data ?? []).filter((pkg) => {
		if (!searchQuery) return true;
		const query = searchQuery.toLowerCase();
		return (
			pkg.manifest.name.toLowerCase().includes(query) ||
			pkg.manifest.description.toLowerCase().includes(query) ||
			pkg.manifest.keywords.some((kw) => kw.toLowerCase().includes(query))
		);
	});

	const hasUpdates = (availableUpdates.data?.length ?? 0) > 0;

	return (
		<main className="flex-col flex grow max-h-full p-6 overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-7xl space-y-6">
				{/* Header */}
				<div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
					<div className="space-y-1">
						<h1 className="text-3xl font-bold tracking-tight flex items-center gap-2">
							<Package className="h-8 w-8" />
							Installed Packages
						</h1>
						<p className="text-muted-foreground">
							Manage your installed WASM node packages
						</p>
					</div>
					<div className="flex items-center gap-2">
						{hasUpdates && (
							<Button
								onClick={() => updateAllMutation.mutate()}
								disabled={updateAllMutation.isPending}
								className="gap-2"
							>
								{updateAllMutation.isPending ? (
									<RefreshCw className="h-4 w-4 animate-spin" />
								) : (
									<RefreshCw className="h-4 w-4" />
								)}
								Update All ({availableUpdates.data?.length})
							</Button>
						)}
						<Link href="/store/packages">
							<Button variant="outline" className="gap-2">
								<Download className="h-4 w-4" />
								Browse Packages
							</Button>
						</Link>
					</div>
				</div>

				{/* Search */}
				<div className="relative max-w-md">
					<Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
					<Input
						placeholder="Search installed packages..."
						value={searchQuery}
						onChange={(e) => setSearchQuery(e.target.value)}
						className="pl-10"
					/>
				</div>

				{/* Stats */}
				{installedPackages.data && (
					<div className="flex gap-4 text-sm text-muted-foreground">
						<span className="flex items-center gap-1">
							<CheckCircle className="h-4 w-4 text-green-500" />
							{installedPackages.data.length} installed
						</span>
						{hasUpdates && (
							<span className="flex items-center gap-1">
								<AlertCircle className="h-4 w-4 text-yellow-500" />
								{availableUpdates.data?.length} updates available
							</span>
						)}
					</div>
				)}

				{/* Package Grid */}
				{installedPackages.isLoading ? (
					<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4">
						{Array.from({ length: 4 }).map((_, i) => (
							<PackageCardSkeleton key={i} />
						))}
					</div>
				) : filteredPackages.length === 0 ? (
					<Card className="p-12 text-center">
						<Package className="mx-auto h-12 w-12 text-muted-foreground mb-4" />
						<h3 className="text-lg font-semibold">
							{searchQuery ? "No matching packages" : "No packages installed"}
						</h3>
						<p className="text-muted-foreground mt-1 mb-4">
							{searchQuery
								? "Try a different search term"
								: "Browse the registry to find and install packages"}
						</p>
						{!searchQuery && (
							<Link href="/store/packages">
								<Button>
									<Download className="mr-2 h-4 w-4" />
									Browse Packages
								</Button>
							</Link>
						)}
					</Card>
				) : (
					<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4">
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
		</main>
	);
}

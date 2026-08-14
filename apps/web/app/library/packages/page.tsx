"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type InstalledPackage,
	useBackend,
	useMutation,
	useQuery,
	useQueryClient,
	useSearch,
} from "@flow-like/flow-like-ui";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
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
} from "@flow-like/flow-like-ui";
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
import { toast } from "sonner";

function InstalledPackageCard({
	pkg,
	updateAvailable,
	onUninstall,
	onUpdate,
	isUpdating,
	isUninstalling,
}: {
	pkg: InstalledPackage;
	updateAvailable?: string;
	onUninstall: () => void;
	onUpdate: () => void;
	isUpdating: boolean;
	isUninstalling: boolean;
}) {
	const { t } = useTranslation("common");
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
					{updateAvailable && (
						<Badge variant="secondary" className="gap-1">
							<AlertCircle className="h-3 w-3" />
							{t('update', 'Update')}
						</Badge>
					)}
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
						<span>{`v${pkg.version}`}</span>
						{updateAvailable && (
							<>
								<span>→</span>
								<span className="text-primary">{`v${updateAvailable}`}</span>
							</>
						)}
					</div>
					<div className="flex items-center gap-1">
						<span>
							{t('installed', 'Installed')}{" "}
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
						{t('details', 'Details')}
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
						{t('update', 'Update')}
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
	const { t } = useTranslation("common");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [searchQuery, setSearchQuery] = useState("");
	const [updatingPackages, setUpdatingPackages] = useState<Set<string>>(
		new Set(),
	);
	const [uninstallingPackages, setUninstallingPackages] = useState<Set<string>>(
		new Set(),
	);

	const installedPackages = useQuery({
		queryKey: ["installed-packages"],
		queryFn: async () => {
			return backend.registryState.getInstalledPackages();
		},
	});

	const availableUpdates = useQuery({
		queryKey: ["available-updates"],
		queryFn: async () => {
			return backend.registryState.checkForUpdates();
		},
	});

	const updateMutation = useMutation({
		mutationFn: async ({
			packageId,
			version,
		}: { packageId: string; version?: string }) => {
			setUpdatingPackages((prev) => new Set(prev).add(packageId));
			await backend.registryState.updatePackage(packageId, version);
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
			error: Error,
			{ packageId }: { packageId: string; version?: string },
		) => {
			toast.error(`Failed to update package: ${error.message}`);
		},
		onSettled: (
			_: void | undefined,
			__: Error | null,
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
			await backend.registryState.uninstallPackage(packageId);
		},
		onSuccess: (_: void, packageId: string) => {
			toast.success("Package uninstalled");
			queryClient.invalidateQueries({ queryKey: ["installed-packages"] });
			queryClient.invalidateQueries({ queryKey: ["available-updates"] });
			queryClient.invalidateQueries({
				queryKey: ["installed-package", packageId],
			});
		},
		onError: (error: Error) => {
			toast.error(`Failed to uninstall package: ${error.message}`);
		},
		onSettled: (_: void | undefined, __: Error | null, packageId: string) => {
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
				await backend.registryState.updatePackage(
					update.packageId,
					update.latestVersion,
				);
			}
		},
		onSuccess: () => {
			toast.success("All packages updated");
			queryClient.invalidateQueries({ queryKey: ["installed-packages"] });
			queryClient.invalidateQueries({ queryKey: ["available-updates"] });
		},
		onError: (error: Error) => {
			toast.error(`Failed to update packages: ${error.message}`);
		},
	});

	const updatesMap = new Map(
		(availableUpdates.data ?? []).map((u) => [u.packageId, u.latestVersion]),
	);

	const filteredPackages = useSearch(installedPackages.data, searchQuery, {
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
		<main className="flex-col flex flex-grow max-h-full p-6 overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-7xl space-y-6">
				{/* Header */}
				<div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
					<div className="space-y-1">
						<h1 className="text-3xl font-bold tracking-tight flex items-center gap-2">
							<Package className="h-8 w-8" />
							{t('installedPackages', 'Installed Packages')}
						</h1>
						<p className="text-muted-foreground">
							{t('manageYourInstalledWasmNodePackages', 'Manage your installed WASM node packages')}
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
								{t("updateAllLength", "Update All ({{length}})", {
									length: availableUpdates.data?.length ?? 0,
								})}
							</Button>
						)}
						<Link href="/store/packages">
							<Button variant="outline" className="gap-2">
								<Download className="h-4 w-4" />
								{t('browsePackages', 'Browse Packages')}
							</Button>
						</Link>
					</div>
				</div>

				{/* Search */}
				<div className="relative max-w-md">
					<Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
					<Input
						placeholder={t('searchInstalledPackages', 'Search installed packages...')}
						value={searchQuery}
						onChange={(e) => setSearchQuery(e.target.value)}
						className="pl-10"
					/>
				</div>

				{/* Stats */}
				{installedPackages.data && (
					<div className="flex gap-4 text-sm text-muted-foreground">
						<span className="flex items-center gap-1">
							<CheckCircle className="h-4 w-4 text-green-500" />{t('lengthInstalled', '{{length}} installed', { length: installedPackages.data.length })}</span>
						{hasUpdates && (
							<span className="flex items-center gap-1">
								<AlertCircle className="h-4 w-4 text-yellow-500" />
								{availableUpdates.data?.length} {t('updatesAvailable', 'updates available')}
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
							{searchQuery ? t('noMatchingPackages', 'No matching packages') : t('noPackagesInstalled', 'No packages installed')}
						</h3>
						<p className="text-muted-foreground mt-1 mb-4">
							{searchQuery
								? t('tryADifferentSearchTerm', 'Try a different search term')
								: t('browseTheRegistryToFindAndInstallPackages', 'Browse the registry to find and install packages')}
						</p>
						{!searchQuery && (
							<Link href="/store/packages">
								<Button>
									<Download className="mr-2 h-4 w-4" />
									{t('browsePackages', 'Browse Packages')}
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
							/>
						))}
					</div>
				)}
			</div>
		</main>
	);
}

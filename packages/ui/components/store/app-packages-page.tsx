"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ArrowUpCircle,
	ChevronDown,
	ChevronRight,
	Layers,
	Package,
	RefreshCw,
	Trash2,
	TriangleAlert,
	Zap,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import type { INode } from "../../lib/schema/flow/node";
import type {
	AddAppPackageRequest,
	AppPackage,
	InstalledPackage,
	PackageUpdate,
	UpdateAppPackageRequest,
} from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import { Alert, AlertDescription, AlertTitle } from "../ui/alert";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../ui/card";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../ui/collapsible";
import { EmptyState } from "../ui/empty-state";
import { Skeleton } from "../ui/skeleton";
import { Switch } from "../ui/switch";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../ui/tooltip";
import { PackageSearchDialog } from "./package-search-dialog";

export interface AppPackagesPageProps {
	appId: string;
}

function installedToAppPackage(
	pkg: InstalledPackage,
	pinnedVersion: string,
): AppPackage {
	return {
		id: pkg.id,
		appId: "",
		packageId: pkg.id,
		packageName: pkg.manifest.name,
		version: pinnedVersion,
		autoUpdate: false,
		addedAt: pkg.installedAt,
		stale: false,
	};
}

function groupNodesByCategory(nodes: INode[]): Map<string, INode[]> {
	const grouped = new Map<string, INode[]>();
	for (const node of nodes) {
		const category = node.category || "Uncategorized";
		const existing = grouped.get(category);
		if (existing) {
			existing.push(node);
		} else {
			grouped.set(category, [node]);
		}
	}
	return new Map([...grouped.entries()].sort(([a], [b]) => a.localeCompare(b)));
}

export function AppPackagesPage({ appId }: AppPackagesPageProps) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [searchOpen, setSearchOpen] = useState(false);

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const isOffline = useQuery<boolean>({
		queryKey: ["app-offline", appId],
		queryFn: () => backend.isOffline(appId),
		enabled: !!appId,
	});

	const packages = useQuery<AppPackage[]>({
		queryKey: ["app", appId, "packages"],
		queryFn: async () => {
			if (isOffline.data && backend.appState.listPackages) {
				const pkgMap = await backend.appState.listPackages(appId);
				const installed = await backend.registryState.getInstalledPackages();
				const installedMap = new Map(installed.map((p) => [p.id, p]));
				return Object.entries(pkgMap).map(([pkgId, version]) => {
					const local = installedMap.get(pkgId);
					if (local) return installedToAppPackage(local, version);
					return {
						id: pkgId,
						appId,
						packageId: pkgId,
						packageName: pkgId,
						version,
						autoUpdate: false,
						addedAt: new Date().toISOString(),
						stale: false,
					} satisfies AppPackage;
				});
			}
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<AppPackage[]>(
				profile.data,
				`apps/${appId}/packages`,
			);
		},
		enabled:
			!!appId &&
			isOffline.data !== undefined &&
			(isOffline.data || !!profile.data),
	});

	const catalog = useQuery<INode[]>({
		queryKey: ["app-catalog-nodes", appId],
		queryFn: () => backend.boardState.getCatalog(appId),
		enabled: !!appId && !!packages.data?.length,
	});

	const updates = useQuery<PackageUpdate[]>({
		queryKey: ["app", appId, "package-updates"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<PackageUpdate[]>(
				profile.data,
				`apps/${appId}/packages/updates`,
			);
		},
		enabled:
			!!appId &&
			isOffline.data === false &&
			!!profile.data &&
			!!packages.data?.length,
	});

	const nodesByPackage = useMemo(() => {
		if (!catalog.data) return new Map<string, INode[]>();
		const map = new Map<string, INode[]>();
		for (const node of catalog.data) {
			if (!node.wasm?.package_id) continue;
			const existing = map.get(node.wasm.package_id);
			if (existing) {
				existing.push(node);
			} else {
				map.set(node.wasm.package_id, [node]);
			}
		}
		return map;
	}, [catalog.data]);

	const updatesByPackage = useMemo(() => {
		const map = new Map<string, PackageUpdate>();
		for (const update of updates.data ?? []) {
			map.set(update.packageId, update);
		}
		return map;
	}, [updates.data]);

	const addPackage = useMutation({
		mutationFn: async (req: AddAppPackageRequest) => {
			if (isOffline.data && backend.appState.addPackage) {
				await backend.appState.addPackage(appId, req.packageId, req.version);
				return;
			}
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post(profile.data, `apps/${appId}/packages`, req);
		},
		onSuccess: () => {
			toast.success("Package added");
			queryClient.invalidateQueries({ queryKey: ["app", appId, "packages"] });
			queryClient.invalidateQueries({ queryKey: ["app-catalog-nodes", appId] });
			queryClient.invalidateQueries({ queryKey: ["getCatalog", appId] });
		},
		onError: (err: Error) =>
			toast.error(`Failed to add package: ${err.message}`),
	});

	const removePackage = useMutation({
		mutationFn: async (pkgId: string) => {
			if (isOffline.data && backend.appState.removePackage) {
				await backend.appState.removePackage(appId, pkgId);
				return;
			}
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.del(
				profile.data,
				`apps/${appId}/packages/${pkgId}`,
			);
		},
		onSuccess: () => {
			toast.success("Package removed");
			queryClient.invalidateQueries({ queryKey: ["app", appId, "packages"] });
			queryClient.invalidateQueries({ queryKey: ["app-catalog-nodes", appId] });
			queryClient.invalidateQueries({ queryKey: ["getCatalog", appId] });
		},
		onError: (err: Error) =>
			toast.error(`Failed to remove package: ${err.message}`),
	});

	const toggleAutoUpdate = useMutation({
		mutationFn: async ({
			pkgId,
			autoUpdate,
		}: { pkgId: string; autoUpdate: boolean }) => {
			if (isOffline.data) return;
			if (!profile.data) throw new Error("Profile not loaded");
			const body: UpdateAppPackageRequest = { autoUpdate };
			return backend.apiState.patch(
				profile.data,
				`apps/${appId}/packages/${pkgId}`,
				body,
			);
		},
		onSuccess: () => {
			if (!isOffline.data) {
				toast.success("Auto-update toggled");
				queryClient.invalidateQueries({ queryKey: ["app", appId, "packages"] });
			}
		},
		onError: (err: Error) =>
			toast.error(`Failed to update package: ${err.message}`),
	});

	const reactivatePackage = useMutation({
		mutationFn: async (pkgId: string) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post(
				profile.data,
				`apps/${appId}/packages/${pkgId}/reactivate`,
				{},
			);
		},
		onSuccess: () => {
			toast.success("Package reactivated");
			queryClient.invalidateQueries({ queryKey: ["app", appId, "packages"] });
		},
		onError: (err: Error) =>
			toast.error(`Failed to reactivate: ${err.message}`),
	});

	const patchPackageVersion = useCallback(
		async (pkgId: string, version: string) => {
			if (!profile.data) throw new Error("Profile not loaded");
			const body: UpdateAppPackageRequest = { version };
			return backend.apiState.patch(
				profile.data,
				`apps/${appId}/packages/${pkgId}`,
				body,
			);
		},
		[backend.apiState, profile.data, appId],
	);

	const invalidatePackageQueries = useCallback(() => {
		queryClient.invalidateQueries({ queryKey: ["app", appId, "packages"] });
		queryClient.invalidateQueries({
			queryKey: ["app", appId, "package-updates"],
		});
		queryClient.invalidateQueries({ queryKey: ["app-catalog-nodes", appId] });
		queryClient.invalidateQueries({ queryKey: ["getCatalog", appId] });
	}, [queryClient, appId]);

	const applyUpdate = useMutation({
		mutationFn: ({ pkgId, version }: { pkgId: string; version: string }) =>
			patchPackageVersion(pkgId, version),
		onSuccess: () => {
			toast.success("Package updated");
			invalidatePackageQueries();
		},
		onError: (err: Error) =>
			toast.error(`Failed to update package: ${err.message}`),
	});

	const applyAllUpdates = useMutation({
		mutationFn: async (updatesToApply: PackageUpdate[]) => {
			const results = await Promise.allSettled(
				updatesToApply.map((update) =>
					patchPackageVersion(update.packageId, update.latestVersion),
				),
			);
			const failed = results.filter((r) => r.status === "rejected").length;
			return { total: results.length, failed };
		},
		onSuccess: ({ total, failed }) => {
			if (failed === 0) {
				toast.success(
					total === 1 ? "Package updated" : `${total} packages updated`,
				);
			} else {
				toast.error(`Failed to update ${failed} of ${total} packages`);
			}
			invalidatePackageQueries();
		},
	});

	const handleSelect = useCallback(
		(packageId: string, version: string) => {
			addPackage.mutate({ packageId, version, autoUpdate: !isOffline.data });
			setSearchOpen(false);
		},
		[addPackage, isOffline.data],
	);

	// Updates that can actually be applied: package is not stale and has a
	// newer version available. Stale packages must be reactivated first.
	const applicableUpdates = useMemo(
		() =>
			(packages.data ?? [])
				.filter((p) => !p.stale)
				.flatMap((p) => updatesByPackage.get(p.packageId) ?? []),
		[packages.data, updatesByPackage],
	);

	const excludeIds = packages.data?.map((p) => p.packageId) ?? [];
	const staleCount = packages.data?.filter((p) => p.stale).length ?? 0;

	if (packages.isLoading || isOffline.isLoading)
		return <PackagesPageSkeleton />;

	return (
		<Card>
			<CardHeader className="flex flex-row items-center justify-between">
				<div>
					<CardTitle>Packages</CardTitle>
					<CardDescription>WASM packages linked to this app</CardDescription>
				</div>
				<div className="flex items-center gap-2">
					{applicableUpdates.length > 0 && !isOffline.data && (
						<Button
							size="sm"
							variant="outline"
							onClick={() => applyAllUpdates.mutate(applicableUpdates)}
							disabled={applyAllUpdates.isPending || applyUpdate.isPending}
						>
							<ArrowUpCircle className="mr-2 h-4 w-4" />
							Update all ({applicableUpdates.length})
						</Button>
					)}
					<Button size="sm" onClick={() => setSearchOpen(true)}>
						<Package className="mr-2 h-4 w-4" />
						Add Package
					</Button>
				</div>
			</CardHeader>
			<CardContent>
				{staleCount > 0 && !isOffline.data && (
					<Alert variant="destructive" className="mb-4">
						<TriangleAlert className="h-4 w-4" />
						<AlertTitle>Stale packages detected</AlertTitle>
						<AlertDescription>
							{staleCount} package{staleCount > 1 ? "s are" : " is"} stale
							because the member who added {staleCount > 1 ? "them" : "it"} left
							the project. Stale packages cannot be updated or placed on new
							boards. An admin with access to the package can reactivate it.
						</AlertDescription>
					</Alert>
				)}
				{!packages.data?.length ? (
					<EmptyState
						className="w-full max-w-none grow"
						icons={[Package]}
						title="No packages"
						description="Add a WASM package to get started."
					/>
				) : (
					<div className="space-y-3">
						{packages.data.map((pkg) => (
							<PackageCard
								key={pkg.id}
								pkg={pkg}
								offline={!!isOffline.data}
								nodes={nodesByPackage.get(pkg.packageId) ?? []}
								catalogLoading={catalog.isLoading}
								update={updatesByPackage.get(pkg.packageId)}
								onToggleAutoUpdate={(val) =>
									toggleAutoUpdate.mutate({
										pkgId: pkg.packageId,
										autoUpdate: val,
									})
								}
								onApplyUpdate={() => {
									const update = updatesByPackage.get(pkg.packageId);
									if (update) {
										applyUpdate.mutate({
											pkgId: pkg.packageId,
											version: update.latestVersion,
										});
									}
								}}
								onRemove={() => removePackage.mutate(pkg.packageId)}
								onReactivate={() => reactivatePackage.mutate(pkg.packageId)}
								isToggling={toggleAutoUpdate.isPending}
								isRemoving={removePackage.isPending}
								isReactivating={reactivatePackage.isPending}
								isApplyingUpdate={
									applyUpdate.isPending || applyAllUpdates.isPending
								}
							/>
						))}
					</div>
				)}
			</CardContent>

			<PackageSearchDialog
				open={searchOpen}
				onOpenChange={setSearchOpen}
				onSelect={handleSelect}
				excludePackageIds={excludeIds}
				appId={appId}
			/>
		</Card>
	);
}

function PackageCard(props: {
	pkg: AppPackage;
	offline: boolean;
	nodes: INode[];
	catalogLoading: boolean;
	update?: PackageUpdate;
	onToggleAutoUpdate: (val: boolean) => void;
	onApplyUpdate: () => void;
	onRemove: () => void;
	onReactivate: () => void;
	isToggling: boolean;
	isRemoving: boolean;
	isReactivating: boolean;
	isApplyingUpdate: boolean;
}) {
	const { pkg, nodes, update } = props;
	const [nodesOpen, setNodesOpen] = useState(false);
	const grouped = useMemo(() => groupNodesByCategory(nodes), [nodes]);
	const updatable = !!update && !pkg.stale && !props.offline;

	return (
		<div
			className={`rounded-lg border bg-card transition-colors ${pkg.stale ? "opacity-60" : ""}`}
		>
			<div className="flex items-center gap-3 p-4">
				<div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-muted">
					<Package className="h-4 w-4 text-muted-foreground" />
				</div>
				<div className="min-w-0 flex-1">
					<div className="flex items-center gap-2">
						<span className="truncate font-medium text-sm">
							{pkg.packageName ?? pkg.packageId}
						</span>
						<Badge variant="secondary" className="shrink-0 text-xs">
							v{pkg.version}
						</Badge>
						{pkg.stale ? (
							<Badge variant="destructive" className="shrink-0 text-xs">
								Stale
							</Badge>
						) : (
							<Badge variant="outline" className="shrink-0 text-xs">
								Active
							</Badge>
						)}
					</div>
					{nodes.length > 0 && (
						<p className="mt-0.5 text-xs text-muted-foreground">
							{nodes.length} node{nodes.length !== 1 ? "s" : ""} across{" "}
							{grouped.size} categor{grouped.size !== 1 ? "ies" : "y"}
						</p>
					)}
				</div>
				<div className="flex items-center gap-2 shrink-0">
					{!props.offline && (
						<TooltipProvider delayDuration={300}>
							<Tooltip>
								<TooltipTrigger asChild>
									<div className="flex items-center gap-1.5">
										<Switch
											checked={pkg.autoUpdate}
											onCheckedChange={props.onToggleAutoUpdate}
											disabled={props.isToggling || pkg.stale}
											aria-label="Toggle auto-update"
										/>
									</div>
								</TooltipTrigger>
								<TooltipContent className="max-w-60">
									<p className="font-medium">Auto-update</p>
									<p className="text-xs text-muted-foreground">
										Flags this package to track new versions. Apply available
										updates from the package list.
									</p>
								</TooltipContent>
							</Tooltip>
						</TooltipProvider>
					)}
					{pkg.stale && !props.offline && (
						<Button
							variant="outline"
							size="sm"
							onClick={props.onReactivate}
							disabled={props.isReactivating}
						>
							<RefreshCw className="mr-1 h-3.5 w-3.5" />
							Reactivate
						</Button>
					)}
					<Button
						variant="ghost"
						size="icon"
						className="h-8 w-8"
						onClick={props.onRemove}
						disabled={props.isRemoving}
						aria-label="Remove package"
					>
						<Trash2 className="h-4 w-4 text-destructive" />
					</Button>
				</div>
			</div>

			{updatable && update && (
				<div className="flex items-center gap-2 border-t bg-primary/5 px-4 py-2.5">
					<ArrowUpCircle className="h-4 w-4 shrink-0 text-primary" />
					<span className="min-w-0 flex-1 text-xs text-muted-foreground">
						Update available:{" "}
						<span className="font-medium text-foreground">
							v{update.currentVersion}
						</span>{" "}
						→{" "}
						<span className="font-medium text-foreground">
							v{update.latestVersion}
						</span>
					</span>
					<Button
						size="sm"
						onClick={props.onApplyUpdate}
						disabled={props.isApplyingUpdate}
					>
						<ArrowUpCircle className="mr-1 h-3.5 w-3.5" />
						Apply
					</Button>
				</div>
			)}

			{(nodes.length > 0 || props.catalogLoading) && (
				<Collapsible open={nodesOpen} onOpenChange={setNodesOpen}>
					<CollapsibleTrigger className="flex w-full items-center gap-2 border-t px-4 py-2.5 text-xs font-medium text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-colors">
						{nodesOpen ? (
							<ChevronDown className="h-3.5 w-3.5" />
						) : (
							<ChevronRight className="h-3.5 w-3.5" />
						)}
						<Layers className="h-3.5 w-3.5" />
						<span>Provided Nodes</span>
						<Badge variant="secondary" className="ml-auto text-xs px-1.5 py-0">
							{nodes.length}
						</Badge>
					</CollapsibleTrigger>
					<CollapsibleContent>
						<div className="border-t px-4 py-3">
							{props.catalogLoading ? (
								<div className="space-y-2">
									{Array.from({ length: 3 }).map((_, i) => (
										<Skeleton key={i} className="h-4 w-full" />
									))}
								</div>
							) : (
								<NodeCategoryList grouped={grouped} />
							)}
						</div>
					</CollapsibleContent>
				</Collapsible>
			)}
		</div>
	);
}

function NodeCategoryList({ grouped }: { grouped: Map<string, INode[]> }) {
	return (
		<div className="space-y-3">
			{[...grouped.entries()].map(([category, catNodes]) => (
				<div key={category}>
					<p className="text-xs font-medium text-muted-foreground mb-1.5">
						{category.replace(/\//g, " / ")}
					</p>
					<div className="flex flex-wrap gap-1.5">
						{catNodes.map((node) => (
							<TooltipProvider key={node.name} delayDuration={200}>
								<Tooltip>
									<TooltipTrigger asChild>
										<div className="inline-flex items-center gap-1 rounded-md bg-muted px-2 py-1 text-xs">
											<Zap className="h-3 w-3 text-muted-foreground" />
											<span className="max-w-50 truncate">
												{node.friendly_name || node.name}
											</span>
										</div>
									</TooltipTrigger>
									<TooltipContent
										side="bottom"
										sideOffset={6}
										className="max-w-xs border border-border bg-popover text-popover-foreground shadow-lg [&>svg]:bg-popover [&>svg]:fill-popover"
									>
										<p className="font-medium">
											{node.friendly_name || node.name}
										</p>
										{node.description && (
											<p className="mt-1 text-xs text-popover-foreground/70">
												{node.description}
											</p>
										)}
									</TooltipContent>
								</Tooltip>
							</TooltipProvider>
						))}
					</div>
				</div>
			))}
		</div>
	);
}

function PackagesPageSkeleton() {
	return (
		<Card>
			<CardHeader className="flex flex-row items-center justify-between">
				<div className="space-y-2">
					<Skeleton className="h-5 w-24" />
					<Skeleton className="h-4 w-48" />
				</div>
				<Skeleton className="h-9 w-32" />
			</CardHeader>
			<CardContent className="space-y-3">
				{Array.from({ length: 3 }).map((_, i) => (
					<Skeleton key={i} className="h-20 w-full rounded-lg" />
				))}
			</CardContent>
		</Card>
	);
}

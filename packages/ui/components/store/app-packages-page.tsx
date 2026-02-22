"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Download, Package, RefreshCw, Trash2, TriangleAlert } from "lucide-react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import type {
	AddAppPackageRequest,
	AppPackage,
	UpdateAppPackageRequest,
} from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import { Alert, AlertDescription, AlertTitle } from "../ui/alert";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../ui/card";
import { EmptyState } from "../ui/empty-state";
import { Skeleton } from "../ui/skeleton";
import { Switch } from "../ui/switch";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../ui/table";
import { PackageSearchDialog } from "./package-search-dialog";

export interface AppPackagesPageProps {
	appId: string;
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

	const packages = useQuery<AppPackage[]>({
		queryKey: ["app", appId, "packages"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<AppPackage[]>(
				profile.data,
				`apps/${appId}/packages`,
			);
		},
		enabled: !!profile.data && !!appId,
	});

	const addPackage = useMutation({
		mutationFn: async (req: AddAppPackageRequest) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post(
				profile.data,
				`apps/${appId}/packages`,
				req,
			);
		},
		onSuccess: () => {
			toast.success("Package added");
			queryClient.invalidateQueries({ queryKey: ["app", appId, "packages"] });
		},
		onError: (err: Error) => toast.error(`Failed to add package: ${err.message}`),
	});

	const removePackage = useMutation({
		mutationFn: async (pkgId: string) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.del(
				profile.data,
				`apps/${appId}/packages/${pkgId}`,
			);
		},
		onSuccess: () => {
			toast.success("Package removed");
			queryClient.invalidateQueries({ queryKey: ["app", appId, "packages"] });
		},
		onError: (err: Error) => toast.error(`Failed to remove package: ${err.message}`),
	});

	const toggleAutoUpdate = useMutation({
		mutationFn: async ({ pkgId, autoUpdate }: { pkgId: string; autoUpdate: boolean }) => {
			if (!profile.data) throw new Error("Profile not loaded");
			const body: UpdateAppPackageRequest = { autoUpdate };
			return backend.apiState.patch(
				profile.data,
				`apps/${appId}/packages/${pkgId}`,
				body,
			);
		},
		onSuccess: () => {
			toast.success("Auto-update toggled");
			queryClient.invalidateQueries({ queryKey: ["app", appId, "packages"] });
		},
		onError: (err: Error) => toast.error(`Failed to update package: ${err.message}`),
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
		onError: (err: Error) => toast.error(`Failed to reactivate: ${err.message}`),
	});

	const handleSelect = useCallback(
		(packageId: string, version: string) => {
			addPackage.mutate({ packageId, version, autoUpdate: true });
			setSearchOpen(false);
		},
		[addPackage],
	);

	const excludeIds = packages.data?.map((p) => p.packageId) ?? [];
	const staleCount = packages.data?.filter((p) => p.stale).length ?? 0;

	if (packages.isLoading) return <PackagesPageSkeleton />;

	return (
		<Card>
			<CardHeader className="flex flex-row items-center justify-between">
				<div>
					<CardTitle>Packages</CardTitle>
					<CardDescription>
						WASM packages linked to this app
					</CardDescription>
				</div>
				<Button size="sm" onClick={() => setSearchOpen(true)}>
					<Package className="mr-2 h-4 w-4" />
					Add Package
				</Button>
			</CardHeader>
			<CardContent>
				{staleCount > 0 && (
					<Alert variant="destructive" className="mb-4">
						<TriangleAlert className="h-4 w-4" />
						<AlertTitle>Stale packages detected</AlertTitle>
						<AlertDescription>
							{staleCount} package{staleCount > 1 ? "s are" : " is"} stale because
							the member who added {staleCount > 1 ? "them" : "it"} left the
							project. Stale packages cannot be updated or placed on new boards.
							An admin with access to the package can reactivate it.
						</AlertDescription>
					</Alert>
				)}
				{!packages.data?.length ? (
					<EmptyState
						icons={[Package]}
						title="No packages"
						description="Add a WASM package to get started."
					/>
				) : (
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>Package</TableHead>
								<TableHead>Status</TableHead>
								<TableHead>Version</TableHead>
								<TableHead className="text-center">Auto-update</TableHead>
								<TableHead className="text-right">Actions</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{packages.data.map((pkg) => (
								<PackageRow
									key={pkg.id}
									pkg={pkg}
									onToggleAutoUpdate={(val) =>
										toggleAutoUpdate.mutate({ pkgId: pkg.id, autoUpdate: val })
									}
									onRemove={() => removePackage.mutate(pkg.id)}
									onReactivate={() => reactivatePackage.mutate(pkg.packageId)}
									isToggling={toggleAutoUpdate.isPending}
									isRemoving={removePackage.isPending}
									isReactivating={reactivatePackage.isPending}
								/>
							))}
						</TableBody>
					</Table>
				)}
			</CardContent>

			<PackageSearchDialog
				open={searchOpen}
				onOpenChange={setSearchOpen}
				onSelect={handleSelect}
				excludePackageIds={excludeIds}
			/>
		</Card>
	);
}

function PackageRow(props: {
	pkg: AppPackage;
	onToggleAutoUpdate: (val: boolean) => void;
	onRemove: () => void;
	onReactivate: () => void;
	isToggling: boolean;
	isRemoving: boolean;
	isReactivating: boolean;
}) {
	const { pkg } = props;

	return (
		<TableRow className={pkg.stale ? "opacity-60" : undefined}>
			<TableCell className="font-medium">
				<div className="flex items-center gap-2">
					<Package className="h-4 w-4 text-muted-foreground" />
					{pkg.packageName ?? pkg.packageId}
				</div>
			</TableCell>
			<TableCell>
				{pkg.stale ? (
					<Badge variant="destructive">Stale</Badge>
				) : (
					<Badge variant="outline">Active</Badge>
				)}
			</TableCell>
			<TableCell>
				<Badge variant="secondary">v{pkg.version}</Badge>
			</TableCell>
			<TableCell className="text-center">
				<Switch
					checked={pkg.autoUpdate}
					onCheckedChange={props.onToggleAutoUpdate}
					disabled={props.isToggling || pkg.stale}
					aria-label="Toggle auto-update"
				/>
			</TableCell>
			<TableCell className="text-right space-x-1">
				{pkg.stale ? (
					<Button
						variant="outline"
						size="sm"
						onClick={props.onReactivate}
						disabled={props.isReactivating}
					>
						<RefreshCw className="mr-1 h-4 w-4" />
						Reactivate
					</Button>
				) : null}
				<Button
					variant="ghost"
					size="icon"
					onClick={props.onRemove}
					disabled={props.isRemoving}
					aria-label="Remove package"
				>
					<Trash2 className="h-4 w-4 text-destructive" />
				</Button>
			</TableCell>
		</TableRow>
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
					<Skeleton key={i} className="h-12 w-full rounded-lg" />
				))}
			</CardContent>
		</Card>
	);
}

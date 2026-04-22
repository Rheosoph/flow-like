"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import type { IProfile } from "../../../lib/schema/profile/profile";
import type { RegistryEntry } from "../../../lib/schema/wasm";
import { PackageStatus } from "../../../lib/schema/wasm";
import { useBackend } from "../../../state/backend-state";
import { PackageDetailView } from "../../store/package-detail-view";
import { usePackageStoreData } from "../../store/use-package-store-data";
import type { CompileStatus } from "../../ui/package-status-badge";

// biome-ignore lint/suspicious/noExplicitAny: Required for generic fetcher signature compatibility
export type GenericFetcher = <T>(
	profile: IProfile,
	path: string,
	options?: RequestInit,
	auth?: any,
) => Promise<T>;

export interface StorePackageDetailProps {
	packageId: string;
	onBack: () => void;
	onInstallSuccess?: () => void;
	onUninstallSuccess?: () => void;
	onDeleteSuccess?: () => void;
	onInstallError?: (error: unknown) => void;
	onUninstallError?: (error: unknown) => void;
	fetcher: GenericFetcher;
	auth?: unknown;
	compileStatus?: CompileStatus;
}

export function StorePackageDetail({
	packageId,
	onBack,
	onInstallSuccess,
	onUninstallSuccess,
	onDeleteSuccess,
	onInstallError,
	onUninstallError,
	fetcher,
	auth,
	compileStatus,
}: StorePackageDetailProps) {
	const backend = useBackend();
	const queryClient = useQueryClient();

	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const packageData = useQuery({
		queryKey: ["registry-package", packageId],
		queryFn: async () => {
			if (!profile.data) return null;
			return fetcher<RegistryEntry>(
				profile.data.hub_profile,
				`registry/package/${packageId}`,
				{ method: "GET" },
				auth,
			);
		},
		enabled: !!profile.data && !!packageId,
		retry: false,
	});

	const localPackageData = useQuery({
		queryKey: ["local-package-fallback", packageId],
		queryFn: () => backend.registryState.getPackage(packageId),
		enabled: !!packageId && !packageData.isLoading && !packageData.data,
	});

	const resolvedPkg = useMemo(() => {
		if (packageData.data) return packageData.data;
		if (!localPackageData.data) return undefined;
		const local = localPackageData.data;
		return {
			id: local.id,
			manifest: local.manifest,
			nodes: [],
			versions: [
				{
					version: local.version,
					wasmHash: "",
					wasmSize: 0,
					publishedAt: local.installedAt,
					yanked: false,
				},
			],
			status: PackageStatus.Active,
			downloadCount: 0,
			createdAt: local.installedAt,
			updatedAt: local.installedAt,
			source: local.source,
			verified: false,
			price: 0,
			visibility: "local",
		} as RegistryEntry;
	}, [packageData.data, localPackageData.data]);

	const resolvedLoading =
		packageData.isLoading ||
		(!packageData.data && !packageData.isError && packageData.isFetching) ||
		(!packageData.data && localPackageData.isLoading);

	const installedVersion = useQuery({
		queryKey: ["installed-package", packageId],
		queryFn: () => backend.registryState.getInstalledVersion(packageId),
		enabled: !!packageId,
	});

	const installMutation = useMutation({
		mutationFn: (version?: string) =>
			backend.registryState.installPackage(packageId, version),
		onSuccess: () => {
			onInstallSuccess?.();
			queryClient.invalidateQueries({
				queryKey: ["installed-package", packageId],
			});
		},
		onError: (error: unknown) => {
			onInstallError?.(error);
		},
	});

	const uninstallMutation = useMutation({
		mutationFn: () => backend.registryState.uninstallPackage(packageId),
		onSuccess: () => {
			onUninstallSuccess?.();
			queryClient.invalidateQueries({
				queryKey: ["installed-package", packageId],
			});
		},
		onError: (error: unknown) => {
			onUninstallError?.(error);
		},
	});

	const handleAccessChanged = useCallback(() => {
		queryClient.invalidateQueries({
			queryKey: ["registry-package", packageId],
		});
	}, [queryClient, packageId]);

	const {
		isPurchasing,
		isRequesting,
		priceLabel,
		hasAccess,
		onBuy,
		onGetOrBuy,
	} = usePackageStoreData(
		packageId || undefined,
		resolvedPkg,
		fetcher,
		auth,
		handleAccessChanged,
	);

	const handleInstall = useCallback(
		(version?: string) => installMutation.mutate(version),
		[installMutation],
	);

	const handleUninstall = useCallback(
		() => uninstallMutation.mutate(),
		[uninstallMutation],
	);

	return (
		<PackageDetailView
			pkg={resolvedPkg}
			isLoading={resolvedLoading}
			installedVersion={installedVersion.data}
			onBack={onBack}
			onInstall={handleInstall}
			onUninstall={handleUninstall}
			isInstalling={installMutation.isPending}
			isUninstalling={uninstallMutation.isPending}
			compileStatus={compileStatus}
			price={resolvedPkg?.price}
			visibility={resolvedPkg?.visibility}
			priceLabel={priceLabel}
			hasAccess={hasAccess}
			isPurchasing={isPurchasing}
			isRequesting={isRequesting}
			onBuy={onBuy}
			onGetOrBuy={onGetOrBuy}
			onDeleteSuccess={onDeleteSuccess}
			currentUserPermission={resolvedPkg?.currentUserPermission}
			fetcher={fetcher}
			auth={auth}
		/>
	);
}

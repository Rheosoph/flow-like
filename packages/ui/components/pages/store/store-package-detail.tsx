"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import type { IProfile } from "../../../lib/schema/profile/profile";
import type { RegistryEntry } from "../../../lib/schema/wasm";
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
	onInstallError?: (error: Error) => void;
	onUninstallError?: (error: Error) => void;
	fetcher: GenericFetcher;
	auth?: unknown;
	compileStatus?: CompileStatus;
}

export function StorePackageDetail({
	packageId,
	onBack,
	onInstallSuccess,
	onUninstallSuccess,
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
	});

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
		onError: (error: Error) => {
			onUninstallError?.(error);
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
		onError: (error: Error) => {
			onUninstallError?.(error);
		},
	});

	const handleAccessChanged = useCallback(() => {
		queryClient.invalidateQueries({ queryKey: ["registry-package", packageId] });
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
		packageData.data,
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
			pkg={packageData.data}
			isLoading={packageData.isLoading}
			installedVersion={installedVersion.data}
			onBack={onBack}
			onInstall={handleInstall}
			onUninstall={handleUninstall}
			isInstalling={installMutation.isPending}
			isUninstalling={uninstallMutation.isPending}
			compileStatus={compileStatus}
			price={packageData.data?.price}
			visibility={packageData.data?.visibility}
			priceLabel={priceLabel}
			hasAccess={hasAccess}
			isPurchasing={isPurchasing}
			isRequesting={isRequesting}
			onBuy={onBuy}
			onGetOrBuy={onGetOrBuy}
			currentUserPermission={packageData.data?.currentUserPermission}
			fetcher={fetcher}
			auth={auth}
		/>
	);
}

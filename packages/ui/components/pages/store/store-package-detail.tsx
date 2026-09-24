"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { isRecord } from "../../../lib/response-shape";
import type { IProfile } from "../../../lib/schema/profile/profile";
import type { RegistryEntry } from "../../../lib/schema/wasm";
import { PackageStatus } from "../../../lib/schema/wasm";
import { useBackend } from "../../../state/backend-state";
import { MarketplaceCheckoutDialog } from "../../payments/checkout-dialog";
import { PackageDetailView } from "../../store/package-detail-view";
import { usePackageStoreData } from "../../store/use-package-store-data";
import type { CompileStatus } from "../../ui/package-status-badge";

export type GenericFetcher = <T>(
	profile: IProfile,
	path: string,
	options?: RequestInit,
	// biome-ignore lint/suspicious/noExplicitAny: Required for generic fetcher signature compatibility
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

	// The detail view dereferences `manifest` and `versions` unguarded.
	const remotePkg =
		isRecord(packageData.data) &&
		isRecord(packageData.data.manifest) &&
		Array.isArray(packageData.data.versions)
			? packageData.data
			: undefined;

	const localPackageData = useQuery({
		queryKey: ["local-package-fallback", packageId],
		queryFn: () => backend.registryState.getPackage(packageId),
		enabled: !!packageId && !packageData.isLoading && !remotePkg,
	});

	const resolvedPkg = useMemo(() => {
		if (remotePkg) return remotePkg;
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
	}, [remotePkg, localPackageData.data]);

	// A deep link can mount this before the settings profile resolves; both remote queries are
	// disabled until then, so without this the view would claim the package does not exist.
	const resolvedLoading =
		profile.isLoading ||
		packageData.isLoading ||
		(!packageData.data && !packageData.isError && packageData.isFetching) ||
		(!remotePkg && localPackageData.isLoading);

	const loadError = packageData.error ?? profile.error ?? null;

	const handleRetry = useCallback(() => {
		void profile.refetch();
		void packageData.refetch();
		void localPackageData.refetch();
	}, [profile, packageData, localPackageData]);

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
		awaitingCheckout,
		checkoutOpen,
		setCheckoutOpen,
		priceLabel,
		hasAccess,
		onBuy,
		onRequestAccess,
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
		<>
			{remotePkg && (
				<MarketplaceCheckoutDialog
					key={`checkout:${packageId}`}
					itemKind="PACKAGE"
					itemId={packageId}
					itemName={remotePkg.manifest.name || packageId}
					amount={remotePkg.price ?? 0}
					open={checkoutOpen}
					onOpenChange={setCheckoutOpen}
					onPurchased={handleAccessChanged}
				/>
			)}
			<PackageDetailView
				pkg={resolvedPkg}
				isLoading={resolvedLoading}
				loadError={loadError}
				onRetry={handleRetry}
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
				awaitingCheckout={awaitingCheckout}
				onBuy={onBuy}
				onRequestAccess={onRequestAccess}
				onGetOrBuy={onGetOrBuy}
				onDeleteSuccess={onDeleteSuccess}
				currentUserPermission={resolvedPkg?.currentUserPermission}
				fetcher={fetcher}
				auth={auth}
			/>
		</>
	);
}

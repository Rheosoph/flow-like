"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import type { IProfile } from "../../../lib/schema/profile/profile";
import { useBackend } from "../../../state/backend-state";
import { MarketplaceCheckoutDialog } from "../../payments/checkout-dialog";
import { PackageDetailView } from "../../store/package-detail-view";
import {
	type RegistryPackageAuth,
	useRegistryPackage,
} from "../../store/package-workspace/use-registry-package";
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
	/** @deprecated Deleting moved to the package workspace; the store detail never calls it. */
	onDeleteSuccess?: () => void;
	onInstallError?: (error: unknown) => void;
	onUninstallError?: (error: unknown) => void;
	fetcher: GenericFetcher;
	auth?: RegistryPackageAuth;
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
	const registry = useRegistryPackage(packageId, fetcher, auth);
	const resolvedPkg = registry.entry;

	const signIn = useCallback(() => {
		void auth?.signinRedirect?.({
			url_state: window.location.pathname + window.location.search,
		});
	}, [auth]);

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
			{registry.source === "registry" && resolvedPkg && (
				<MarketplaceCheckoutDialog
					key={`checkout:${packageId}`}
					itemKind="PACKAGE"
					itemId={packageId}
					itemName={resolvedPkg.manifest.name || packageId}
					amount={resolvedPkg.price ?? 0}
					open={checkoutOpen}
					onOpenChange={setCheckoutOpen}
					onPurchased={handleAccessChanged}
				/>
			)}
			<PackageDetailView
				pkg={resolvedPkg}
				isLoading={registry.isLoading}
				loadError={registry.error}
				onRetry={registry.retry}
				onSignIn={registry.authState === "expired" ? signIn : undefined}
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
				currentUserPermission={resolvedPkg?.currentUserPermission}
				fetcher={fetcher}
				auth={auth}
			/>
		</>
	);
}

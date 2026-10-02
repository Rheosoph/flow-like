"use client";

import {
	type QueryClient,
	type UseQueryResult,
	useQueries,
} from "@tanstack/react-query";
import { useCallback } from "react";
import {
	type ManifestAccess,
	readManifestAccess,
} from "../../../lib/app-package-overview";
import {
	readManifestWidgetBundleHash,
	readManifestWidgets,
} from "../../../lib/package-widgets";
import type { PackageWidgetEntry } from "../../../lib/schema/wasm";
import { useBackend } from "../../../state/backend-state";

export const APP_PACKAGE_MANIFEST_KEY = "app-package-manifest";

const MANIFEST_STALE_MS = 5 * 60_000;

/** Reloads everything read from an app's package pins after they changed. */
export function invalidateAppPackageQueries(
	queryClient: QueryClient,
	appId: string,
): void {
	const queryKeys = [
		["app", appId, "packages"],
		["app", appId, "package-updates"],
		["app-catalog-nodes", appId],
		["getCatalog", appId],
		["app-package-widgets", appId],
		[APP_PACKAGE_MANIFEST_KEY],
	];
	for (const queryKey of queryKeys)
		void queryClient.invalidateQueries({ queryKey });
}

export interface PackageManifestView {
	access?: ManifestAccess;
	widgets: PackageWidgetEntry[];
	bundleHash?: string;
}

export interface PackageManifests {
	byId: ReadonlyMap<string, PackageManifestView>;
	loading: boolean;
}

/**
 * Manifests of the given packages: the installed copy on desktop, the
 * registry entry of the version the app pins on web. Both resolve through the
 * app, so members see packages the project licenses. A package the host
 * cannot resolve is simply absent, so the page falls back to what the node
 * catalog reports.
 */
export function usePackageManifests(
	packageIds: readonly string[],
	appId: string,
): PackageManifests {
	const backend = useBackend();
	const idsKey = packageIds.join("\n");
	const combine = useCallback(
		(results: UseQueryResult<PackageManifestView | null>[]) => {
			const ids = idsKey ? idsKey.split("\n") : [];
			const byId = new Map<string, PackageManifestView>();
			ids.forEach((id, index) => {
				const view = results[index]?.data;
				if (view) byId.set(id, view);
			});
			return {
				byId,
				loading: results.some((result) => result.isLoading),
			};
		},
		[idsKey],
	);

	return useQueries({
		queries: packageIds.map((packageId) => ({
			queryKey: [APP_PACKAGE_MANIFEST_KEY, appId, packageId],
			queryFn: async (): Promise<PackageManifestView | null> => {
				const pkg = await backend.registryState.getPackage(packageId, appId);
				if (!pkg) return null;
				return {
					access: readManifestAccess(pkg.manifest),
					widgets: readManifestWidgets(pkg.manifest),
					bundleHash: readManifestWidgetBundleHash(pkg.manifest),
				};
			},
			staleTime: MANIFEST_STALE_MS,
			retry: false,
		})),
		combine,
	});
}

"use client";

import { type UseQueryResult, useQueries } from "@tanstack/react-query";
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
 * registry entry on web. A package the host cannot resolve is simply absent,
 * so the page falls back to what the node catalog reports.
 */
export function usePackageManifests(
	packageIds: readonly string[],
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
			queryKey: [APP_PACKAGE_MANIFEST_KEY, packageId],
			queryFn: async (): Promise<PackageManifestView | null> => {
				const pkg = await backend.registryState.getPackage(packageId);
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

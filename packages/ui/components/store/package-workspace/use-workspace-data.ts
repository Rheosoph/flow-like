"use client";

import { type QueryClient, useQuery } from "@tanstack/react-query";
import { useInvoke } from "../../../hooks/use-invoke";
import type { PackageMeta, PackageUser } from "../../../lib/schema/wasm";
import { useBackend } from "../../../state/backend-state";
import type { GenericFetcher } from "../../pages/store/store-package-detail";

export function useHubProfile() {
	const backend = useBackend();
	return useInvoke(backend.userState.getSettingsProfile, backend.userState, []);
}

/** After a lifecycle change: the entry, and the Mine lists that show its state (desktop and web). */
export function invalidatePackageLists(
	queryClient: QueryClient,
	packageId: string,
) {
	for (const queryKey of [
		["registry-package", packageId],
		["mine-registry-maintained"],
		["mine-registry-lookup"],
	]) {
		void queryClient.invalidateQueries({ queryKey });
	}
}

/** Same key and fallback as `PackageMetaTab`, so the listing form and the previews share one cache entry. */
export function usePackageMeta(
	packageId: string | undefined,
	fetcher: GenericFetcher,
	auth?: unknown,
) {
	const profile = useHubProfile();
	const hubProfile = profile.data?.hub_profile;
	return useQuery<PackageMeta | null>({
		queryKey: ["package-meta", packageId],
		queryFn: async () => {
			if (!hubProfile || !packageId) return null;
			try {
				return await fetcher<PackageMeta>(
					hubProfile,
					`registry/package/${packageId}/meta`,
					{ method: "GET" },
					auth,
				);
			} catch {
				return null;
			}
		},
		enabled: !!hubProfile && !!packageId,
	});
}

/** Maintainer-only; same key as `PackageUsersContainer`. */
export function usePackageUsers(
	packageId: string,
	fetcher: GenericFetcher,
	auth?: unknown,
) {
	const profile = useHubProfile();
	const hubProfile = profile.data?.hub_profile;
	return useQuery<PackageUser[]>({
		queryKey: ["package-users", packageId],
		queryFn: () => {
			if (!hubProfile) throw new Error("Profile not loaded");
			return fetcher<PackageUser[]>(
				hubProfile,
				`registry/package/${packageId}/users`,
				{ method: "GET" },
				auth,
			);
		},
		enabled: !!hubProfile,
	});
}

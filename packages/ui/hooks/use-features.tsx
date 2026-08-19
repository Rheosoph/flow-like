"use client";

import { useQuery } from "@tanstack/react-query";
import type { IFeatures } from "../lib/schema/hub/hub";
import { useBackend } from "../state/backend-state";
import { useInvoke } from "./use-invoke";

/**
 * Resolve the platform feature flags from the backend the app is actually
 * talking to (`GET /info/features`) instead of the cached hub config object.
 *
 * `useHub` fetches the hub root document, which can be stale, point at the
 * wrong hub, or fail silently (leaving every feature gate off). Feature
 * availability must be authoritative, so it is read through the same backend
 * abstraction (`apiState`) that every other API call uses.
 */
export function useFeatures() {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const hubProfile = profile.data?.hub_profile;

	return useQuery<IFeatures>({
		queryKey: ["info-features", hubProfile?.hub],
		queryFn: () => {
			if (!hubProfile) throw new Error("Profile not loaded");
			return backend.apiState.get<IFeatures>(hubProfile, "info/features");
		},
		enabled: !!hubProfile,
		meta: { persist: false },
		staleTime: 5 * 60 * 1000,
	});
}

"use client";

import { useQuery } from "@tanstack/react-query";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../hooks/use-invoke";
import { getApiOrigin } from "../../lib/api-url";
import { GlobalPermission } from "../../lib/permission/global-permission";
import type { IProfile } from "../../lib/schema/profile/profile";
import { useBackend, useBackendReady } from "../../state/backend-state";

export function useProfileTemplates() {
	const backend = useBackend();
	const ready = useBackendReady();
	const auth = useAuth();
	const scope = [
		getApiOrigin(backend.profile),
		auth?.user?.profile.sub ?? "local",
		backend.profile?.id,
	];
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		ready,
		scope,
	);
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		ready,
		scope,
	);
	const permission = new GlobalPermission(info.data?.permission ?? 0);
	const canWrite =
		!info.isError && permission.hasPermission(GlobalPermission.WriteProfile);
	const canRead =
		canWrite ||
		(!info.isError && permission.hasPermission(GlobalPermission.ReadProfile));
	const canEditHome =
		!info.isError &&
		permission.hasPermission(GlobalPermission.WriteLandingPage);
	const effectiveScope = [
		...scope,
		getApiOrigin(profile.data),
		profile.data?.id,
	];
	const queryKey = ["profile-templates", ...effectiveScope];
	const templates = useQuery({
		queryKey,
		queryFn: () => {
			if (!profile.data) throw new Error("Your profile could not be loaded.");
			return backend.apiState.get<IProfile[]>(profile.data, "info/profiles");
		},
		enabled: ready && !!profile.data && canRead,
	});
	return {
		backend,
		profile,
		info,
		templates,
		canRead,
		canWrite,
		canEditHome,
		queryKey,
		scopeKey: JSON.stringify(effectiveScope),
		loading: !ready || profile.isLoading || info.isLoading,
	};
}

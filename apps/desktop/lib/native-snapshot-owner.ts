import { useBackendStore } from "@flow-like/flow-like-ui/state/backend-state";

export interface NativeSnapshotOwner {
	profileId: string;
	hub: string;
	subject: string | null;
	issuer: string | null;
}

export function nativeSnapshotOwner(): NativeSnapshotOwner | undefined {
	const backend = useBackendStore.getState().backend as {
		profile?: { id?: string; hub?: string; secure?: boolean };
		auth?: { user?: { profile?: { sub?: string; iss?: string } } | null };
	} | null;
	if (!backend?.profile?.id) return undefined;
	const profile = backend.profile;
	let hub = "";
	if (profile.hub?.trim()) {
		const value = profile.hub.trim();
		hub = new URL(
			value.includes("://")
				? value
				: `${profile.secure === false ? "http" : "https"}://${value}`,
		).href.replace(/\/+$/, "");
	}
	return {
		profileId: profile.id!,
		hub,
		subject: backend.auth?.user?.profile?.sub ?? null,
		issuer: backend.auth?.user?.profile?.iss ?? null,
	};
}

export function matchesNativeSnapshotOwner(
	owner?: NativeSnapshotOwner,
): boolean {
	const current = nativeSnapshotOwner();
	return (
		!!owner &&
		!!current &&
		owner.profileId === current.profileId &&
		owner.hub === current.hub &&
		owner.subject === current.subject &&
		owner.issuer === current.issuer
	);
}

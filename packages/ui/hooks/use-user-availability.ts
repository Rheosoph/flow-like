"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useRef } from "react";
import { useAuth } from "react-oidc-context";
import { getApiOrigin } from "../lib/api-url";
import {
	type AvailabilityStatus,
	type AvailabilityUpdate,
	type UserAvailability,
	clearLocalAvailability,
	effectiveAvailability,
	readLocalAvailability,
	updateLocalAvailability,
} from "../lib/availability";
import { useBackend } from "../state/backend-state";
import { useInvoke } from "./use-invoke";

export function useUserAvailability({
	local = false,
	enabled = true,
}: { local?: boolean; enabled?: boolean } = {}) {
	const backend = useBackend();
	const auth = useAuth();
	const client = useQueryClient();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const activeProfile = backend.profile ?? profile.data;
	const isLocal = local || !auth.isAuthenticated;
	const scope = JSON.stringify([
		activeProfile ? getApiOrigin(activeProfile) : "unresolved",
		activeProfile?.id,
		auth.user?.profile.iss,
		auth.isAuthenticated ? auth.user?.profile.sub : "signedout",
		isLocal ? "device" : "account",
	]);
	const current = useRef({ scope, state: backend.userState });
	current.current = { scope, state: backend.userState };
	const active = () =>
		current.current.scope === scope &&
		current.current.state === backend.userState;
	const key = ["userAvailability", scope] as const;
	const load = useCallback(async (): Promise<UserAvailability> => {
		if (isLocal) return readLocalAvailability(scope);
		if (!activeProfile)
			throw new Error("Wait for your account profile to load.");
		if (!backend.userState.getAvailability)
			throw new Error("Availability is unavailable on this Hub.");
		const value = await backend.userState.getAvailability();
		if (
			current.current.scope !== scope ||
			current.current.state !== backend.userState
		)
			throw new Error("Your account changed. Try again.");
		return effectiveAvailability(value);
	}, [backend.userState, isLocal, scope, activeProfile]);
	const query = useQuery({
		queryKey: key,
		queryFn: load,
		enabled: enabled && (isLocal || Boolean(activeProfile)),
		staleTime: 10_000,
		refetchInterval: 15_000,
	});
	const invalidate = async () => {
		await Promise.all([
			client.invalidateQueries({ queryKey: key }),
			client.invalidateQueries({ queryKey: ["lookupUserBatched"] }),
		]);
	};
	const update = async (value: AvailabilityUpdate) => {
		if (!active()) throw new Error("Your account changed. Try again.");
		const next = isLocal
			? updateLocalAvailability(scope, value)
			: await backend.userState.updateAvailability?.(value);
		if (!next) throw new Error("Availability is unavailable on this Hub.");
		if (!active()) return next;
		client.setQueryData(key, next);
		await invalidate();
		return next;
	};
	const clear = async () => {
		if (!active()) throw new Error("Your account changed. Try again.");
		const next = isLocal
			? clearLocalAvailability(scope)
			: await backend.userState.clearAvailability?.();
		if (!next) throw new Error("Availability is unavailable on this Hub.");
		if (!active()) return next;
		client.setQueryData(key, next);
		await invalidate();
		return next;
	};
	const applyLocalStatus = (status: AvailabilityStatus, until: number) => {
		// An accepted inbox response belongs to the scope that approved it, even
		// when its request card closed while the send was finishing.
		if (!isLocal) return;
		client.setQueryData(
			key,
			updateLocalAvailability(scope, { status }, "bot", Date.now(), until),
		);
	};
	return {
		...query,
		data: query.data ? effectiveAvailability(query.data) : undefined,
		isLocal,
		scope,
		load,
		update,
		clear,
		applyLocalStatus,
		invalidate,
	};
}

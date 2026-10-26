"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import type { IHub } from "../lib";
import { getApiOrigin } from "../lib/api-url";
import { useBackend } from "../state/backend-state";
import { subscribeToHub } from "./hub-request";
import { useInvoke } from "./use-invoke";

export function useHub(queryScope: string[] = []) {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		true,
		queryScope,
	);
	const origin = profile.data ? getApiOrigin(profile.data) : undefined;
	const scope = JSON.stringify([origin, profile.data?.id, ...queryScope]);
	const [snapshot, setSnapshot] = useState<{ scope: string; hub?: IHub }>();
	const subscription = useRef<{
		scope: string;
		refetch: () => Promise<void>;
	} | null>(null);

	const fetchHub = useCallback(async () => {
		if (subscription.current?.scope === scope)
			await subscription.current.refetch();
	}, [scope]);

	useEffect(() => {
		if (!origin) return;
		const current = subscribeToHub(scope, origin, (hub) =>
			setSnapshot({ scope, hub }),
		);
		subscription.current = { scope, refetch: current.refetch };
		return () => {
			current.release();
			subscription.current = null;
		};
	}, [origin, scope]);

	// Scope changes must hide old availability before the next effect runs.
	return {
		hub: snapshot?.scope === scope ? snapshot.hub : undefined,
		refetch: fetchHub,
	};
}

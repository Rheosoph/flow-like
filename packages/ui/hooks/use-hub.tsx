"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import type { IHub } from "../lib";
import { upstreamFailureInSuccess } from "../lib/api-error";
import { getApiOrigin } from "../lib/api-url";
import { isRecord } from "../lib/response-shape";
import { useBackend } from "../state/backend-state";
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
	const [snapshot, setSnapshot] = useState<{ scope: string; hub: IHub }>();
	const pending = useRef<AbortController | null>(null);

	const fetchHub = useCallback(async () => {
		pending.current?.abort();
		pending.current = null;
		if (!origin) return;
		const request = new AbortController();
		pending.current = request;
		// The hub root has no trailing slash, matching hosted API routing.
		try {
			const hubData = await fetch(`${origin}/api/v1`, {
				signal: request.signal,
				cache: "no-store",
				headers: {
					"Cache-Control": "no-cache",
					Pragma: "no-cache",
				},
			});
			if (request.signal.aborted || pending.current !== request) return;
			if (!hubData.ok) {
				console.error(
					`Hub config fetch returned ${hubData.status} from ${origin}/api/v1`,
				);
				return;
			}
			const hubJson: unknown = await hubData.json();
			if (request.signal.aborted || pending.current !== request) return;
			const upstreamError = upstreamFailureInSuccess(hubData, hubJson);
			if (upstreamError || !isRecord(hubJson)) {
				console.error(
					`Hub config fetch from ${origin}/api/v1 returned no hub record`,
					upstreamError?.message,
				);
				return;
			}
			setSnapshot({ scope, hub: hubJson as unknown as IHub });
		} catch (err) {
			if (request.signal.aborted || pending.current !== request) return;
			console.error("Failed to fetch hub config:", err);
		}
	}, [origin, scope]);

	useEffect(() => {
		void fetchHub();
		return () => {
			pending.current?.abort();
			pending.current = null;
		};
	}, [fetchHub]);

	// Scope changes must hide old availability before the next effect runs.
	return {
		hub: snapshot?.scope === scope ? snapshot.hub : undefined,
		refetch: fetchHub,
	};
}

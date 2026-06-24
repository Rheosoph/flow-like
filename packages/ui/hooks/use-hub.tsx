"use client";

import { useCallback, useEffect, useState } from "react";
import type { IHub } from "../lib";
import { useBackend } from "../state/backend-state";
import { useInvoke } from "./use-invoke";

export function useHub() {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const [hub, setHub] = useState<IHub | undefined>();

	const fetchHub = useCallback(async () => {
		if (!profile.data?.hub) {
			setHub(undefined);
			return;
		}
		let hubUrl = profile.data.hub;
		if (!hubUrl.startsWith("http://") && !hubUrl.startsWith("https://")) {
			const protocol = (profile.data?.secure ?? true) ? "https" : "http";
			hubUrl = `${protocol}://${hubUrl}`;
		}
		// Strip any trailing slash so we control the suffix exactly. The live
		// API exposes the hub root at `/api/v1`; `/api/v1/` can 404 behind
		// CloudFront/Lambda routing.
		const base = hubUrl.replace(/\/+$/, "");
		try {
			const hubData = await fetch(`${base}/api/v1`, {
				cache: "no-store",
				headers: {
					"Cache-Control": "no-cache",
					Pragma: "no-cache",
				},
			});
			if (!hubData.ok) {
				console.error(
					`Hub config fetch returned ${hubData.status} from ${base}/api/v1`,
				);
				return;
			}
			const hubJson: IHub = await hubData.json();
			setHub(hubJson);
		} catch (err) {
			console.error("Failed to fetch hub config:", err);
		}
	}, [profile.data?.hub, profile.data?.secure]);

	useEffect(() => {
		fetchHub();
	}, [fetchHub]);

	return { hub, refetch: fetchHub };
}

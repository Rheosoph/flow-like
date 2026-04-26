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
		if (!profile.data?.hub) return;
		let hubUrl = profile.data.hub;
		if (!hubUrl.startsWith("http://") && !hubUrl.startsWith("https://")) {
			const protocol = (profile.data?.secure ?? true) ? "https" : "http";
			hubUrl = `${protocol}://${hubUrl}`;
		}
		// Strip any trailing slash so we control the suffix exactly. The hub
		// root handler is `.route("/", ...)` nested under `/api/v1`, which
		// in axum 0.8 only matches the trailing-slash form.
		const base = hubUrl.replace(/\/+$/, "");
		try {
			const hubData = await fetch(`${base}/api/v1/`, {});
			if (!hubData.ok) {
				console.error(
					`Hub config fetch returned ${hubData.status} from ${base}/api/v1/`,
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
	}, [profile.data?.hub]);

	return { hub, refetch: fetchHub };
}

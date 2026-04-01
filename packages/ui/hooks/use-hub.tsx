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
		// If hub doesn't already have a protocol, add one
		if (!hubUrl.startsWith("http://") && !hubUrl.startsWith("https://")) {
			const protocol = (profile.data?.secure ?? true) ? "https" : "http";
			hubUrl = `${protocol}://${hubUrl}`;
		}
		try {
			const hubData = await fetch(`${hubUrl}/api/v1`, {});
			const hubJson: IHub = await hubData.json();
			setHub(hubJson);
		} catch (err) {
			console.error("Failed to fetch hub config:", err);
		}
	}, [profile.data?.hub]);

	useEffect(() => {
		fetchHub();
	}, [profile.data?.hub]);

	return { hub, refetch: fetchHub };
}

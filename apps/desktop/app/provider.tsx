// app/providers.tsx
"use client";
import { invoke } from "@tauri-apps/api/core";
import posthog from "posthog-js";
import { PostHogProvider } from "posthog-js/react";
import { useEffect } from "react";

async function isTrackingAuthorized(): Promise<boolean> {
	try {
		const status = await invoke<string>("get_tracking_authorization_status");
		return status === "authorized";
	} catch {
		return true;
	}
}

export function PHProvider({
	children,
}: Readonly<{
	children: React.ReactNode;
}>) {
	useEffect(() => {
		const apiKey = process.env.NEXT_PUBLIC_POSTHOG_KEY;

		if (!apiKey) {
			return;
		}

		const apiHost = process.env.NEXT_PUBLIC_POSTHOG_HOST ?? "https://app.posthog.com";

		posthog.init(apiKey, {
			api_host: apiHost,
			person_profiles: "always",
			capture_pageleave: true,
			autocapture: true,
			enable_heatmaps: true,
		});

		isTrackingAuthorized().then((authorized) => {
			if (!authorized) {
				posthog.opt_out_capturing();
			} else {
				posthog.opt_in_capturing();
			}
		});
	}, []);

	return <PostHogProvider client={posthog}>{children}</PostHogProvider>;
}

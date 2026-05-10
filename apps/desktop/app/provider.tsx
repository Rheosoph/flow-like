// app/providers.tsx
"use client";
import posthog from "posthog-js";
import { PostHogProvider } from "posthog-js/react";
import { useEffect } from "react";

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

		const apiHost =
			process.env.NEXT_PUBLIC_POSTHOG_HOST ?? "https://app.posthog.com";

		posthog.init(apiKey, {
			api_host: apiHost,
			ui_host: 'https://eu.posthog.com',
			person_profiles: "always",
			capture_pageleave: true,
			autocapture: true,
			enable_heatmaps: true,
		});
	}, []);

	return <PostHogProvider client={posthog}>{children}</PostHogProvider>;
}

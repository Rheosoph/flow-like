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
		posthog.init(process.env.NEXT_PUBLIC_POSTHOG_KEY!, {
			api_host: process.env.NEXT_PUBLIC_POSTHOG_HOST,
			person_profiles: "always",
			capture_pageleave: true,
			autocapture: true,
			enable_heatmaps: true,
		});
	}, []);

	return <PostHogProvider client={posthog}>{children}</PostHogProvider>;
}

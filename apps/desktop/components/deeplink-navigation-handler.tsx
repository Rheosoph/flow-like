"use client";

import { listen } from "@tauri-apps/api/event";
import { useRouter } from "next/navigation";
import { type ReactNode, useEffect } from "react";

interface DeeplinkStorePayload {
	appId: string | null;
	packageId?: string | null;
}

interface DeeplinkJoinPayload {
	appId: string | null;
	token: string | null;
}

export function DeeplinkNavigationHandler({
	children,
}: Readonly<{ children: ReactNode }>) {
	const router = useRouter();

	useEffect(() => {
		const storeUnlisten = listen<DeeplinkStorePayload>(
			"deeplink/store",
			(event) => {
				const { appId, packageId } = event.payload;
				if (packageId) {
					console.log("Navigating to package store page:", packageId);
					router.push(`/store/packages?id=${encodeURIComponent(packageId)}`);
					return;
				}
				if (appId) {
					console.log("Navigating to store page for app:", appId);
					router.push(`/store?id=${encodeURIComponent(appId)}`);
				}
			},
		);

		const joinUnlisten = listen<DeeplinkJoinPayload>(
			"deeplink/join",
			(event) => {
				const { appId, token } = event.payload;
				if (appId && token) {
					console.log("Navigating to join page:", appId, token);
					router.push(`/join?appId=${appId}&token=${token}`);
				}
			},
		);

		return () => {
			storeUnlisten.then((unsub) => unsub());
			joinUnlisten.then((unsub) => unsub());
		};
	}, [router]);

	return <>{children}</>;
}

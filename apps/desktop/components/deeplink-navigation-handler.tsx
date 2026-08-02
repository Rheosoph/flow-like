"use client";

import { invoke } from "@tauri-apps/api/core";
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

// `get_current()` keeps returning the launch URL for the whole process lifetime, and Rust re-emits
// on replay, so both guards are module-scoped: a remount must not drag the user back to a route
// they already navigated away from.
const handledTargets = new Set<string>();
let replayRequested = false;

export function DeeplinkNavigationHandler({
	children,
}: Readonly<{ children: ReactNode }>) {
	const router = useRouter();

	useEffect(() => {
		const navigateOnce = (key: string, target: string) => {
			if (handledTargets.has(key)) return;
			handledTargets.add(key);
			router.push(target);
		};

		const storeUnlisten = listen<DeeplinkStorePayload>(
			"deeplink/store",
			(event) => {
				const { appId, packageId } = event.payload;
				if (packageId) {
					navigateOnce(
						`package:${packageId}`,
						`/store/packages?id=${encodeURIComponent(packageId)}`,
					);
					return;
				}
				if (appId) {
					navigateOnce(`app:${appId}`, `/store?id=${encodeURIComponent(appId)}`);
				}
			},
		);

		const joinUnlisten = listen<DeeplinkJoinPayload>(
			"deeplink/join",
			(event) => {
				const { appId, token } = event.payload;
				if (appId && token) {
					navigateOnce(
						`join:${appId}:${token}`,
						`/join?appId=${encodeURIComponent(appId)}&token=${encodeURIComponent(token)}`,
					);
				}
			},
		);

		// A deep link that cold-started the app was emitted during Rust `setup()`, before this
		// component existed — Tauri drops emissions to webviews with no listener, so the
		// navigation was lost. Ask Rust to replay it now that both listeners are attached.
		if (!replayRequested) {
			replayRequested = true;
			void Promise.all([storeUnlisten, joinUnlisten])
				.then(() => invoke("deeplink_replay_pending"))
				.catch((error) => {
					console.warn("Failed to replay pending deep links:", error);
				});
		}

		return () => {
			storeUnlisten.then((unsub) => unsub());
			joinUnlisten.then((unsub) => unsub());
		};
	}, [router]);

	return <>{children}</>;
}

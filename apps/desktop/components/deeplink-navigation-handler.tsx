"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useRouter } from "next/navigation";
import { type ReactNode, useEffect } from "react";

interface DeeplinkStorePayload {
	appId: string | null;
	packageId?: string | null;
	replayed?: boolean;
}

interface DeeplinkJoinPayload {
	appId: string | null;
	token: string | null;
	replayed?: boolean;
}

// `get_current()` keeps returning the launch URL for the whole process
// lifetime, and every page load asks Rust to replay it. Replayed emissions
// (flagged by Rust) are deduplicated; fresh clicks always navigate, so
// re-opening the same invite link works. The handled set lives in
// sessionStorage so a hard reload (e.g. ProfileSyncer's onboarding redirect)
// doesn't drag the user back to an already-handled deep link.
const HANDLED_KEY = "flow-like-handled-deeplinks";
let replayRequested = false;

function wasHandled(key: string): boolean {
	try {
		const raw = sessionStorage.getItem(HANDLED_KEY);
		return raw ? (JSON.parse(raw) as string[]).includes(key) : false;
	} catch {
		return false;
	}
}

function markHandled(key: string): void {
	try {
		const raw = sessionStorage.getItem(HANDLED_KEY);
		const handled = raw ? (JSON.parse(raw) as string[]) : [];
		if (!handled.includes(key)) {
			handled.push(key);
			sessionStorage.setItem(HANDLED_KEY, JSON.stringify(handled));
		}
	} catch {}
}

export function DeeplinkNavigationHandler({
	children,
}: Readonly<{ children: ReactNode }>) {
	const router = useRouter();

	useEffect(() => {
		const navigate = (key: string, target: string, replayed?: boolean) => {
			if (replayed && wasHandled(key)) return;
			markHandled(key);
			router.push(target);
		};

		const storeUnlisten = listen<DeeplinkStorePayload>(
			"deeplink/store",
			(event) => {
				const { appId, packageId, replayed } = event.payload;
				if (packageId) {
					navigate(
						`package:${packageId}`,
						`/store/packages?id=${encodeURIComponent(packageId)}`,
						replayed,
					);
					return;
				}
				if (appId) {
					navigate(
						`app:${appId}`,
						`/store?id=${encodeURIComponent(appId)}`,
						replayed,
					);
				}
			},
		);

		const joinUnlisten = listen<DeeplinkJoinPayload>(
			"deeplink/join",
			(event) => {
				const { appId, token, replayed } = event.payload;
				if (appId && token) {
					navigate(
						`join:${appId}:${token}`,
						`/join?appId=${encodeURIComponent(appId)}&token=${encodeURIComponent(token)}`,
						replayed,
					);
				}
			},
		);

		// A deep link that cold-started the app was emitted during Rust `setup()`,
		// before this component existed — Tauri drops emissions to webviews with
		// no listener, so the navigation was lost. Ask Rust to replay it now that
		// both listeners are attached.
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

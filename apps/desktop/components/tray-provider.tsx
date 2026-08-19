"use client";

import { useTranslation } from "@flow-like/locales";
import {
	useBackend,
	useInvoke,
	useNetworkStatus,
} from "@flow-like/flow-like-ui";
import { useSpotlightStore } from "@flow-like/flow-like-ui/state/spotlight-state";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useRouter } from "next/navigation";
import { useEffect, useMemo } from "react";
import { TauriBackend } from "./tauri-provider";

interface TraySyncStatus {
	status: string;
	detail?: string;
}

interface TrayUpdate {
	unreadCount?: number;
	syncStatus?: TraySyncStatus;
	signedIn?: boolean;
}

const NOTIFICATION_POLL_INTERVAL = 60_000;

const pushTrayUpdate = (update: TrayUpdate) =>
	invoke("tray_update_state", { update }).catch((error) =>
		console.warn("Failed to update tray state", error),
	);

const TrayProvider: React.FC = () => {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const isOnline = useNetworkStatus();
	const router = useRouter();

	const syncStatus = useMemo<TraySyncStatus>(
		() => ({
			status: isOnline ? "Online" : "Offline",
			detail: isOnline ? t('cloudSyncActive', 'Cloud sync active') : t('waitingForNetwork', 'Waiting for network'),
		}),
		[isOnline],
	);

	useEffect(() => {
		pushTrayUpdate({ syncStatus });
	}, [syncStatus]);

	// Share the notifications overview query with the page and sidebar so that
	// marking notifications read (which invalidates this query) updates the tray
	// immediately instead of lagging a full poll interval behind.
	const overview = useInvoke(
		backend.userState.getNotifications,
		backend.userState,
		[],
	);

	useEffect(() => {
		// Only report facts we positively know: a failed fetch must not flip the
		// tray to zero-unread (offline is a normal state for a signed-in user).
		// Auth state comes from the local OIDC context, not from network success.
		const data = overview.data;
		const update: TrayUpdate = {};
		if (data) {
			// The sidebar badge sums unread + invites; keep the tray in agreement.
			update.unreadCount = (data.unread_count ?? 0) + (data.invites_count ?? 0);
		}
		if (backend instanceof TauriBackend) {
			update.signedIn = Boolean(
				backend.auth?.isAuthenticated && backend.auth?.user?.access_token,
			);
		}
		if (Object.keys(update).length > 0) void pushTrayUpdate(update);
	}, [overview.data, backend]);

	useEffect(() => {
		// Still poll so notifications that arrive server-side (no local action to
		// invalidate the query) surface without waiting for user interaction.
		const intervalId = setInterval(() => {
			void overview.refetch();
		}, NOTIFICATION_POLL_INTERVAL);

		return () => clearInterval(intervalId);
	}, [overview.refetch]);

	useEffect(() => {
		const unlistenNavigate = listen<string>("tray:navigate", (event) => {
			if (typeof event.payload === "string") router.push(event.payload);
		});
		const unlistenOpenSpotlight = listen("tray:open-spotlight", () => {
			useSpotlightStore.getState().open();
		});
		const unlistenQuickCreate = listen("tray:open-quick-create", () => {
			useSpotlightStore.getState().open();
			useSpotlightStore.getState().setMode("quick-create");
		});

		return () => {
			Promise.allSettled([
				unlistenNavigate,
				unlistenOpenSpotlight,
				unlistenQuickCreate,
			]).then((results) => {
				for (const result of results) {
					if (result.status === "fulfilled") result.value();
				}
			});
		};
	}, [router]);

	return null;
};

export default TrayProvider;

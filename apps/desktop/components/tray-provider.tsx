"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { check } from "@tauri-apps/plugin-updater";
import { useBackend, useNetworkStatus } from "@tm9657/flow-like-ui";
import { useSpotlightStore } from "@tm9657/flow-like-ui/state/spotlight-state";
import { useEffect, useMemo } from "react";

interface TrayNotification {
	id: string;
	title: string;
	read: boolean;
	createdAt?: string;
}

interface TraySyncStatus {
	status: string;
	detail?: string;
}

interface TrayAccountState {
	label: string;
	tier?: string;
}

interface TrayUpdateState {
	available: boolean;
}

interface TrayUpdate {
	notifications?: TrayNotification[];
	unreadCount?: number;
	syncStatus?: TraySyncStatus;
	updateState?: TrayUpdateState;
	accountState?: TrayAccountState;
}

const TrayProvider: React.FC = () => {
	const backend = useBackend();
	const isOnline = useNetworkStatus();

	const syncStatus = useMemo<TraySyncStatus>(
		() => ({
			status: isOnline ? "Online" : "Offline",
			detail: isOnline ? "Cloud sync active" : "Waiting for network",
		}),
		[isOnline],
	);

	useEffect(() => {
		invoke("tray_update_state", {
			update: {
				syncStatus,
			},
		}).catch((error) =>
			console.warn("Failed to update tray sync status", error),
		);
	}, [syncStatus]);

	useEffect(() => {
		let mounted = true;
		let intervalId: NodeJS.Timeout | undefined;

		const updateTrayMeta = async () => {
			try {
				const [overview, notifications, userInfo, updateAvailable] =
					await Promise.all([
						backend.userState.getNotifications().catch(() => null),
						backend.userState.listNotifications(false, 0, 5).catch(() => []),
						backend.userState.getInfo().catch(() => null),
						check().catch(() => null),
					]);

				if (!mounted) return;

				const trayNotifications = notifications.map((notification) => ({
					id: notification.id,
					title: notification.title,
					read: notification.read,
					createdAt: notification.created_at,
				}));

				const accountState: TrayAccountState = {
					label:
						userInfo?.name ??
						userInfo?.email ??
						userInfo?.username ??
						"Signed out",
					tier: userInfo?.tier ?? userInfo?.status ?? undefined,
				};

				const updateState: TrayUpdateState = {
					available: Boolean(updateAvailable),
				};

				await invoke("tray_update_state", {
					update: {
						notifications: trayNotifications,
						unreadCount: overview?.unread_count ?? 0,
						accountState,
						updateState,
					},
				});
			} catch (error) {
				console.warn("Failed to update tray metadata", error);
			}
		};

		updateTrayMeta();
		intervalId = setInterval(updateTrayMeta, 60000);

		return () => {
			mounted = false;
			if (intervalId) clearInterval(intervalId);
		};
	}, [backend]);

	useEffect(() => {
		const unlistenOpenSpotlight = listen("tray:open-spotlight", () => {
			useSpotlightStore.getState().open();
		});
		const unlistenQuickCreate = listen("tray:open-quick-create", () => {
			useSpotlightStore.getState().open();
			useSpotlightStore.getState().setMode("quick-create");
		});
		const unlistenUpdate = listen("tray:restart-update", () => {
			invoke("update").catch((error) =>
				console.warn("Failed to trigger update", error),
			);
		});

		return () => {
			Promise.all([
				unlistenOpenSpotlight,
				unlistenQuickCreate,
				unlistenUpdate,
			])
				.then((unlisteners) => {
					for (const unlisten of unlisteners) {
						unlisten();
					}
				})
				.catch(() => undefined);
		};
	}, []);

	return null;
};

export default TrayProvider;

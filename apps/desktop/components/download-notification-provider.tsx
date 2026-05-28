"use client";

import {
	type DownloadCompleteListener,
	useDownloadManager,
} from "@flow-like/flow-like-ui";
import { useEffect, useRef } from "react";
import { toast } from "sonner";

type NotificationPermission = "granted" | "denied" | "default";
type NotificationApi = {
	isPermissionGranted: () => Promise<boolean>;
	requestPermission: () => Promise<NotificationPermission>;
	sendNotification: (options: { title: string; body?: string }) => void;
};

async function loadNotificationPlugin(): Promise<NotificationApi | null> {
	try {
		const mod = await import("@tauri-apps/plugin-notification");
		return {
			isPermissionGranted: mod.isPermissionGranted,
			requestPermission: mod.requestPermission,
			sendNotification: mod.sendNotification,
		};
	} catch {
		return null;
	}
}

export default function DownloadNotificationProvider() {
	const onComplete = useDownloadManager((s) => s.onComplete);
	const notificationApi = useRef<NotificationApi | null>(null);
	const permissionGranted = useRef<boolean>(false);

	useEffect(() => {
		const initNotifications = async () => {
			try {
				const api = await loadNotificationPlugin();
				if (api) {
					notificationApi.current = api;
					let granted = await api.isPermissionGranted();
					if (!granted) {
						const permission = await api.requestPermission();
						granted = permission === "granted";
					}
					permissionGranted.current = granted;
				}
			} catch (e) {
				console.log(
					"[DownloadNotificationProvider] Desktop notifications not available:",
					e,
				);
			}
		};

		initNotifications();
	}, []);

	useEffect(() => {
		const handleDownloadComplete: DownloadCompleteListener = (bit) => {
			const modelName =
				bit.meta?.en?.name || bit.meta?.en?.short || bit.id || "Model";

			if (notificationApi.current && permissionGranted.current) {
				notificationApi.current.sendNotification({
					title: "Download Complete",
					body: `${modelName} has been downloaded successfully.`,
				});
			} else {
				toast.success("Download Complete", {
					description: `${modelName} has been downloaded successfully.`,
				});
			}
		};

		const unsubscribe = onComplete(handleDownloadComplete);
		return unsubscribe;
	}, [onComplete]);

	return null;
}

"use client";

import { useQueryClient } from "@tanstack/react-query";
import { type Event, type UnlistenFn, listen } from "@tauri-apps/api/event";
import { useBackend, useHub } from "@tm9657/flow-like-ui";
import type { IIntercomEvent, INotificationEvent, IPushNotificationsConfig } from "@tm9657/flow-like-ui";
import { useEffect, useRef } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { fetcher } from "../lib/api";
import { addLocalNotification } from "../lib/notifications-db";

type NotificationPermission = "granted" | "denied" | "default";
type NotificationApi = {
	isPermissionGranted: () => Promise<boolean>;
	requestPermission: () => Promise<NotificationPermission>;
	sendNotification: (options: { title: string; body?: string }) => void;
};

type PushTargetPlatform = "IOS" | "ANDROID" | "DESKTOP";

type RemotePushPayload = {
	title?: string;
	body?: string;
	data: Record<string, unknown>;
	badge?: number;
	sound?: string;
	channelId?: string;
	category?: string;
};

type RemotePushListener = {
	unregister: () => Promise<void> | void;
};

type RemotePushApi = {
	getToken: () => Promise<string>;
	requestPermission: () => Promise<{ granted: boolean }>;
	onNotificationReceived: (
		handler: (notification: RemotePushPayload) => void,
	) => Promise<RemotePushListener>;
	onNotificationTapped: (
		handler: (notification: RemotePushPayload) => void,
	) => Promise<RemotePushListener>;
	onTokenRefresh: (
		handler: (token: string) => void,
	) => Promise<RemotePushListener>;
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

async function loadRemotePushPlugin(): Promise<RemotePushApi | null> {
	try {
		const mod = await import("tauri-plugin-remote-push-api");
		return {
			getToken: mod.getToken,
			requestPermission: mod.requestPermission,
			onNotificationReceived: mod.onNotificationReceived,
			onNotificationTapped: mod.onNotificationTapped,
			onTokenRefresh: mod.onTokenRefresh,
		};
	} catch {
		return null;
	}
}

function getPushDeviceId(): string {
	if (typeof window === "undefined") {
		return "server-device";
	}

	const storageKey = "flow-like-push-device-id";
	const existing = window.localStorage.getItem(storageKey);
	if (existing) {
		return existing;
	}

	const created = crypto.randomUUID();
	window.localStorage.setItem(storageKey, created);
	return created;
}

function detectPushPlatform(): PushTargetPlatform | null {
	if (typeof navigator === "undefined") {
		return null;
	}

	const userAgent = navigator.userAgent.toLowerCase();
	if (userAgent.includes("android")) {
		return "ANDROID";
	}
	if (
		userAgent.includes("iphone") ||
		userAgent.includes("ipad") ||
		userAgent.includes("ipod")
	) {
		return "IOS";
	}
	if ("__TAURI_INTERNALS__" in window) {
		return "DESKTOP";
	}

	return null;
}

function canUseRemotePushForPlatform(
	pushConfig: IPushNotificationsConfig | undefined,
	platform: PushTargetPlatform | null,
): boolean {
	if (!pushConfig?.enabled || pushConfig.provider !== "fcm" || !platform) {
		return false;
	}

	if (platform === "DESKTOP") {
		return pushConfig.allow_desktop === true;
	}

	return pushConfig.allow_mobile === true;
}

function dataString(
	data: Record<string, unknown>,
	key: string,
): string | undefined {
	const value = data[key];
	return typeof value === "string" && value.trim().length > 0 ? value : undefined;
}

interface NotificationProviderProps {
	appId?: string;
}

export default function NotificationProvider({
	appId,
}: NotificationProviderProps = {}) {
	const auth = useAuth();
	const backend = useBackend();
	const hub = useHub();
	const queryClient = useQueryClient();
	// Use a constant for offline/unauthenticated users
	const userId = auth.user?.profile?.sub ?? "offline-user";
	const notificationApi = useRef<NotificationApi | null>(null);
	const permissionGranted = useRef<boolean>(false);
	const remotePushApi = useRef<RemotePushApi | null>(null);
	const remotePushListeners = useRef<RemotePushListener[]>([]);
	const lastRegisteredToken = useRef<string | null>(null);
	const deviceId = useRef<string | null>(null);
	const pushConfig = hub.hub?.push_notifications;

	const storeNotification = async ({
		title,
		description,
		icon,
		link,
		sourceRunId,
		sourceNodeId,
		notificationType,
	}: {
		title: string;
		description?: string;
		icon?: string;
		link?: string;
		sourceRunId?: string;
		sourceNodeId?: string;
		notificationType?: "WORKFLOW" | "SYSTEM";
	}) => {
		try {
			await addLocalNotification({
				userId,
				appId,
				title,
				description,
				icon,
				link,
				notificationType: notificationType ?? "WORKFLOW",
				sourceRunId,
				sourceNodeId,
			});

			await queryClient.refetchQueries({
				predicate: (query) => {
					const key = query.queryKey[0];
					return key === "getNotifications" || key === "listNotifications";
				},
			});
		} catch (error) {
			console.error(
				"[NotificationProvider] Failed to store local notification:",
				error,
			);
		}
	};

	const registerPushTarget = async (token: string) => {
		const platform = detectPushPlatform();
		if (
			!remotePushApi.current ||
			!backend?.profile ||
			!auth.user ||
			!canUseRemotePushForPlatform(pushConfig, platform) ||
			!platform
		) {
			return;
		}

		if (!deviceId.current) {
			deviceId.current = getPushDeviceId();
		}

		await fetcher<{ id: string; success: boolean }>(
			backend.profile,
			"user/push-targets/register",
			{
				method: "POST",
				body: JSON.stringify({
					device_id: deviceId.current,
					platform,
					token,
					device_name:
						typeof navigator !== "undefined" ? navigator.userAgent : undefined,
					channel_id: pushConfig?.channel_id,
					metadata: {
						app_id: appId,
						platform,
						provider: pushConfig?.provider,
					},
				}),
			},
			auth,
		);

		lastRegisteredToken.current = token;
	};

	const unregisterPushTarget = async () => {
		if (!backend?.profile || !auth.user || !deviceId.current) {
			return;
		}

		try {
			await fetcher<{ success: boolean }>(
				backend.profile,
				`user/push-targets/${deviceId.current}`,
				{
					method: "DELETE",
				},
				auth,
			);
		} catch (error) {
			console.warn(
				"[NotificationProvider] Failed to unregister push target:",
				error,
			);
		}
	};

	useEffect(() => {
		const initNotifications = async () => {
			deviceId.current = getPushDeviceId();

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
				// Notification plugin not available (e.g., in dev mode or unsupported platform)
				console.log(
					"[NotificationProvider] Desktop notifications not available:",
					e,
				);
			}

			try {
				remotePushApi.current = await loadRemotePushPlugin();
			} catch (error) {
				console.log(
					"[NotificationProvider] Remote push plugin not available:",
					error,
				);
			}
		};

		initNotifications();
	}, []);

	useEffect(() => {
		const platform = detectPushPlatform();
		const canRegister =
			auth.isAuthenticated &&
			backend?.profile &&
			canUseRemotePushForPlatform(pushConfig, platform);
		if (!canRegister) {
			if (auth.isAuthenticated && backend?.profile && auth.user && deviceId.current) {
				void unregisterPushTarget();
			}
			return;
		}

		let cancelled = false;

		const initRemotePush = async () => {
			if (!remotePushApi.current) {
				return;
			}

			try {
				const permission = await remotePushApi.current.requestPermission();
				if (!permission.granted) {
					await unregisterPushTarget();
					return;
				}

				const token = await remotePushApi.current.getToken();
				if (!token) {
					console.warn(
						"[NotificationProvider] Remote push plugin returned an empty FCM token.",
					);
					return;
				}
				if (!cancelled && token && token !== lastRegisteredToken.current) {
					await registerPushTarget(token);
				}

				remotePushListeners.current.push(
					await remotePushApi.current.onTokenRefresh(async (nextToken) => {
						if (!nextToken || nextToken === lastRegisteredToken.current) {
							return;
						}

						try {
							await registerPushTarget(nextToken);
						} catch (error) {
							console.warn(
								"[NotificationProvider] Failed to refresh push token:",
								error,
							);
						}
					}),
				);

				remotePushListeners.current.push(
					await remotePushApi.current.onNotificationReceived(async (notification) => {
						await storeNotification({
							title: notification.title ?? "Notification",
							description: notification.body,
							icon: dataString(notification.data, "icon"),
							link: dataString(notification.data, "link"),
							sourceRunId: dataString(notification.data, "source_run_id"),
							sourceNodeId: dataString(notification.data, "source_node_id"),
							notificationType: (dataString(notification.data, "notification_type") as "WORKFLOW" | "SYSTEM") ?? "SYSTEM",
						});

						toast.info(notification.title ?? "Notification", {
							description: notification.body,
						});
					}),
				);

				remotePushListeners.current.push(
					await remotePushApi.current.onNotificationTapped(async (notification) => {
						await storeNotification({
							title: notification.title ?? "Notification",
							description: notification.body,
							icon: dataString(notification.data, "icon"),
							link: dataString(notification.data, "link"),
							sourceRunId: dataString(notification.data, "source_run_id"),
							sourceNodeId: dataString(notification.data, "source_node_id"),
							notificationType: (dataString(notification.data, "notification_type") as "WORKFLOW" | "SYSTEM") ?? "SYSTEM",
						});

						const link = dataString(notification.data, "link");
						if (link && typeof window !== "undefined") {
							window.location.assign(link);
						}
					}),
				);
			} catch (error) {
				console.warn(
					"[NotificationProvider] Failed to initialize remote push registration:",
					error,
				);
			}
		};

		initRemotePush();

		return () => {
			cancelled = true;
			const listeners = remotePushListeners.current.splice(0);
			void Promise.allSettled(
				listeners.map((listener) => Promise.resolve(listener.unregister())),
			);
		};
	}, [auth.isAuthenticated, auth.user, backend?.profile, pushConfig, appId]);

	useEffect(() => {
		const subscriptions: (Promise<UnlistenFn> | undefined)[] = [];

		const unlistenFn = listen(
			"flow_notification",
			async (events: Event<IIntercomEvent[]>) => {
				for (const event of events.payload) {
					const notification = event.payload as INotificationEvent;

					await storeNotification({
						title: notification.title,
						description: notification.description,
						icon: notification.icon,
						link: notification.link,
						sourceRunId: notification.source_run_id,
						sourceNodeId: notification.source_node_id,
					});

					// Persist notification via backend API (requires event_id)
					if (
						appId &&
						backend?.profile &&
						auth.user &&
						notification.event_id &&
						notification.event_id.trim().length > 0
					) {
						try {
							await fetcher<{ id: string; success: boolean }>(
								backend.profile,
								`apps/${appId}/notifications/create`,
								{
									method: "POST",
									body: JSON.stringify({
										event_id: notification.event_id,
										target_user_sub: notification.target_user_sub,
										title: notification.title,
										description: notification.description,
										icon: notification.icon,
										link: notification.link,
										run_id: notification.source_run_id,
										node_id: notification.source_node_id,
									}),
								},
								auth,
							);
						} catch (e) {
							console.warn(
								"[NotificationProvider] Failed to persist remote notification:",
								e,
							);
						}
					}

					// Show desktop notification if enabled
					if (
						notificationApi.current &&
						permissionGranted.current &&
						notification.show_desktop
					) {
						notificationApi.current.sendNotification({
							title: notification.title,
							body: notification.description ?? undefined,
						});
					} else {
						toast.info(notification.title, {
							description: notification.description,
						});
					}
				}
			},
		);

		subscriptions.push(unlistenFn);

		return () => {
			(async () => {
				for await (const subscription of subscriptions) {
					if (subscription) subscription();
				}
			})();
		};
	}, [userId, appId, queryClient, backend?.profile, auth.user]);

	return null;
}

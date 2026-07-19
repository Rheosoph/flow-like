"use client";

import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { type Event, type UnlistenFn, listen } from "@tauri-apps/api/event";
import { useBackend, useHub } from "@flow-like/flow-like-ui";
import type { IIntercomEvent, INotificationEvent } from "@flow-like/flow-like-ui";
import { useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { fetcher } from "../lib/api";
import {
	FLOW_NOTIFICATION_EVENT,
	type FlowNotificationBatchDetail,
} from "../lib/flow-notification-events";
import { addLocalNotification } from "../lib/notifications-db";
import {
	REMOTE_PUSH_PREFERENCE_EVENT,
	canUseRemotePushForPlatform,
	detectPushPlatform,
	getPushDeviceId,
	isRemotePushPreferenceEnabled,
	loadRemotePushPlugin,
	type PushTargetPlatform,
	type RemotePushApi,
	type RemotePushListener,
	type RemotePushPayload,
} from "../lib/remote-push";
import type { TauriBackend } from "./tauri-provider";

type NotificationPermission = "granted" | "denied" | "default";
type NotificationApi = {
	isPermissionGranted: () => Promise<boolean>;
	requestPermission: () => Promise<NotificationPermission>;
	sendNotification: (options: { title: string; body?: string }) => void;
};

type RemotePushPluginState = "loading" | "available" | "unavailable";

const LOCAL_EXECUTION_SUB = "local";

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

function dataString(
	data: Record<string, unknown>,
	key: string,
): string | undefined {
	const value = data[key];
	return typeof value === "string" && value.trim().length > 0
		? value
		: undefined;
}

function normalizeColdStartPayload(
	userInfo: Record<string, unknown>,
): RemotePushPayload {
	const data: Record<string, unknown> = {};
	let title: string | undefined;
	let body: string | undefined;
	let badge: number | undefined;
	let sound: string | undefined;
	let category: string | undefined;

	for (const [key, value] of Object.entries(userInfo)) {
		if (key === "aps" && value && typeof value === "object") {
			const aps = value as Record<string, unknown>;
			const alert = aps.alert;
			if (alert && typeof alert === "object") {
				const alertObj = alert as Record<string, unknown>;
				if (typeof alertObj.title === "string") title = alertObj.title;
				if (typeof alertObj.body === "string") body = alertObj.body;
			} else if (typeof alert === "string") {
				body = alert;
			}
			if (typeof aps.badge === "number") badge = aps.badge;
			if (typeof aps.sound === "string") sound = aps.sound;
			if (typeof aps.category === "string") category = aps.category;
			continue;
		}
		if (key === "from" || key === "collapse_key" || key === "message_type") {
			continue;
		}
		if (key.startsWith("gcm.") || key.startsWith("google.")) continue;
		data[key] = value;
	}

	return { title, body, data, badge, sound, category };
}

function appPathFromNotificationLink(link: string): string | null {
	if (link.startsWith("/") && !link.startsWith("//")) {
		return link;
	}

	try {
		const url = new URL(link);
		const currentHost =
			typeof window !== "undefined" ? window.location.hostname : null;
		const isKnownAppHost =
			url.hostname === "app.flow-like.com" ||
			url.hostname === "localhost" ||
			url.hostname === "127.0.0.1" ||
			url.hostname === currentHost;
		if (
			(url.protocol === "https:" || url.protocol === "http:") &&
			isKnownAppHost
		) {
			return `${url.pathname}${url.search}${url.hash}`;
		}
	} catch {
		return null;
	}

	return null;
}

interface NotificationProviderProps {
	appId?: string;
}

export default function NotificationProvider({
	appId,
}: NotificationProviderProps = {}) {
	const auth = useAuth();
	const router = useRouter();
	const backend = useBackend();
	const tauriBackend = backend as TauriBackend | undefined;
	const authContext = tauriBackend?.auth ?? auth;
	const currentUser = authContext.user;
	const isAuthenticated = authContext.isAuthenticated;
	const hub = useHub();
	const queryClient = useQueryClient();
	const userId = currentUser?.profile?.sub ?? "offline-user";
	const notificationApi = useRef<NotificationApi | null>(null);
	const permissionGranted = useRef<boolean>(false);
	const remotePushApi = useRef<RemotePushApi | null>(null);
	const remotePushListeners = useRef<RemotePushListener[]>([]);
	const tapListener = useRef<RemotePushListener | null>(null);
	const lastRegistrationKey = useRef<string | null>(null);
	const deviceId = useRef<string | null>(null);
	const [pushDeviceId, setPushDeviceId] = useState<string | null>(null);
	const [remotePushPluginState, setRemotePushPluginState] =
		useState<RemotePushPluginState>("loading");
	const [remotePushPreferenceEnabled, setRemotePushPreferenceEnabled] =
		useState(isRemotePushPreferenceEnabled);
	const pushConfig = hub.hub?.push_notifications;
	const handleTapRef = useRef<(notification: RemotePushPayload) => void>(
		() => {},
	);

	const storeNotification = async ({
		title,
		description,
		icon,
		link,
		appIdOverride,
		sourceRunId,
		sourceNodeId,
		notificationType,
	}: {
		title: string;
		description?: string;
		icon?: string;
		link?: string;
		appIdOverride?: string;
		sourceRunId?: string;
		sourceNodeId?: string;
		notificationType?: "WORKFLOW" | "SYSTEM";
	}) => {
		try {
			const notificationAppId = appIdOverride ?? appId;

			await addLocalNotification({
				userId,
				appId: notificationAppId,
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

	handleTapRef.current = (notification: RemotePushPayload) => {
		void storeNotification({
			title: notification.title ?? "Notification",
			description: notification.body,
			icon: dataString(notification.data, "icon"),
			link: dataString(notification.data, "link"),
			appIdOverride: dataString(notification.data, "app_id") ?? appId,
			sourceRunId: dataString(notification.data, "source_run_id"),
			sourceNodeId: dataString(notification.data, "source_node_id"),
			notificationType:
				(dataString(notification.data, "notification_type") as
					| "WORKFLOW"
					| "SYSTEM") ?? "SYSTEM",
		});

		const link = dataString(notification.data, "link");
		console.log(
			"[NotificationProvider] handleTap link=",
			link,
			"data=",
			notification.data,
		);
		if (link && typeof window !== "undefined") {
			const appPath = appPathFromNotificationLink(link);
			if (appPath) {
				router.push(appPath);
				return;
			}

			if (link.startsWith("http://") || link.startsWith("https://")) {
				window.open(link, "_blank", "noopener,noreferrer");
				return;
			}

			window.location.assign(link);
		}
	};

	const pushTargetRegistrationKey = (
		token: string,
		platform: PushTargetPlatform,
	): string => {
		return JSON.stringify({
			user: currentUser?.profile?.sub ?? "",
			hub: backend?.profile?.hub ?? "",
			deviceId: deviceId.current ?? pushDeviceId ?? "",
			platform,
			provider: pushConfig?.provider ?? "",
			channelId: pushConfig?.channel_id ?? "",
			token,
		});
	};

	const registerPushTarget = async (token: string) => {
		const platform = detectPushPlatform();
		if (
			!remotePushApi.current ||
			!backend?.profile ||
			!currentUser ||
			!canUseRemotePushForPlatform(pushConfig, platform) ||
			!platform
		) {
			return;
		}

		if (!deviceId.current) {
			deviceId.current = await getPushDeviceId();
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
			authContext,
		);

		lastRegistrationKey.current = pushTargetRegistrationKey(token, platform);
	};

	useEffect(() => {
		let cancelled = false;

		const initNotifications = async () => {
			const nextDeviceId = await getPushDeviceId();
			if (cancelled) {
				return;
			}
			deviceId.current = nextDeviceId;
			setPushDeviceId(nextDeviceId);

			try {
				const api = await loadNotificationPlugin();
				if (cancelled) {
					return;
				}
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
				const api = await loadRemotePushPlugin();
				if (cancelled) {
					return;
				}
				remotePushApi.current = api;
				setRemotePushPluginState(api ? "available" : "unavailable");
			} catch (error) {
				if (cancelled) {
					return;
				}
				setRemotePushPluginState("unavailable");
				console.log(
					"[NotificationProvider] Remote push plugin not available:",
					error,
				);
			}

			// Register the tap listener as early as possible - independent of
			// auth state - so taps that arrive before authentication is
			// hydrated (or before the auth-gated effect runs) still navigate.
			if (remotePushApi.current && !tapListener.current) {
				try {
					tapListener.current =
						await remotePushApi.current.onNotificationTapped((notification) => {
							console.log(
								"[NotificationProvider] live tap fired",
								notification,
							);
							handleTapRef.current(notification);
						});
					console.log("[NotificationProvider] tap listener registered");
				} catch (error) {
					console.warn(
						"[NotificationProvider] Failed to register tap listener:",
						error,
					);
				}
			}

			// Drain any cold-start tap captured natively before the JS bundle
			// loaded. On iOS, when the app is launched from a tap, the plugin
			// flushes its `notification-tapped` event before any JS listener
			// can register - so the event is lost. The native bridge persists
			// the userInfo to UserDefaults; this call retrieves and clears it.
			try {
				const pending = await invoke<Record<string, unknown> | null>(
					"get_pending_notification_tap",
				);
				console.log("[NotificationProvider] cold-start tap pending=", pending);
				if (pending && typeof pending === "object") {
					const payload = normalizeColdStartPayload(pending);
					console.log(
						"[NotificationProvider] cold-start normalized payload=",
						payload,
					);
					handleTapRef.current(payload);
				}
			} catch (error) {
				console.log(
					"[NotificationProvider] get_pending_notification_tap failed:",
					error,
				);
			}
		};

		initNotifications();

		return () => {
			cancelled = true;
			const listener = tapListener.current;
			tapListener.current = null;
			if (listener) {
				void Promise.resolve(listener.unregister());
			}
		};
	}, []);

	useEffect(() => {
		if (typeof window === "undefined") {
			return;
		}

		const handlePreferenceChange = () => {
			setRemotePushPreferenceEnabled(isRemotePushPreferenceEnabled());
		};

		window.addEventListener(
			REMOTE_PUSH_PREFERENCE_EVENT,
			handlePreferenceChange,
		);
		return () => {
			window.removeEventListener(
				REMOTE_PUSH_PREFERENCE_EVENT,
				handlePreferenceChange,
			);
		};
	}, []);

	useEffect(() => {
		const platform = detectPushPlatform();
		if (
			!isAuthenticated ||
			!backend?.profile ||
			!currentUser ||
			!pushDeviceId ||
			!remotePushPreferenceEnabled ||
			remotePushPluginState === "loading"
		) {
			return;
		}

		if (
			remotePushPluginState !== "available" ||
			!remotePushApi.current ||
			!pushConfig ||
			!platform ||
			!canUseRemotePushForPlatform(pushConfig, platform)
		) {
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
					console.warn(
						"[NotificationProvider] Remote push permission not granted; keeping existing server target untouched.",
					);
					return;
				}

				const token = await remotePushApi.current.getToken();
				if (!token) {
					console.warn(
						"[NotificationProvider] Remote push plugin returned an empty FCM token.",
					);
					return;
				}
				if (
					!cancelled &&
					token &&
					pushTargetRegistrationKey(token, platform) !==
						lastRegistrationKey.current
				) {
					await registerPushTarget(token);
				}

				remotePushListeners.current.push(
					await remotePushApi.current.onTokenRefresh(async (nextToken) => {
						if (
							!nextToken ||
							pushTargetRegistrationKey(nextToken, platform) ===
								lastRegistrationKey.current
						) {
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
					await remotePushApi.current.onNotificationReceived(
						async (notification) => {
							await storeNotification({
								title: notification.title ?? "Notification",
								description: notification.body,
								icon: dataString(notification.data, "icon"),
								link: dataString(notification.data, "link"),
								appIdOverride: dataString(notification.data, "app_id") ?? appId,
								sourceRunId: dataString(notification.data, "source_run_id"),
								sourceNodeId: dataString(notification.data, "source_node_id"),
								notificationType:
									(dataString(notification.data, "notification_type") as
										| "WORKFLOW"
										| "SYSTEM") ?? "SYSTEM",
							});

							toast.info(notification.title ?? "Notification", {
								description: notification.body,
							});
						},
					),
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
	}, [
		isAuthenticated,
		currentUser,
		backend?.profile,
		pushConfig,
		appId,
		pushDeviceId,
		remotePushPreferenceEnabled,
		remotePushPluginState,
	]);

	useEffect(() => {
		const subscriptions: (Promise<UnlistenFn> | undefined)[] = [];

		const handleNotificationBatch = async (
			events: IIntercomEvent[],
			notificationAppId?: string,
		) => {
			for (const event of events) {
				const notification = event.payload as INotificationEvent;
				const targetUserSub = notification.target_user_sub?.trim();
				const normalizedTargetUserSub =
					targetUserSub && targetUserSub !== LOCAL_EXECUTION_SUB
						? targetUserSub
						: undefined;
				const isCurrentUserTarget =
					!normalizedTargetUserSub || normalizedTargetUserSub === userId;

				if (!isCurrentUserTarget) {
					continue;
				}

				// Store locally for immediate/offline desktop history only.
				// Remote persistence is owned by notification nodes.
				await storeNotification({
					title: notification.title,
					description: notification.description,
					icon: notification.icon,
					link: notification.link,
					appIdOverride: notificationAppId,
					sourceRunId: notification.source_run_id,
					sourceNodeId: notification.source_node_id,
				});

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
		};

		const handleWindowNotification = (event: globalThis.Event) => {
			const detail = (event as CustomEvent<FlowNotificationBatchDetail>).detail;
			if (!detail) {
				return;
			}

			void handleNotificationBatch(detail.events, detail.appId ?? appId);
		};

		const unlistenFn = listen(
			"flow_notification",
			async (events: Event<IIntercomEvent[]>) => {
				await handleNotificationBatch(events.payload, appId);
			},
		);

		subscriptions.push(unlistenFn);
		window.addEventListener(
			FLOW_NOTIFICATION_EVENT,
			handleWindowNotification as EventListener,
		);

		return () => {
			window.removeEventListener(
				FLOW_NOTIFICATION_EVENT,
				handleWindowNotification as EventListener,
			);
			(async () => {
				for await (const subscription of subscriptions) {
					if (subscription) subscription();
				}
			})();
		};
	}, [userId, appId, queryClient]);

	return null;
}

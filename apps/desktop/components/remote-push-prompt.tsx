"use client";

import {
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	useBackend,
	useHub,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { BellRing } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import {
	REMOTE_PUSH_REGISTRATION_EVENT,
	RemotePushPermissionDeniedError,
	type RemotePushPromptReason,
	canUseRemotePushForPlatform,
	declineRemotePushPrompt,
	detectPushPlatform,
	enableRemotePush,
	getPushDeviceId,
	getRemotePushPermission,
	hasDeclinedRemotePushPrompt,
	isRemotePushPreferenceEnabled,
	loadRemotePushPlugin,
	openNotificationSettings,
	remotePushPromptReason,
} from "../lib/remote-push";
import type { TauriBackend } from "./tauri-provider";

function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

function isMobilePushPlatform(): boolean {
	const platform = detectPushPlatform();
	return platform === "IOS" || platform === "ANDROID";
}

/**
 * Asks once per launch to turn mobile push back on when it is off, until the
 * user answers "No", which is remembered on this device.
 */
export default function RemotePushPrompt() {
	const { t } = useTranslation("common");
	const auth = useAuth();
	const backend = useBackend();
	const hub = useHub();
	const authContext = (backend as TauriBackend).auth ?? auth;
	const pushConfig = hub.hub?.push_notifications;
	const [reason, setReason] = useState<RemotePushPromptReason | null>(null);
	const [busy, setBusy] = useState(false);
	const launchPermission = useRef<Promise<boolean | null> | null>(null);
	const evaluated = useRef(false);
	const awaitingSettings = useRef(false);

	const eligible =
		Boolean(authContext.isAuthenticated) &&
		!authContext.isLoading &&
		Boolean(backend.profile) &&
		isMobilePushPlatform() &&
		canUseRemotePushForPlatform(pushConfig, detectPushPlatform());

	// Read the permission before startup's own request can be answered, so a
	// refusal given in this launch's system dialog is not asked about again.
	useEffect(() => {
		if (!isMobilePushPlatform() || hasDeclinedRemotePushPrompt()) return;
		launchPermission.current = getRemotePushPermission().catch(() => null);
	}, []);

	useEffect(() => {
		if (!eligible || evaluated.current || !launchPermission.current) return;
		let cancelled = false;
		const stale = () => cancelled || evaluated.current;
		const fetchStatus = async () =>
			backend.userState.getPushTargetStatus(await getPushDeviceId());

		// Only the server status can rule push out for good; without it a "no
		// reason" result waits for the next registration.
		const settle = (next: RemotePushPromptReason | null, decisive: boolean) => {
			if (!next && !decisive) return;
			evaluated.current = true;
			if (next) setReason(next);
		};

		const evaluate = async (withServerStatus: boolean) => {
			if (stale() || hasDeclinedRemotePushPrompt()) return;
			const [permissionAtLaunch, api, status] = await Promise.all([
				launchPermission.current,
				loadRemotePushPlugin(),
				withServerStatus ? fetchStatus() : null,
			]);
			if (stale() || !api) return;
			settle(
				remotePushPromptReason({
					permissionAtLaunch,
					preferenceEnabled: isRemotePushPreferenceEnabled(),
					status,
				}),
				withServerStatus,
			);
		};

		const onRegistered = () => {
			evaluate(true).catch(() => {});
		};
		window.addEventListener(REMOTE_PUSH_REGISTRATION_EVENT, onRegistered);
		evaluate(false).catch(() => {});
		return () => {
			cancelled = true;
			window.removeEventListener(REMOTE_PUSH_REGISTRATION_EVENT, onRegistered);
		};
	}, [eligible, backend.userState]);

	const turnOn = useCallback(async () => {
		const deviceId = await getPushDeviceId();
		await enableRemotePush({
			userState: backend.userState,
			pushConfig,
			deviceId,
			source: "prompt",
		});
		toast.success(t("pushNotificationsEnabled", "Push notifications enabled"));
	}, [backend.userState, pushConfig, t]);

	const reportFailure = useCallback(
		(error: unknown) => {
			toast.error(
				t(
					"failedToUpdatePushNotificationsVal",
					"Failed to update push notifications: {{val}}",
					{ val: errorMessage(error) },
				),
			);
		},
		[t],
	);

	useEffect(() => {
		const onReturn = () => {
			if (document.visibilityState !== "visible" || !awaitingSettings.current)
				return;
			getRemotePushPermission()
				.then((granted) => {
					if (granted !== true || !awaitingSettings.current) return;
					awaitingSettings.current = false;
					return turnOn();
				})
				.catch(reportFailure);
		};
		document.addEventListener("visibilitychange", onReturn);
		return () => document.removeEventListener("visibilitychange", onReturn);
	}, [turnOn, reportFailure]);

	const accept = useCallback(async () => {
		setBusy(true);
		try {
			await turnOn();
		} catch (error) {
			if (error instanceof RemotePushPermissionDeniedError) {
				awaitingSettings.current = true;
				await openNotificationSettings().catch(reportFailure);
			} else {
				reportFailure(error);
			}
		} finally {
			setBusy(false);
			setReason(null);
		}
	}, [turnOn, reportFailure]);

	const decline = useCallback(() => {
		declineRemotePushPrompt();
		setReason(null);
	}, []);

	return (
		<Dialog
			open={reason !== null}
			onOpenChange={(open) => {
				if (!open && !busy) setReason(null);
			}}
		>
			<DialogContent
				className="max-w-sm"
				showCloseButton={false}
				onInteractOutside={(event) => event.preventDefault()}
			>
				<DialogHeader>
					<div className="flex items-center gap-2">
						<BellRing className="h-5 w-5 text-primary" />
						<DialogTitle>
							{t("pushPromptTitle", "Turn on push notifications?")}
						</DialogTitle>
					</div>
					<DialogDescription>
						{reason === "permission"
							? t(
									"pushPromptPermissionDescription",
									"Notifications for Flow-Like are blocked on this device, so you won't hear from your bots and flows while the app is closed. You can allow them in Settings.",
								)
							: t(
									"pushPromptDisabledDescription",
									"Push notifications are off on this device, so you won't hear from your bots and flows while the app is closed.",
								)}
					</DialogDescription>
				</DialogHeader>
				<DialogFooter className="gap-2">
					<Button variant="outline" disabled={busy} onClick={decline}>
						{t("pushPromptDecline", "No, don't ask again")}
					</Button>
					<Button disabled={busy} onClick={() => void accept()}>
						{reason === "permission"
							? t("pushPromptOpenSettings", "Open Settings")
							: t("pushPromptTurnOn", "Turn on")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

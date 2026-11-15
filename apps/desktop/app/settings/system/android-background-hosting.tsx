"use client";

import { Button, Switch } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

interface BackgroundHostingStatus {
	supported: boolean;
	enabled: boolean;
	running: boolean;
	batteryOptimized: boolean;
	notificationsEnabled: boolean;
}

export function AndroidBackgroundHosting() {
	const { t } = useTranslation("common");
	const [status, setStatus] = useState<BackgroundHostingStatus | null>(null);
	const [pending, setPending] = useState(false);
	const [loadFailed, setLoadFailed] = useState(false);
	const active = useRef(false);
	const changing = useRef(false);
	const requestVersion = useRef(0);

	const refresh = useCallback(async () => {
		if (!active.current || changing.current) return;
		const version = ++requestVersion.current;
		try {
			const next = await invoke<BackgroundHostingStatus>(
				"native_background_hosting_status",
			);
			if (!active.current || version !== requestVersion.current) return;
			setStatus(next);
			setLoadFailed(false);
		} catch (error) {
			if (!active.current || version !== requestVersion.current) return;
			setLoadFailed(true);
			toast.error(String(error), { id: "background-hosting-status" });
		}
	}, []);

	useEffect(() => {
		active.current = true;
		void refresh();
		const refreshWhenVisible = () => {
			if (document.visibilityState !== "hidden") void refresh();
		};
		const timer = window.setInterval(refreshWhenVisible, 15_000);
		window.addEventListener("focus", refreshWhenVisible);
		document.addEventListener("visibilitychange", refreshWhenVisible);
		return () => {
			active.current = false;
			++requestVersion.current;
			window.clearInterval(timer);
			window.removeEventListener("focus", refreshWhenVisible);
			document.removeEventListener("visibilitychange", refreshWhenVisible);
		};
	}, [refresh]);

	const change = async (enabled: boolean) => {
		if (changing.current) return;
		changing.current = true;
		++requestVersion.current;
		setPending(true);
		try {
			const next = await invoke<BackgroundHostingStatus>(
				"native_set_background_hosting",
				{ enabled },
			);
			if (active.current) setStatus(next);
		} catch (error) {
			if (active.current) toast.error(String(error));
		} finally {
			changing.current = false;
			if (active.current) {
				setPending(false);
				void refresh();
			}
		}
	};

	const openSettings = async (
		command:
			| "native_open_background_hosting_battery_settings"
			| "native_open_background_hosting_notification_settings",
	) => {
		try {
			await invoke(command);
		} catch (error) {
			if (active.current) toast.error(String(error));
		}
	};

	return (
		<section
			aria-labelledby="android-background-hosting-title"
			className="w-full max-w-screen-2xl space-y-4 rounded-lg border p-4"
		>
			<div className="flex items-start justify-between gap-4">
				<div className="space-y-1">
					<h2 id="android-background-hosting-title" className="font-medium">
						{t(
							"androidBackgroundHostingTitle",
							"Keep bots and Events active in the background",
						)}
					</h2>
					<p
						id="android-background-hosting-description"
						className="text-sm text-muted-foreground"
					>
						{t(
							"androidBackgroundHostingDescription",
							"Keep bots and Events on this device running while you use other apps or lock the screen. Flow-Like shows a persistent notification and may use more battery.",
						)}
					</p>
				</div>
				<Switch
					aria-labelledby="android-background-hosting-title"
					aria-describedby="android-background-hosting-description"
					checked={status?.enabled === true}
					disabled={!status?.supported || pending}
					onCheckedChange={change}
					className="shrink-0"
				/>
			</div>
			<div className="space-y-2 text-sm text-muted-foreground">
				<p>
					{t(
						"androidBackgroundHostingSession",
						"Off by default for each app session. Fully closing the app, removing it from recent apps, or restarting the device ends background hosting.",
					)}
				</p>
				<p>
					{t(
						"androidBackgroundHostingDisable",
						"Turning this off lets Android pause the app in the background. Bots and Events already running are not cancelled.",
					)}
				</p>
				<output className="block font-medium text-foreground">
					{loadFailed
						? t(
								"androidBackgroundHostingStatusFailed",
								"Could not refresh background hosting status.",
							)
						: status
							? !status.supported
								? t(
										"androidBackgroundHostingUnsupported",
										"Background hosting is unavailable on this device.",
									)
								: status.running
									? t(
											"androidBackgroundHostingRunning",
											"Background hosting is active.",
										)
									: status.enabled
										? t(
												"androidBackgroundHostingStarting",
												"Background hosting is starting.",
											)
										: t(
												"androidBackgroundHostingOff",
												"Background hosting is off.",
											)
							: t(
									"androidBackgroundHostingLoading",
									"Checking background hosting…",
								)}
				</output>
			</div>
			{loadFailed && (
				<Button variant="outline" size="sm" onClick={() => void refresh()}>
					{t("androidBackgroundHostingRetry", "Retry status check")}
				</Button>
			)}
			{status?.supported && !status.notificationsEnabled && (
				<div className="space-y-2 rounded-md bg-muted p-3">
					<p className="text-sm">
						{t(
							"androidBackgroundHostingNotificationsDescription",
							"Allow notifications in Android settings before enabling background hosting.",
						)}
					</p>
					<Button
						variant="outline"
						size="sm"
						disabled={pending}
						onClick={() =>
							void openSettings(
								"native_open_background_hosting_notification_settings",
							)
						}
					>
						{t(
							"androidBackgroundHostingNotificationSettings",
							"Enable notifications",
						)}
					</Button>
				</div>
			)}
			{status?.supported && status.batteryOptimized && (
				<div className="space-y-2 rounded-md bg-muted p-3">
					<p className="text-sm">
						{t(
							"androidBackgroundHostingBatteryDescription",
							"Android is optimizing battery use for Flow-Like and may pause network connections while the screen is off. Review the battery settings to allow background activity.",
						)}
					</p>
					<Button
						variant="outline"
						size="sm"
						disabled={pending}
						onClick={() =>
							void openSettings(
								"native_open_background_hosting_battery_settings",
							)
						}
					>
						{t(
							"androidBackgroundHostingBatterySettings",
							"Open battery settings",
						)}
					</Button>
				</div>
			)}
		</section>
	);
}

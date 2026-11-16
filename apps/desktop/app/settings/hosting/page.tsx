"use client";

import { Switch } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeft } from "lucide-react";
import Link from "next/link";
import { useCallback, useEffect, useId, useState } from "react";
import { toast } from "sonner";
import {
	type BackgroundHostingPlatform,
	getBackgroundHostingPlatform,
} from "../../../lib/platform";
import { getStartAtLogin, setStartAtLogin } from "../../../lib/start-at-login";
import { AndroidBackgroundHosting } from "./android-background-hosting";

interface IBackgroundSetting {
	readonly name: string;
	readonly load: () => Promise<boolean>;
	readonly save: (enabled: boolean) => Promise<boolean>;
}

const KEEP_BOTS_RUNNING: IBackgroundSetting = {
	name: "keep-alive",
	load: () => invoke<boolean>("get_keep_bots_running"),
	save: (enabled) => invoke<boolean>("set_keep_bots_running", { enabled }),
};

const START_AT_LOGIN: IBackgroundSetting = {
	name: "start at login",
	load: getStartAtLogin,
	save: setStartAtLogin,
};

interface IBackgroundToggleProps {
	readonly setting: IBackgroundSetting;
	readonly title: string;
	readonly description: string;
}

function BackgroundToggle({
	setting,
	title,
	description,
}: IBackgroundToggleProps) {
	const [enabled, setEnabled] = useState<boolean | undefined>();
	const id = useId();

	useEffect(() => {
		setting
			.load()
			.then(setEnabled)
			.catch((error) => console.warn(`Failed to load ${setting.name}:`, error));
	}, [setting]);

	const change = useCallback(
		async (next: boolean) => {
			try {
				setEnabled(await setting.save(next));
			} catch (error) {
				console.warn(`Failed to update ${setting.name}:`, error);
				toast.error(String(error));
			}
		},
		[setting],
	);

	return (
		<div className="flex w-full max-w-screen-2xl items-center justify-between gap-4 rounded-lg border p-4">
			<div className="space-y-1">
				<label htmlFor={id} className="font-medium">
					{title}
				</label>
				<p className="text-sm text-muted-foreground">{description}</p>
			</div>
			<Switch
				id={id}
				checked={enabled === true}
				disabled={enabled === undefined}
				onCheckedChange={change}
			/>
		</div>
	);
}

function DesktopBackgroundHosting() {
	const { t } = useTranslation("common");
	return (
		<div className="flex w-full max-w-screen-2xl flex-col gap-4">
			<BackgroundToggle
				setting={KEEP_BOTS_RUNNING}
				title={t(
					"keepBotsRunningWhenWindowClosed",
					"Keep bots running when the window is closed",
				)}
				description={t(
					"keepBotsRunningDescription",
					"The app stays in the tray so your bots keep answering. Quit it from the tray icon or the app menu to stop them.",
				)}
			/>
			<BackgroundToggle
				setting={START_AT_LOGIN}
				title={t("startAtLogin", "Start Flow-Like at login")}
				description={t(
					"startAtLoginDescription",
					"Flow-Like opens without a window when you log in to this computer, so bots hosted here keep answering after a restart. Open the window from the tray icon or the Dock. It starts this way even when no bot runs here.",
				)}
			/>
		</div>
	);
}

export default function HostingSettingsPage() {
	const { t } = useTranslation("common");
	const [platform, setPlatform] = useState<BackgroundHostingPlatform | null>();

	useEffect(() => setPlatform(getBackgroundHostingPlatform()), []);

	return (
		<main className="flex min-h-0 w-full flex-1 flex-col overflow-y-auto">
			<div className="container mx-auto flex w-full max-w-3xl flex-col gap-6 px-4 py-4">
				<Link
					href="/settings"
					className="inline-flex w-fit items-center gap-2 text-sm text-muted-foreground hover:text-foreground"
				>
					<ArrowLeft className="size-4" aria-hidden />
					{t("settings", "Settings")}
				</Link>
				<div className="space-y-2">
					<h1 className="text-3xl font-bold tracking-tight">
						{t("backgroundHosting", "Background hosting")}
					</h1>
					<p className="text-muted-foreground">
						{t(
							"backgroundHostingPageDescription",
							"Choose how bots and Events hosted on this device keep running when you leave the app.",
						)}
					</p>
				</div>
				{platform === "android" && <AndroidBackgroundHosting />}
				{platform === "desktop" && <DesktopBackgroundHosting />}
				{platform === null && (
					<p className="rounded-lg border p-4 text-sm text-muted-foreground">
						{t(
							"backgroundHostingUnavailable",
							"Background hosting is available in the Android and desktop apps.",
						)}
					</p>
				)}
			</div>
		</main>
	);
}

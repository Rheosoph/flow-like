"use client";
import { Switch, type UseQueryResult } from "@flow-like/flow-like-ui";
import { humanFileSize } from "@flow-like/flow-like-ui/lib/utils";
import type { ISystemInfo } from "@flow-like/flow-like-ui/types";
import { Trans, useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { useTauriInvoke } from "../../../components/useInvoke";
import { isMobileDevice, isTauriRuntime } from "../../../lib/platform";
import { getStartAtLogin, setStartAtLogin } from "../../../lib/start-at-login";

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
				<div className="font-medium">{title}</div>
				<p className="text-sm text-muted-foreground">{description}</p>
			</div>
			<Switch
				checked={enabled === true}
				disabled={enabled === undefined}
				onCheckedChange={change}
			/>
		</div>
	);
}

function BackgroundSettings() {
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

export default function SettingsPage() {
	const { t } = useTranslation("common");
	const systemInfo: UseQueryResult<ISystemInfo> = useTauriInvoke(
		"get_system_info",
		{},
	);

	return (
		<main className="justify-start flex flex-col items-center w-full pr-4 flex-1 min-h-0">
			<div className="flex flex-row items-center justify-between w-full max-w-screen-2xl">
				<h1 className="scroll-m-20 text-4xl font-extrabold tracking-tight lg:text-5xl">
					{t("systemInfo", "System Info")}
				</h1>
			</div>
			<br />
			{isTauriRuntime() && !isMobileDevice() && <BackgroundSettings />}
			<br />
			<p className="w-full">
				<Trans i18nKey="bcoresb">
					<b>Cores</b>:
				</Trans>
				{systemInfo.data?.cores}
			</p>
			<p className="w-full">
				<Trans i18nKey="bvramb">
					<b>VRAM</b>:
				</Trans>
				{humanFileSize(systemInfo.data?.vram ?? 0)}
			</p>
			<p className="w-full">
				<Trans i18nKey="bramb">
					<b>RAM</b>:
				</Trans>
				{humanFileSize(systemInfo.data?.ram ?? 0)}
			</p>
		</main>
	);
}

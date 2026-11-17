"use client";

import { Switch } from "@flow-like/flow-like-ui";
import { BotAvatar } from "@flow-like/flow-like-ui/components/bots";
import { useHiddenBots } from "@flow-like/flow-like-ui/components/bots-tab/use-bot-visibility";
import { useBotLabels, useBots } from "@flow-like/flow-like-ui/hooks/use-bots";
import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useId, useMemo, useState } from "react";
import { toast } from "sonner";
import { notchCommands } from "../../lib/bot-notch/commands";
import { notchBotsOf } from "../../lib/bot-notch/model";
import {
	NOTCH_EVENTS,
	type NotchBot,
	type NotchSettings,
} from "../../lib/bot-notch/protocol";
import { useTauriEvent } from "./use-notch-data";

/** Whether the bot notch shows, and which of the bots that can use this computer sit in it. */
export function NotchSettingsSection() {
	const { t } = useTranslation("bots");
	const id = useId();
	const [settings, setSettings] = useState<NotchSettings | null>(null);
	const bots = useBots();
	const profileHidden = useHiddenBots();
	const botLabels = useBotLabels();

	useEffect(() => {
		notchCommands
			.settings()
			.then(setSettings)
			.catch((error) =>
				console.warn("[BotNotch] The notch settings did not load:", error),
			);
	}, []);
	useTauriEvent<NotchSettings>(NOTCH_EVENTS.settings, setSettings);

	const eligible = useMemo(
		() =>
			notchBotsOf(
				bots.data ?? [],
				profileHidden.keys,
				(bot) => botLabels(bot).name || bot.id,
			),
		[bots.data, profileHidden.keys, botLabels],
	);
	const hidden = useMemo(
		() => new Set(settings?.hiddenBots ?? []),
		[settings?.hiddenBots],
	);

	const fail = useCallback((error: unknown) => {
		console.warn("[BotNotch] The notch setting was not saved:", error);
		toast.error(String(error));
	}, []);

	const setEnabled = (enabled: boolean) =>
		notchCommands.setEnabled(enabled).catch(fail);

	const setShown = (bot: NotchBot, shown: boolean) => {
		const rest = (settings?.hiddenBots ?? []).filter((key) => key !== bot.key);
		void notchCommands
			.setHiddenBots(shown ? rest : [...rest, bot.key])
			.catch(fail);
	};

	const reach = (bot: NotchBot) =>
		bot.computer && bot.browser
			? t("notch.settings.computerAndBrowser", "Apps and browser tabs")
			: bot.computer
				? t("notch.settings.computer", "Apps")
				: t("notch.settings.browser", "Browser tabs");

	return (
		<section className="flex w-full max-w-screen-2xl flex-col gap-3 rounded-lg border p-4">
			<div className="flex items-center justify-between gap-4">
				<div className="space-y-1">
					<label htmlFor={id} className="font-medium">
						{t("notch.settings.show", "Show the bot notch")}
					</label>
					<p className="text-sm text-muted-foreground">
						{t(
							"notch.settings.description",
							"Bots that can use this computer or a browser wait at the top of your screen. Drag one onto a window or a browser tab and tell it what to do there.",
						)}
					</p>
				</div>
				<Switch
					id={id}
					checked={settings?.enabled === true}
					disabled={!settings}
					onCheckedChange={setEnabled}
				/>
			</div>
			{settings?.enabled &&
				(eligible.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						{t(
							"notch.settings.none",
							"No bot can use this computer or a browser yet. Switch on Computer or Browser in a bot's settings and it joins the notch.",
						)}
					</p>
				) : (
					<ul className="divide-y rounded-md border">
						{eligible.map((bot) => (
							<li key={bot.id} className="flex items-center gap-3 px-3 py-2">
								<BotAvatar
									appearance={bot.appearance}
									size={28}
									animated={false}
									decorative
								/>
								<span className="min-w-0 flex-1 truncate text-sm">
									{bot.name}
								</span>
								<span className="text-xs text-muted-foreground">
									{reach(bot)}
								</span>
								<Switch
									checked={!hidden.has(bot.key)}
									onCheckedChange={(shown) => setShown(bot, shown)}
									aria-label={t(
										"notch.settings.toggle",
										"Show {{name}} in the notch",
										{
											name: bot.name,
										},
									)}
								/>
							</li>
						))}
					</ul>
				))}
		</section>
	);
}

"use client";

import { useTranslation } from "@flow-like/locales";
import { BellIcon, BellOffIcon } from "lucide-react";
import { toast } from "sonner";
import { useUpdateBot } from "../../hooks/use-bots";
import type { IBot } from "../../state/backend-state/bot-state";
import { Switch } from "../ui/switch";

/** Whether the owner switched this bot's notifications off. */
export function botNotificationsMuted(bot: Pick<IBot, "appearance">): boolean {
	return bot.appearance.mute_notifications === true;
}

/** The appearance with notifications muted or on; on leaves no key in the saved JSON. */
export function withNotificationsMuted(
	appearance: IBot["appearance"],
	muted: boolean,
): IBot["appearance"] {
	return { ...appearance, mute_notifications: muted ? true : undefined };
}

/** The per-bot switch for notifications about what the bot does on its own. */
export function BotNotifications({ bot }: Readonly<{ bot: IBot }>) {
	const { t } = useTranslation("bots");
	const { mutateAsync: updateBot, isPending } = useUpdateBot();
	const muted = botNotificationsMuted(bot);
	const Icon = muted ? BellOffIcon : BellIcon;
	const toggle = async (enabled: boolean) => {
		try {
			await updateBot({
				botId: bot.id,
				request: {
					appearance: withNotificationsMuted(bot.appearance, !enabled),
				},
			});
		} catch (cause) {
			console.warn("Saving the bot's notification setting failed", cause);
			toast.error(
				t(
					"panel.notifications.failed",
					"The notification setting could not be saved",
				),
			);
		}
	};
	return (
		<section
			data-slot="bot-notifications"
			className="flex items-start gap-3 rounded-xl border bg-card p-3"
		>
			<span className="grid size-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
				<Icon className="size-4" aria-hidden />
			</span>
			<div className="min-w-0 flex-1">
				<p className="text-xs text-muted-foreground" id={`notify-${bot.id}`}>
					{t("panel.notifications.label", "Notifications")}
				</p>
				<p className="text-sm font-medium">
					{muted
						? t("panel.notifications.off", "Muted")
						: t("panel.notifications.on", "On")}
				</p>
				<p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
					{t(
						"panel.notifications.hint",
						"When this bot messages you without being asked, or sends you a notification, you get a notification under its name.",
					)}
				</p>
			</div>
			<Switch
				checked={!muted}
				disabled={isPending}
				aria-labelledby={`notify-${bot.id}`}
				onCheckedChange={(enabled) => void toggle(enabled)}
			/>
		</section>
	);
}

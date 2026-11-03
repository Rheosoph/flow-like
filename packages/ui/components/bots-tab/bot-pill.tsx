"use client";

import { useTranslation } from "@flow-like/locales";
import { HardDriveIcon, Loader2Icon, ShieldQuestionIcon } from "lucide-react";
import { botPresence, useBotPresence } from "../../hooks/use-bot-presence";
import { useRunningBotTurn } from "../../hooks/use-bot-turn";
import {
	type DraftRelay,
	GlobalChatBotAvatar,
} from "../global-chat/chat-bot-avatar";
import type { GlobalChatBotView } from "../global-chat/use-global-chat-bot";
import { useBotPresenceText, useBotStatusText } from "./bot-labels";
import { subjectSystem } from "./bot-panel-tabs";
import { StatusDot } from "./crew-section";

/**
 * A bot working on a turn shows a spinner and a bot on this device the drive; every other a dot,
 * lit while it is online (the built-in and cloud bots always are).
 */
function BotPresence({ view }: Readonly<{ view: GlobalChatBotView }>) {
	const { t } = useTranslation("bots");
	const statusText = useBotStatusText();
	const presenceText = useBotPresenceText();
	const { bot } = view;
	const hosted = Boolean(
		bot && !bot.local && !bot.system && bot.location !== "server",
	);
	const reports = useBotPresence(hosted).data;
	const { turn, waiting } = useRunningBotTurn(bot?.id);
	if (turn) {
		return (
			<>
				<Loader2Icon
					className="size-3.5 shrink-0 animate-spin text-muted-foreground"
					aria-hidden
				/>
				<span className="sr-only">{t("panel.working", "Working…")}</span>
			</>
		);
	}
	if (waiting) {
		return (
			<>
				<ShieldQuestionIcon
					className="size-3.5 shrink-0 text-primary"
					aria-hidden
				/>
				<span className="sr-only">
					{t("panel.waitingForYou", "Waiting for you")}
				</span>
			</>
		);
	}
	if (view.bot?.local) {
		return (
			<>
				<HardDriveIcon
					className="size-3.5 shrink-0 text-muted-foreground"
					aria-hidden
				/>
				<span className="sr-only">{t("local.badge", "On this device")}</span>
			</>
		);
	}
	const presence = botPresence(bot, reports);
	if (presence && !subjectSystem(view)) {
		return (
			<>
				<StatusDot muted={presence.state !== "online"} />
				<span className="sr-only">
					{hosted
						? presenceText(presence.state)
						: statusText(null, bot?.location).label}
				</span>
			</>
		);
	}
	const status = statusText(subjectSystem(view), view.bot?.location);
	return (
		<>
			<StatusDot muted={!status.ready} />
			<span className="sr-only">{status.label}</span>
		</>
	);
}

/** The bot a chat talks to, floating at the top of it; opens and closes the bot's panel. */
export function BotPill({
	view,
	relay,
	expanded,
	controls,
	onToggle,
}: Readonly<{
	view: GlobalChatBotView;
	relay: DraftRelay;
	expanded: boolean;
	/** The panel's id while it is open. */
	controls?: string;
	onToggle: () => void;
}>) {
	const { t } = useTranslation("bots");
	const hint = expanded
		? t("panel.hide", "Hide {{name}}'s details", { name: view.name })
		: t("panel.show", "Show {{name}}'s details", { name: view.name });
	return (
		<button
			type="button"
			data-slot="bot-pill"
			aria-expanded={expanded}
			aria-controls={expanded ? controls : undefined}
			title={hint}
			onClick={onToggle}
			className="flex h-9 max-w-64 min-w-0 items-center gap-2 rounded-full border border-border/60 bg-background/85 py-1 pr-3 pl-1 text-sm font-medium text-foreground shadow-sm backdrop-blur-md outline-none transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring aria-expanded:bg-muted"
		>
			<span className="flex size-7 shrink-0 items-end justify-center overflow-hidden rounded-full bg-muted">
				<GlobalChatBotAvatar
					relay={relay}
					appearance={view.appearance}
					size={28}
					className="[&_.fl-bot-badges]:hidden"
				/>
			</span>
			<span data-slot="bot-pill-name" className="min-w-0 truncate">
				{view.name}
			</span>
			<BotPresence view={view} />
		</button>
	);
}

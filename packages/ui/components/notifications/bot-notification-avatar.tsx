"use client";

import { useBot } from "../../hooks/use-bots";
import { BotAvatar } from "../bots/bot-avatar";

const BOT_CHAT_LINK = "/bots?bot=";

/** The bot a notification came from: its chat is the link, so the link names it. */
export function notificationBotId(link?: string | null): string | null {
	if (!link?.startsWith(BOT_CHAT_LINK)) return null;
	const id = link.slice(BOT_CHAT_LINK.length).split(/[&#]/)[0];
	return id ? id : null;
}

/** The bot's own avatar in place of an app icon. */
export function BotNotificationAvatar({
	botId,
	className,
}: Readonly<{ botId: string; className?: string }>) {
	const { data: bot } = useBot(botId);
	return (
		<BotAvatar
			appearance={bot?.appearance}
			size={24}
			animated={false}
			decorative
			className={className}
		/>
	);
}

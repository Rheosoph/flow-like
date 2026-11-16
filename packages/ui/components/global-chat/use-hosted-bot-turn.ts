"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { botPresence, useBotPresence } from "../../hooks/use-bot-presence";
import { useBotTurnActivity } from "../../hooks/use-conversation-activity";
import { useBotHistoryStore } from "../../state/global-chat/bot-history-store";
import { activeBotTarget } from "../../state/global-chat/bot-inbox";
import type { GlobalChatBotView } from "./use-global-chat-bot";

/**
 * What a bot's own host does with the messages this chat left to it, as the line the chat shows
 * while it waits; null while nothing is pending or the chat runs the bot itself.
 */
export function useHostedBotTurnText(
	sessionId: string,
	view: GlobalChatBotView,
	hosted: boolean,
): string | null {
	const { t } = useTranslation("bots");
	const botId = hosted ? (view.bot?.id ?? null) : null;
	const cursor = useBotHistoryStore((state) =>
		botId ? (state.views[botId]?.cursor ?? undefined) : undefined,
	);
	const conversationId = botId
		? (activeBotTarget(sessionId)?.conversationId ?? null)
		: null;
	const botIds = useMemo(() => new Set<string>(botId ? [botId] : []), [botId]);
	const [activity] = useBotTurnActivity(conversationId, botIds, cursor);
	const presence = botPresence(view.bot, useBotPresence(hosted).data);
	if (!activity) return null;
	const name = view.name;
	if (activity.status === "running") {
		return activity.statusText
			? t("crew.activity.runningStatus", "{{name}} · {{status}}", {
					name,
					status: activity.statusText,
				})
			: t("crew.activity.running", "{{name}} is answering", { name });
	}
	if (activity.status === "waiting") {
		return t("crew.activity.waiting", "{{name}} is waiting for approval", {
			name,
		});
	}
	if (presence?.state === "online") {
		return t("crew.activity.queued", "Waiting for {{name}}", { name });
	}
	return view.bot?.location === "device"
		? t(
				"crew.activity.queuedDeviceOffline",
				"{{name}} answers once its device is online",
				{ name },
			)
		: t(
				"crew.activity.queuedDesktopOffline",
				"{{name}} answers once your desktop app is online",
				{ name },
			);
}

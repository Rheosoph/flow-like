"use client";

import { useQuery } from "@tanstack/react-query";
import { useBackend, useSignedIn } from "../state/backend-state";
import type { IBot, IBotPresence } from "../state/backend-state/bot-state";
import { botKeys } from "./use-bots";

export const BOT_PRESENCE_POLL_MS = 60_000;

/**
 * Where a bot stands: server and built-in bots are always there, a bot on this computer has no
 * presence to show (null), and a desktop or device bot is whatever its hosts last reported.
 */
export function botPresence(
	bot: IBot | undefined,
	reports: Readonly<Record<string, IBotPresence>> | undefined,
): IBotPresence | null {
	if (!bot || bot.local) return null;
	if (bot.system || bot.location === "server") return { state: "online" };
	return reports?.[bot.id] ?? { state: "unknown" };
}

/**
 * The hub's presence of the account's bots, read again every minute and when the window gets
 * focus again. It only runs while something that shows it is mounted.
 */
export function useBotPresence(enabled = true) {
	const state = useBackend().botState;
	const signedIn = useSignedIn();
	return useQuery({
		queryKey: botKeys.presence(),
		queryFn: () => (state?.getPresence ? state.getPresence() : {}),
		enabled: enabled && signedIn && typeof state?.getPresence === "function",
		staleTime: 10_000,
		refetchInterval: BOT_PRESENCE_POLL_MS,
		refetchOnWindowFocus: "always",
	});
}

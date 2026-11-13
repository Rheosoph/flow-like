"use client";

import { useQuery } from "@tanstack/react-query";
import {
	type BotSecretSealing,
	canSealBotSecret,
} from "../lib/bot-secret-sealing";
import { useBackend } from "../state/backend-state";
import { useBot } from "./use-bots";
import { inboxKeys } from "./use-inbox";

const KEYS_STALE_MS = 60_000;

/**
 * Whether a secret of `botId` can be sealed to the bot's own key here: an account bot with
 * end-to-end encryption whose host published a key, and an app that can seal.
 */
export function useBotSecretSealing(botId?: string | null): BotSecretSealing {
	const inbox = useBackend().inboxState;
	const sealsHere = Boolean(botId && inbox?.sealBotSecret);
	const bot = useBot(sealsHere ? botId : null);
	const encrypted = Boolean(
		bot.data && !bot.data.local && bot.data.capabilities.includes("encryption"),
	);
	const enabled = sealsHere && encrypted;
	const keys = useQuery({
		queryKey: [...inboxKeys.all, "secret-keys", botId ?? ""],
		queryFn: () => inbox?.getKeys([botId as string]) ?? [],
		enabled,
		staleTime: KEYS_STALE_MS,
	});
	if (!sealsHere) return "unavailable";
	if (bot.isLoading) return "checking";
	if (!enabled) return "unavailable";
	if (keys.isLoading) return "checking";
	return canSealBotSecret(bot.data, keys.data) ? "available" : "unavailable";
}

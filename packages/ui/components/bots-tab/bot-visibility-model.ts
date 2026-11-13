import type { BotSystemKey } from "../../state/backend-state/bot-state";

export const NO_HIDDEN_BOTS: ReadonlySet<string> = new Set();

/**
 * What an entry of a profile's `hidden_bots` names: a built-in bot by its system key, so its
 * account row and its row on this computer hide together, and any other bot by its id.
 */
export function botVisibilityKey(bot: {
	id: string;
	system: BotSystemKey | null;
}): string {
	return bot.system ?? bot.id;
}

/** `hidden` with `key` hidden or shown again; entries for bots this device doesn't list stay. */
export function withBotHidden(
	hidden: readonly string[],
	key: string,
	hide: boolean,
): string[] {
	const rest = hidden.filter((entry) => entry !== key);
	return hide ? [...rest, key] : rest;
}

/** The bots a profile shows, in the order given. */
export function shownBots<
	T extends { id: string; system: BotSystemKey | null },
>(bots: readonly T[], hidden: ReadonlySet<string>): readonly T[] {
	if (hidden.size === 0) return bots;
	return bots.filter((bot) => !hidden.has(botVisibilityKey(bot)));
}

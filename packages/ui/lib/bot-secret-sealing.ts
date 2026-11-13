import type {
	IBot,
	IUpsertBotSecretRequest,
} from "../state/backend-state/bot-state";
import type {
	IInboxKey,
	IInboxState,
} from "../state/backend-state/inbox-state";
import { botSecretKeys } from "./inbox-e2e";

/** Whether a secret form offers sealing; `checking` while the bot or its keys still load. */
export type BotSecretSealing = "available" | "unavailable" | "checking";

/**
 * Sealing needs an account bot with end-to-end encryption whose host published a key. A bot on
 * this computer keeps its secrets on the device anyway.
 */
export function canSealBotSecret(
	bot: Pick<IBot, "id" | "capabilities" | "local"> | undefined,
	keys: readonly IInboxKey[] | undefined,
): boolean {
	if (!bot || bot.local || !bot.capabilities.includes("encryption"))
		return false;
	return botSecretKeys(bot.id, keys ?? []).length > 0;
}

/** Whether the typed value is stored sealed: only where sealing is offered and left on. */
export function sealsSecret(
	sealing: BotSecretSealing | undefined,
	chosen: boolean,
): boolean {
	return sealing === "available" && chosen;
}

/**
 * The request that stores `value`: sealed on this device to the bot's keys, so the server can't
 * read it, or sent for the server to encrypt.
 */
export async function botSecretRequest(
	inbox: Pick<IInboxState, "sealBotSecret"> | undefined,
	botId: string,
	name: string,
	value: string,
	description: string | null | undefined,
	sealed: boolean,
): Promise<IUpsertBotSecretRequest> {
	const meta = description === undefined ? {} : { description };
	if (!sealed) return { value, ...meta };
	if (!inbox?.sealBotSecret)
		throw new Error(
			`Could not seal secret ${name} of bot ${botId}: this app cannot seal to a bot's key`,
		);
	const envelope = await inbox.sealBotSecret(botId, name, value);
	return { sealed: { ...envelope }, ...meta };
}

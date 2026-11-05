/**
 * Every bot keeps one continuous conversation in the global chat, addressed by the bot's id, so the
 * id alone says which bot a conversation, and every turn sent in it, belongs to. Every other
 * conversation is Flux's (FlowPilot), which keeps its own sessions and history.
 */
const BOT_CONVERSATION_PREFIX = "bot:";

/** The Assistant's id while it has no account row: signed out, or where bots can't be stored. */
export const ASSISTANT_BOT_ID = "general";

/** Names Flux in a turn, so the host runs it with its own row (its edits apply) or today's defaults. */
export const FLUX_BOT_ID = "flux";

export function botConversationId(botId: string): string {
	return `${BOT_CONVERSATION_PREFIX}${botId}`;
}

/** The bot a conversation belongs to; null for Flux. */
export function conversationBotId(conversationId: string): string | null {
	if (!conversationId.startsWith(BOT_CONVERSATION_PREFIX)) return null;
	return conversationId.slice(BOT_CONVERSATION_PREFIX.length) || null;
}

export function isBotConversation(conversationId: string): boolean {
	return conversationBotId(conversationId) !== null;
}

/** The desktop `global_chat` argument naming a turn's bot; Flux's turns name Flux. */
export function desktopChatBotArgs(conversationId: string): { botId: string } {
	return { botId: conversationBotId(conversationId) ?? FLUX_BOT_ID };
}

/** The web request body field naming a turn's bot; Flux's turns name Flux. */
export function webChatBotBody(conversationId: string): { bot_id: string } {
	return { bot_id: conversationBotId(conversationId) ?? FLUX_BOT_ID };
}

/** Old → new id of every bot chat moved in this app session (`rebindBotChat`). */
const movedConversations = new Map<string, string>();

/** From now on, whatever is still addressed to `from` (a read in flight, a send mid-upload) lands in `to`. */
export function rememberMovedConversation(from: string, to: string) {
	if (from === to || currentConversationId(to) === from) return;
	movedConversations.set(from, to);
}

/** `from` answers as itself again, as the Assistant's `general` does once no row stands in for it. */
export function forgetMovedConversation(from: string) {
	movedConversations.delete(from);
}

/** The id a conversation answers under now: a moved bot's chat lives on under its new id. */
export function currentConversationId(conversationId: string): string {
	let current = conversationId;
	let next = movedConversations.get(current);
	while (next !== undefined) {
		current = next;
		next = movedConversations.get(current);
	}
	return current;
}

/** The message addressed to its conversation's current id; the same object when that's unchanged. */
export function addressedNow<T extends { sessionId: string }>(message: T): T {
	const sessionId = currentConversationId(message.sessionId);
	return sessionId === message.sessionId ? message : { ...message, sessionId };
}

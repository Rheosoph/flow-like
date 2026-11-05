import { joinContentText } from "../../components/interfaces/chat-default/inline-segments";
import { IRole } from "../../lib/schema/llm/history";
import type { IInboxState } from "../backend-state/inbox-state";
import {
	type BotHistoryTarget,
	type LegacyMessage,
	botConversation,
	forgetBotConversation,
	importLegacyChat,
	loadAround,
	loadJumpList,
	loadLatest,
	loadOlder,
	sendInteractive,
	startNewTopic,
} from "./bot-history";
import { useBotHistoryStore } from "./bot-history-store";
import {
	FLUX_BOT_ID,
	botConversationId,
	conversationBotId,
} from "./global-chat-bots";
import { globalChatDb } from "./global-chat-db";
import { useGlobalChatStore } from "./global-chat-store";

let inbox: IInboxState | null = null;
const targets = new Map<string, BotHistoryTarget>();
const resolving = new Map<string, Promise<BotHistoryTarget | null>>();

/** The inbox the bot chats read their history from; null until the backend is known. */
export function registerBotInbox(next: IInboxState | null) {
	if (inbox === next) return;
	inbox = next;
	targets.clear();
	resolving.clear();
	forgetBotConversation();
}

export function botInbox(): IInboxState | null {
	return inbox;
}

async function legacyMessages(botId: string): Promise<LegacyMessage[]> {
	const rows = await globalChatDb.messages
		.where("sessionId")
		.equals(botConversationId(botId))
		.toArray();
	return rows
		.filter(
			(row) =>
				row.inner.role === IRole.User || row.inner.role === IRole.Assistant,
		)
		.map((row) => ({
			id: row.id,
			role: row.inner.role === IRole.Assistant ? "assistant" : "user",
			text: joinContentText(row.inner.content),
			timestamp: row.timestamp,
		}));
}

async function resolve(botId: string): Promise<BotHistoryTarget | null> {
	const backend = inbox;
	if (!backend) return null;
	try {
		const conversationId = await botConversation(backend, botId);
		const target = { botId, conversationId };
		await importLegacyChat(backend, target, await legacyMessages(botId));
		if (inbox !== backend) return null;
		targets.set(botId, target);
		return target;
	} catch (error) {
		forgetBotConversation(botId);
		console.warn(`[BotChat] ${botId}'s chat stays on this device:`, error);
		return null;
	}
}

/**
 * The bot's inbox conversation once its earlier chat has moved there; null where the bot has no
 * inbox (Flux, an Assistant without a row, a signed-out hub), so the chat keeps its saved sessions.
 */
export function resolveBotTarget(
	botId: string,
): Promise<BotHistoryTarget | null> {
	if (botId === FLUX_BOT_ID) return Promise.resolve(null);
	const known = targets.get(botId);
	if (known) return Promise.resolve(known);
	let pending = resolving.get(botId);
	if (!pending) {
		pending = resolve(botId).finally(() => resolving.delete(botId));
		resolving.set(botId, pending);
	}
	return pending;
}

/** The inbox target of a bot chat that already reads its history from the inbox. */
export function activeBotTarget(
	conversationId: string,
): BotHistoryTarget | null {
	const botId = conversationBotId(conversationId);
	return botId ? (targets.get(botId) ?? null) : null;
}

/**
 * The bot chat's inbox target, resolved now when nothing resolved it yet (the backend changed under
 * an open chat, or the first resolve failed). Await it before saving a message, or the message
 * stays on this device and the bot's later turns never see it.
 */
export async function ensureBotTarget(
	conversationId: string,
): Promise<BotHistoryTarget | null> {
	const known = activeBotTarget(conversationId);
	if (known) return known;
	const botId = conversationBotId(conversationId);
	return botId ? resolveBotTarget(botId) : null;
}

/** The message a run answers in the bot's inbox: the run stores its reply there as the bot. */
export interface InteractiveRef {
	conversationId: string;
	triggerMessageId: string;
	clientRunId: string;
}

/**
 * Writes the user's message to the bot's inbox with the interactive mark, before the run starts, so
 * the turn exists and no host answers it too. Null where the chat has no inbox history or the write
 * failed; the run then goes on without storing anything.
 */
export async function sendInteractiveMessage(
	conversationId: string,
	message: { id: string; text: string },
	clientRunId: string,
): Promise<InteractiveRef | null> {
	const target = await ensureBotTarget(conversationId);
	const backend = inbox;
	if (!target || !backend) return null;
	try {
		await sendInteractive(backend, target, { ...message, clientRunId });
		return {
			conversationId: target.conversationId,
			triggerMessageId: message.id,
			clientRunId,
		};
	} catch (error) {
		console.warn(
			"[BotChat] The message could not be stored in the inbox:",
			error,
		);
		return null;
	}
}

/** The desktop `global_chat` argument naming the message a run answers. */
export function desktopInteractiveArgs(
	ref: InteractiveRef | null,
): { interactive: InteractiveRef } | Record<string, never> {
	return ref ? { interactive: ref } : {};
}

/** The web request field naming the message a run answers. */
export function webInteractiveBody(ref: InteractiveRef | null):
	| {
			interactive: {
				conversation_id: string;
				trigger_message_id: string;
				client_run_id: string;
			};
	  }
	| Record<string, never> {
	return ref
		? {
				interactive: {
					conversation_id: ref.conversationId,
					trigger_message_id: ref.triggerMessageId,
					client_run_id: ref.clientRunId,
				},
			}
		: {};
}

function liveTarget(botId: string): BotHistoryTarget | null {
	return inbox ? (targets.get(botId) ?? null) : null;
}

function chatIsBusy(conversationId: string): boolean {
	const state = useGlobalChatStore.getState();
	return state.activeConversationId !== conversationId || state.isStreaming;
}

/** Puts the part of the bot's history around `seq` on screen; false while a reply streams or another chat is open. */
export async function jumpBotChat(
	botId: string,
	seq: number,
): Promise<boolean> {
	const target = liveTarget(botId);
	const backend = inbox;
	const sessionId = botConversationId(botId);
	if (!target || !backend || chatIsBusy(sessionId)) return false;
	const page = await loadAround(backend, target, sessionId, seq);
	useBotHistoryStore.getState().show(botId, page, true);
	useGlobalChatStore.getState().loadConversation(sessionId, page.messages);
	return true;
}

/** Puts the newest messages of the bot's history back on screen. */
export async function showLatestBotChat(botId: string): Promise<boolean> {
	const target = liveTarget(botId);
	const backend = inbox;
	const sessionId = botConversationId(botId);
	if (!target || !backend || chatIsBusy(sessionId)) return false;
	const page = await loadLatest(backend, target, sessionId);
	useBotHistoryStore.getState().show(botId, page, false);
	useGlobalChatStore.getState().loadConversation(sessionId, page.messages);
	return true;
}

/** Adds the page before the oldest message on screen; false at the start of the history. */
export async function showOlderBotChat(botId: string): Promise<boolean> {
	const target = liveTarget(botId);
	const backend = inbox;
	const olderTo = useBotHistoryStore.getState().views[botId]?.olderTo;
	const sessionId = botConversationId(botId);
	if (!target || !backend || olderTo == null || chatIsBusy(sessionId)) {
		return false;
	}
	const page = await loadOlder(backend, target, sessionId, olderTo);
	const shown = useGlobalChatStore.getState().messages;
	const known = new Set(shown.map((message) => message.id));
	useBotHistoryStore.getState().prepend(botId, page);
	useGlobalChatStore
		.getState()
		.loadConversation(sessionId, [
			...page.messages.filter((message) => !known.has(message.id)),
			...shown,
		]);
	return true;
}

/** The bot's summaries for the jump list; empty where the bot has no inbox history. */
export async function loadBotSegments(botId: string) {
	const target = liveTarget(botId);
	const backend = inbox;
	if (!target || !backend) return [];
	return loadJumpList(backend, target);
}

/** Writes a "New topic" marker into the bot's history; false where it has no inbox history. */
export async function startBotTopic(
	botId: string,
	title?: string,
): Promise<boolean> {
	const target = liveTarget(botId);
	const backend = inbox;
	if (!target || !backend) return false;
	const entry = await startNewTopic(backend, target, title);
	if (typeof entry.seq === "number") {
		useBotHistoryStore.getState().noteTopic(botId, {
			seq: entry.seq,
			title: title?.trim() || null,
			at: Date.parse(entry.created_at) || Date.now(),
		});
	}
	return true;
}

/** Whether the bot's chat on screen reads its history from the inbox. */
export function botHasInboxHistory(botId: string): boolean {
	return liveTarget(botId) !== null;
}

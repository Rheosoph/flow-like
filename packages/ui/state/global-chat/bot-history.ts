import { createId } from "@paralleldrive/cuid2";
import { foldInboxEntries } from "../../components/bots-tab/fold-entries";
import type { IMessage } from "../../components/interfaces/chat-default/chat-db";
import { IRole } from "../../lib/schema/llm/history";
import type {
	IConversationSegment,
	IInboxEntry,
	IInboxImportEntry,
	IInboxPlainEnvelope,
	IInboxState,
} from "../backend-state/inbox-state";
import { GLOBAL_CHAT_APP_ID } from "./global-chat-db";

/** Hosts leave a marked message to its chat this long; the chat holds the turn while it streams. */
export const INTERACTIVE_GRACE_MS = 120_000;

/** Entries one page of the chat reads. */
export const HISTORY_PAGE = 50;

/** A jump shows this many messages before the target and after it, so the target sits near the bottom. */
const WINDOW_BEFORE = 40;
const WINDOW_AFTER = 10;
const NEWEST = Number.MAX_SAFE_INTEGER;
const IMPORT_CHUNK = 200;
const MAX_IMPORT_TEXT = 48_000;
const IMPORTED_KEY_PREFIX = "flow-like:bot-chat-imported:";

export interface BotHistoryTarget {
	botId: string;
	conversationId: string;
}

export interface TranscriptPage {
	messages: IMessage[];
	/** Position of each message id, for the jump list and "show where this was said". */
	seqs: Record<string, number>;
	/** The `to_seq` that continues further back; null at the start of the history. */
	olderTo: number | null;
	/** The `from_seq` that continues forward; null at the newest entry. */
	newerFrom: number | null;
}

const conversations = new Map<string, Promise<string>>();

/** The id of the bot's direct conversation with its owner; created on first use. */
export function botConversation(
	inbox: IInboxState,
	botId: string,
): Promise<string> {
	let found = conversations.get(botId);
	if (!found) {
		found = inbox
			.createConversation({ kind: "direct", members: [botId] })
			.then((conversation) => conversation.id);
		conversations.set(botId, found);
		found.catch(() => conversations.delete(botId));
	}
	return found;
}

export function forgetBotConversation(botId?: string) {
	if (botId === undefined) conversations.clear();
	else conversations.delete(botId);
}

export function interactiveEnvelope(
	text: string,
	clientRunId: string,
): IInboxPlainEnvelope {
	return {
		v: 1,
		enc: "none",
		body: { text, interactive: { client_run_id: clientRunId } },
	};
}

function plainEnvelope(body: unknown): IInboxPlainEnvelope {
	return { v: 1, enc: "none", body };
}

/**
 * Writes the user's message into the bot's inbox with the interactive mark: the server's queue and
 * the desktop host both leave it to this chat while the run answers it.
 */
export function sendInteractive(
	inbox: IInboxState,
	target: BotHistoryTarget,
	message: { id: string; text: string; clientRunId: string },
): Promise<IInboxEntry> {
	return inbox.sendEntry(target.conversationId, {
		message_id: message.id,
		kind: "message",
		envelope: interactiveEnvelope(message.text, message.clientRunId),
	});
}

/** Starts the bot's next segment: a "New topic" marker, optionally titled. */
export function startNewTopic(
	inbox: IInboxState,
	target: BotHistoryTarget,
	title?: string,
): Promise<IInboxEntry> {
	const trimmed = title?.trim();
	return inbox.sendEntry(target.conversationId, {
		message_id: `topic_${createId()}`,
		kind: "topic",
		envelope: plainEnvelope(trimmed ? { title: trimmed } : {}),
	});
}

function messageOf(
	item: ReturnType<typeof foldInboxEntries>[number],
	botId: string,
	sessionId: string,
): IMessage | null {
	if (item.type !== "message" || item.content.kind !== "text") return null;
	const { approval } = item;
	return {
		...(approval
			? {
					approval: {
						id: approval.id,
						botId: item.senderId,
						turnId: approval.turnId,
						tool: approval.tool,
						summary: approval.summary,
						scopes: approval.scopes,
						answer: approval.answer,
					},
				}
			: {}),
		id: item.id,
		appId: GLOBAL_CHAT_APP_ID,
		sessionId,
		inner: {
			role: item.senderId === botId ? IRole.Assistant : IRole.User,
			content: approval ? "" : item.content.text,
		},
		files: [],
		tools: [],
		actions: [],
		timestamp: item.time,
	};
}

/** The chat's messages from inbox entries (any order, repeats allowed), oldest first. */
export function transcriptOf(
	entries: readonly IInboxEntry[],
	botId: string,
	sessionId: string,
): Pick<TranscriptPage, "messages" | "seqs"> {
	const seqs: Record<string, number> = {};
	for (const entry of entries) {
		if (entry.kind === "message" && typeof entry.seq === "number") {
			seqs[entry.message_id] = entry.seq;
		}
	}
	const messages = foldInboxEntries(entries)
		.map((item) => messageOf(item, botId, sessionId))
		.filter((message): message is IMessage => message !== null);
	return { messages, seqs };
}

async function readPage(
	inbox: IInboxState,
	target: BotHistoryTarget,
	sessionId: string,
	query: Parameters<IInboxState["listEntries"]>[1],
): Promise<TranscriptPage> {
	const page = await inbox.listEntries(target.conversationId, query);
	return {
		...transcriptOf(page.entries, target.botId, sessionId),
		olderTo: page.next_to_seq ?? null,
		newerFrom: page.next_from_seq ?? null,
	};
}

/** The newest page of the chat. */
export function loadLatest(
	inbox: IInboxState,
	target: BotHistoryTarget,
	sessionId: string,
	limit = HISTORY_PAGE,
): Promise<TranscriptPage> {
	return readPage(inbox, target, sessionId, { to_seq: NEWEST, limit });
}

/** The page before `olderTo`. */
export function loadOlder(
	inbox: IInboxState,
	target: BotHistoryTarget,
	sessionId: string,
	olderTo: number,
	limit = HISTORY_PAGE,
): Promise<TranscriptPage> {
	return readPage(inbox, target, sessionId, { to_seq: olderTo, limit });
}

/** A window of the chat around a position, for a jump-list click or a fact's `#seq`. */
export function loadAround(
	inbox: IInboxState,
	target: BotHistoryTarget,
	sessionId: string,
	seq: number,
): Promise<TranscriptPage> {
	return readPage(inbox, target, sessionId, {
		around_seq: seq,
		before_count: WINDOW_BEFORE,
		after_count: WINDOW_AFTER,
	});
}

export interface JumpEntry {
	id: string;
	title: string;
	seqFrom: number;
	seqTo: number;
	from: number;
	to: number;
}

/**
 * The jump list: the bot's own level-0 summaries, newest first, each titled and dated. Higher
 * levels merge the same ranges, so listing only the finest keeps the list one row per stretch.
 */
export function jumpList(
	segments: readonly IConversationSegment[],
	fallbackTitle: (from: Date) => string,
): JumpEntry[] {
	return segments
		.filter((segment) => segment.level === 0)
		.map((segment) => {
			const from = Date.parse(segment.t_from);
			return {
				id: segment.id,
				title: segment.title?.trim() || fallbackTitle(new Date(from)),
				seqFrom: segment.seq_from,
				seqTo: segment.seq_to,
				from,
				to: Date.parse(segment.t_to),
			};
		})
		.sort((a, b) => b.seqFrom - a.seqFrom);
}

export async function loadJumpList(
	inbox: IInboxState,
	target: BotHistoryTarget,
): Promise<IConversationSegment[]> {
	const found: IConversationSegment[] = [];
	let after: string | undefined;
	for (let page = 0; page < 20; page += 1) {
		const list = await inbox.listSegments(target.conversationId, {
			level: 0,
			after,
			limit: 100,
		});
		found.push(...list.segments);
		if (!list.next_after) break;
		after = list.next_after;
	}
	return found;
}

export interface LegacyMessage {
	id: string;
	role: "user" | "assistant";
	text: string;
	timestamp: number;
}

export type LegacyImportEntry = IInboxImportEntry;

/** What one old IndexedDB message becomes in the inbox: its id is kept, so a repeat adds nothing. */
export function legacyImportEntry(
	message: LegacyMessage,
): LegacyImportEntry | null {
	const text = message.text.trim().slice(0, MAX_IMPORT_TEXT);
	if (!text || !/^[A-Za-z0-9_-]{1,64}$/.test(message.id)) return null;
	return {
		message_id: message.id,
		sender: message.role === "assistant" ? "bot" : "owner",
		envelope: plainEnvelope({ text }),
		created_at: new Date(message.timestamp).toISOString(),
	};
}

function importedKey(botId: string): string {
	return `${IMPORTED_KEY_PREFIX}${botId}`;
}

function wasImported(botId: string): boolean {
	try {
		return localStorage.getItem(importedKey(botId)) === "1";
	} catch {
		return false;
	}
}

function markImported(botId: string) {
	try {
		localStorage.setItem(importedKey(botId), "1");
	} catch {
		// the server skips repeats by message id, so a lost flag only costs a repeat
	}
}

/**
 * Moves the bot's old IndexedDB chat into its inbox, once: oldest first, in chunks, with the old
 * message ids, which the inbox skips when they are already there. Resolves with how many entries
 * the inbox added.
 */
export async function importLegacyChat(
	inbox: IInboxState,
	target: BotHistoryTarget,
	legacy: readonly LegacyMessage[],
): Promise<number> {
	if (wasImported(target.botId)) return 0;
	const entries = [...legacy]
		.sort((a, b) => a.timestamp - b.timestamp)
		.map(legacyImportEntry)
		.filter((entry): entry is LegacyImportEntry => entry !== null);
	let added = 0;
	if (entries.length > 0) {
		if (!inbox.importHistory) {
			throw new Error("This backend cannot import a bot's earlier chat");
		}
		for (let start = 0; start < entries.length; start += IMPORT_CHUNK) {
			const result = await inbox.importHistory(
				target.botId,
				target.conversationId,
				entries.slice(start, start + IMPORT_CHUNK),
			);
			added += result.imported;
		}
	}
	markImported(target.botId);
	return added;
}

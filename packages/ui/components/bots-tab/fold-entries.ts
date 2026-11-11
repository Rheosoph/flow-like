import { compareInboxCursors } from "../../hooks/use-inbox";
import { isHttpUrl } from "../../lib/inbox-attachment-part";
import { isRecord } from "../../lib/response-shape";
import type { IBotNoteRef } from "../../state/backend-state/bot-state";
import type {
	IInboxApprovalSecret,
	IInboxAttachmentPart,
	IInboxEntry,
	IInboxFileHandle,
} from "../../state/backend-state/inbox-state";
import {
	type ApprovalDecision,
	approvalOf,
	approvalResponseOf,
	plainBody,
} from "./approval-entry";
import { noteRefOf } from "./bot-note-entry";
import { isSelfPrincipal } from "./crew-model";
import {
	type UserContextCardData,
	userContextRequestOf,
	userContextResponseOf,
	userContextRevokeOf,
} from "./user-context-entry";

export interface ConversationReaction {
	emoji: string;
	count: number;
	mine: boolean;
	reactorIds: string[];
}

export type ConversationMessageContent =
	| { kind: "text"; text: string }
	| { kind: "sealed" }
	| { kind: "unsupported" };

/** A bot's request to run a call, with the owner's answer once there is one. */
export interface ConversationApproval {
	id: string;
	turnId: string;
	tool: string;
	summary: string;
	scopes: string[];
	/** Who woke the turn, when it was not the owner alone. */
	requesterName?: string;
	room?: string;
	secret?: IInboxApprovalSecret;
	configurationChanges?: Record<string, unknown>;
	answer?: ApprovalDecision;
}

function attachmentsOf(entry: IInboxEntry): IInboxAttachmentPart[] {
	const parts = plainBody(entry)?.parts;
	if (!Array.isArray(parts)) return [];
	return parts.filter(
		(part): part is IInboxAttachmentPart =>
			isRecord(part) &&
			part.kind === "attachment" &&
			typeof part.name === "string" &&
			((isRecord(part.handle) && typeof part.handle.path === "string") ||
				isHttpUrl(part.url)),
	);
}

function errorCodeOf(entry: IInboxEntry): string | undefined {
	const error = plainBody(entry)?.error;
	return isRecord(error) && typeof error.code === "string"
		? error.code
		: undefined;
}

function stringsOf(value: unknown): string[] {
	return Array.isArray(value)
		? value.filter((item): item is string => typeof item === "string")
		: [];
}

function mentionsOf(entry: IInboxEntry): string[] {
	return stringsOf(plainBody(entry)?.mentions);
}

/** Who a bot's message went to by email, when the message records a sent mail. */
function mailedToOf(entry: IInboxEntry): string[] {
	const mail = plainBody(entry)?.mail;
	if (!isRecord(mail) || mail.direction !== "out") return [];
	return [...stringsOf(mail.to), ...stringsOf(mail.cc)];
}

export interface ConversationMessage {
	type: "message";
	/** The event's `message_id`, which edits, deletes and reactions target. */
	id: string;
	senderId: string;
	mine: boolean;
	createdAt: string;
	time: number;
	content: ConversationMessageContent;
	replyTo?: string;
	mentions: string[];
	errorCode?: string;
	attachments?: IInboxAttachmentPart[];
	approval?: ConversationApproval;
	userContext?: UserContextCardData;
	userContextDerived?: boolean;
	userContextDerivedFrom?: string;
	note?: IBotNoteRef;
	/** Recipients of the email this message was sent as. */
	mailedTo?: string[];
	edited: boolean;
	reactions: ConversationReaction[];
}

export interface ConversationMemberEvent {
	type: "member";
	id: string;
	createdAt: string;
	time: number;
	op: "join" | "leave";
	principalId: string;
	by: string;
}

/** Someone renamed the group or changed its picture. */
export interface ConversationRoomEvent {
	type: "room";
	id: string;
	createdAt: string;
	time: number;
	change: "title" | "image";
	/** The new name, when a rename set one. */
	title?: string;
	/** The name or picture was removed rather than set. */
	removed: boolean;
	by: string;
}

export type TriggerSource = "schedule" | "webhook" | "email" | "event";

/** Work that woke a bot: a schedule firing, a webhook, an email or an event. */
export interface ConversationTrigger {
	type: "trigger";
	id: string;
	createdAt: string;
	time: number;
	source: TriggerSource;
	/** The task, webhook, mail or flow request that fired. */
	refId: string;
	title: string;
	/** What the bot was asked to do. */
	text: string;
	payload?: unknown;
	/** A payload too large to inline, in the bot's workspace. */
	payloadFile?: IInboxFileHandle;
	/** The bot it woke, when the conversation is a group. */
	botId?: string;
}

/** One bot asked another; question and answer live in the exchange conversation. */
export interface ConversationBotAsk {
	type: "bot_ask";
	id: string;
	createdAt: string;
	time: number;
	askId: string;
	askerId: string;
	calleeId: string;
	exchangeId: string;
	question: string;
}

/** A line between messages: what woke a bot, or one bot asking another. */
export type ConversationNote = ConversationTrigger | ConversationBotAsk;

/** A call someone started, with how it ended once its last person left. */
export interface ConversationCall {
	type: "call";
	/** The `started` entry's `message_id`, which `ended` entries and awareness name. */
	id: string;
	senderId: string;
	createdAt: string;
	time: number;
	durationMs?: number;
}

/** One spoken utterance of a person in a call, transcribed on their own device. */
export interface ConversationTranscript {
	type: "transcript";
	id: string;
	senderId: string;
	mine: boolean;
	createdAt: string;
	time: number;
	call: string;
	text: string;
	offsetMs: number;
	mentions: string[];
}

export type ConversationItem =
	| ConversationMessage
	| ConversationMemberEvent
	| ConversationRoomEvent
	| ConversationNote
	| ConversationCall
	| ConversationTranscript;

export type ConversationBlock =
	| { type: "day"; key: string; time: number }
	| { type: "member"; key: string; event: ConversationMemberEvent }
	| { type: "room"; key: string; event: ConversationRoomEvent }
	| { type: "note"; key: string; note: ConversationNote }
	| { type: "call"; key: string; call: ConversationCall }
	| {
			type: "spoken";
			key: string;
			senderId: string;
			mine: boolean;
			lines: ConversationTranscript[];
	  }
	| {
			type: "group";
			key: string;
			senderId: string;
			mine: boolean;
			messages: ConversationMessage[];
	  };

/** Messages from one sender closer together than this share one avatar and name. */
export const MESSAGE_GROUP_WINDOW_MS = 5 * 60_000;

function entryTime(entry: IInboxEntry): number {
	const time = Date.parse(entry.created_at);
	return Number.isFinite(time) ? time : 0;
}

function isSealed(entry: IInboxEntry): boolean {
	const envelope: unknown = entry.envelope;
	return isRecord(envelope) && envelope.enc !== "none";
}

function text(value: unknown): string | undefined {
	return typeof value === "string" ? value : undefined;
}

export function inboxMessageContent(
	entry: IInboxEntry,
): ConversationMessageContent {
	if (isSealed(entry)) return { kind: "sealed" };
	const body = text(plainBody(entry)?.text);
	return body === undefined
		? { kind: "unsupported" }
		: { kind: "text", text: body };
}

function memberEvent(entry: IInboxEntry): ConversationMemberEvent | null {
	const body = plainBody(entry);
	const op = body?.op;
	const principalId = text(body?.principal_id);
	if ((op !== "join" && op !== "leave") || !principalId) return null;
	return {
		type: "member",
		id: entry.message_id,
		createdAt: entry.created_at,
		time: entryTime(entry),
		op,
		principalId,
		by: text(body?.by) || principalId,
	};
}

function roomEvent(entry: IInboxEntry): ConversationRoomEvent | null {
	const body = plainBody(entry);
	const change = body?.change;
	if (change !== "title" && change !== "image") return null;
	const title = text(body?.title)?.trim();
	const removed = body?.removed === true || (change === "title" && !title);
	return {
		type: "room",
		id: entry.message_id,
		createdAt: entry.created_at,
		time: entryTime(entry),
		change,
		...(title && !removed ? { title } : {}),
		removed,
		by: text(body?.by) || entry.sender_id,
	};
}

const TRIGGER_SOURCES: readonly string[] = [
	"schedule",
	"webhook",
	"email",
	"event",
];

function fileHandle(value: unknown): IInboxFileHandle | undefined {
	return isRecord(value) &&
		typeof value.app_id === "string" &&
		typeof value.path === "string"
		? (value as unknown as IInboxFileHandle)
		: undefined;
}

function triggerEvent(entry: IInboxEntry): ConversationTrigger | null {
	const body = plainBody(entry);
	const source = text(body?.source);
	if (!body || !source || !TRIGGER_SOURCES.includes(source)) return null;
	const payloadFile = fileHandle(body.payload_handle);
	const botId = text(body.bot_id);
	return {
		type: "trigger",
		id: entry.message_id,
		createdAt: entry.created_at,
		time: entryTime(entry),
		source: source as TriggerSource,
		refId: text(body.ref_id) ?? "",
		title: text(body.title) ?? "",
		text: text(body.text) ?? "",
		...(body.payload !== undefined && body.payload !== null
			? { payload: body.payload }
			: {}),
		...(payloadFile ? { payloadFile } : {}),
		...(botId ? { botId } : {}),
	};
}

function botAskEvent(entry: IInboxEntry): ConversationBotAsk | null {
	const body = plainBody(entry);
	if (body?.type !== "bot_ask") return null;
	const askerId = text(body.asker_id);
	const calleeId = text(body.callee_id);
	const exchangeId = text(body.exchange_id);
	if (!askerId || !calleeId || !exchangeId) return null;
	return {
		type: "bot_ask",
		id: entry.message_id,
		createdAt: entry.created_at,
		time: entryTime(entry),
		askId: text(body.ask_id) ?? "",
		askerId,
		calleeId,
		exchangeId,
		question: text(body.question) ?? "",
	};
}

/** The note a trigger or activity entry shows as; null for anything else or a body it can't read. */
export function noteOf(entry: IInboxEntry): ConversationNote | null {
	if (entry.kind === "trigger") return triggerEvent(entry);
	if (entry.kind === "activity") return botAskEvent(entry);
	return null;
}

function callStarted(entry: IInboxEntry): ConversationCall | null {
	if (plainBody(entry)?.op !== "started") return null;
	return {
		type: "call",
		id: entry.message_id,
		senderId: entry.sender_id,
		createdAt: entry.created_at,
		time: entryTime(entry),
	};
}

function transcriptLine(
	entry: IInboxEntry,
	me?: string | null,
): ConversationTranscript | null {
	if (isSealed(entry)) return null;
	const body = plainBody(entry);
	const spoken = text(body?.text)?.trim();
	if (!spoken) return null;
	const offset = body?.offset_ms;
	return {
		type: "transcript",
		id: entry.message_id,
		senderId: entry.sender_id,
		mine: isSelfPrincipal(entry.sender_id, me),
		createdAt: entry.created_at,
		time: entryTime(entry),
		call: text(body?.call) ?? "",
		text: spoken,
		offsetMs:
			typeof offset === "number" && Number.isFinite(offset)
				? Math.max(0, offset)
				: 0,
		mentions: mentionsOf(entry),
	};
}

function uniqueChronological(entries: readonly IInboxEntry[]): IInboxEntry[] {
	const seen = new Set<string>();
	const unique: IInboxEntry[] = [];
	for (const entry of entries) {
		if (seen.has(entry.message_id)) continue;
		seen.add(entry.message_id);
		unique.push(entry);
	}
	return unique.sort((a, b) => compareInboxCursors(a.cursor, b.cursor));
}

/**
 * Turns one reader's inbox copies into what a conversation shows, oldest first: messages with
 * their latest edit and aggregated reactions, join/leave lines, and notes for triggers and bot asks.
 * Deleted messages disappear.
 * Edits and deletes only count from the message's own sender; sealed effects can't be applied.
 */
export function foldInboxEntries(
	entries: readonly IInboxEntry[],
	me?: string | null,
): ConversationItem[] {
	const ordered = uniqueChronological(entries);
	const items: ConversationItem[] = [];
	const messages = new Map<string, ConversationMessage>();
	const reactions = new Map<string, Map<string, Set<string>>>();
	const deleted = new Set<string>();
	const calls = new Map<string, ConversationCall>();

	for (const entry of ordered) {
		if (entry.kind === "call") {
			const body = plainBody(entry);
			if (body?.op === "ended") {
				const call = calls.get(text(body.call) ?? "");
				const duration = body.duration_ms;
				if (call && call.durationMs === undefined) {
					call.durationMs =
						typeof duration === "number" && Number.isFinite(duration)
							? Math.max(0, duration)
							: 0;
				}
				continue;
			}
			const call = callStarted(entry);
			if (call) {
				calls.set(call.id, call);
				items.push(call);
			}
			continue;
		}
		if (entry.kind === "transcript") {
			const line = transcriptLine(entry, me);
			if (line) items.push(line);
			continue;
		}
		if (entry.kind === "message") {
			const contextRevoke = userContextRevokeOf(entry);
			if (contextRevoke) {
				for (const card of messages.values()) {
					const context = card.userContext;
					if (
						context?.answer === "session" &&
						context.request.bot_id === contextRevoke.bot_id &&
						context.request.subject_id === contextRevoke.subject_id
					)
						context.revoked = true;
				}
				continue;
			}
			const contextResponse = userContextResponseOf(entry);
			if (contextResponse) {
				const card = messages.get(contextResponse.request_id);
				if (
					card?.userContext?.request.subject_id ===
						contextResponse.subject_id &&
					card.userContext.request.bot_id === contextResponse.bot_id
				) {
					card.userContext.answer = contextResponse.decision;
					card.userContext.response = contextResponse;
				}
				continue;
			}
			const response = approvalResponseOf(entry);
			if (response) {
				const card = messages.get(response.approval_id);
				if (card?.approval) card.approval.answer = response.decision;
				continue;
			}
			const replyTo = text(plainBody(entry)?.reply_to);
			const approval = approvalOf(entry);
			const contextRequest = userContextRequestOf(entry);
			const mailedTo = mailedToOf(entry);
			const note = noteRefOf(entry);
			const message: ConversationMessage = {
				type: "message",
				id: entry.message_id,
				senderId: entry.sender_id,
				mine: isSelfPrincipal(entry.sender_id, me),
				createdAt: entry.created_at,
				time: entryTime(entry),
				content: inboxMessageContent(entry),
				mentions: mentionsOf(entry),
				...(contextRequest ? { userContext: { request: contextRequest } } : {}),
				...(typeof plainBody(entry)?.user_context_derived === "string"
					? {
							userContextDerived: true,
							userContextDerivedFrom: plainBody(entry)
								?.user_context_derived as string,
						}
					: {}),
				...(errorCodeOf(entry) ? { errorCode: errorCodeOf(entry) } : {}),
				...(attachmentsOf(entry).length > 0
					? { attachments: attachmentsOf(entry) }
					: {}),
				...(replyTo ? { replyTo } : {}),
				...(note ? { note } : {}),
				...(mailedTo.length > 0 ? { mailedTo } : {}),
				...(approval
					? {
							approval: {
								id: approval.id,
								turnId: approval.turn_id,
								tool: approval.tool,
								summary: approval.summary,
								scopes: approval.scope.split(",").filter(Boolean),
								requesterName: approval.requester_name ?? approval.requester_id,
								room: approval.room,
								configurationChanges: approval.configuration_changes,
								...(approval.secret ? { secret: approval.secret } : {}),
							},
						}
					: {}),
				edited: false,
				reactions: [],
			};
			messages.set(message.id, message);
			items.push(message);
			continue;
		}
		if (entry.kind === "trigger" || entry.kind === "activity") {
			const note = noteOf(entry);
			if (note) items.push(note);
			continue;
		}
		if (entry.kind === "member") {
			const event = memberEvent(entry);
			if (event) items.push(event);
			continue;
		}
		if (entry.kind === "room") {
			const event = roomEvent(entry);
			if (event) items.push(event);
			continue;
		}
		const body = plainBody(entry);
		const target = messages.get(text(body?.target) ?? "");
		if (!body || !target) continue;
		if (entry.kind === "edit") {
			const edited = text(body.text);
			if (
				edited === undefined ||
				entry.sender_id !== target.senderId ||
				target.content.kind !== "text"
			)
				continue;
			target.content = { kind: "text", text: edited };
			target.edited = true;
		} else if (entry.kind === "delete") {
			if (entry.sender_id === target.senderId) deleted.add(target.id);
		} else if (entry.kind === "reaction") {
			const emoji = text(body.emoji);
			if (!emoji || (body.op !== "add" && body.op !== "remove")) continue;
			const byEmoji =
				reactions.get(target.id) ?? new Map<string, Set<string>>();
			reactions.set(target.id, byEmoji);
			const reactors = byEmoji.get(emoji) ?? new Set<string>();
			byEmoji.set(emoji, reactors);
			if (body.op === "add") reactors.add(entry.sender_id);
			else reactors.delete(entry.sender_id);
		}
	}

	for (const [messageId, byEmoji] of reactions) {
		const message = messages.get(messageId);
		if (!message) continue;
		message.reactions = [...byEmoji]
			.filter(([, reactors]) => reactors.size > 0)
			.map(([emoji, reactors]) => ({
				emoji,
				count: reactors.size,
				mine: [...reactors].some((id) => isSelfPrincipal(id, me)),
				reactorIds: [...reactors],
			}));
	}

	return items.filter(
		(item) => item.type !== "message" || !deleted.has(item.id),
	);
}

export function localDayKey(time: number): string {
	const date = new Date(time);
	return `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`;
}

/** Day dividers, join/leave lines, notes, and runs of messages one sender wrote in quick succession. */
export function groupConversationItems(
	items: readonly ConversationItem[],
	options: {
		windowMs?: number;
		dayKey?: (time: number) => string;
	} = {},
): ConversationBlock[] {
	const windowMs = options.windowMs ?? MESSAGE_GROUP_WINDOW_MS;
	const dayKey = options.dayKey ?? localDayKey;
	const blocks: ConversationBlock[] = [];
	let day: string | undefined;
	let group: Extract<ConversationBlock, { type: "group" }> | undefined;

	for (const item of items) {
		const itemDay = dayKey(item.time);
		if (itemDay !== day) {
			day = itemDay;
			group = undefined;
			blocks.push({ type: "day", key: `day:${itemDay}`, time: item.time });
		}
		if (item.type === "member") {
			group = undefined;
			blocks.push({ type: "member", key: `member:${item.id}`, event: item });
			continue;
		}
		if (item.type === "room") {
			group = undefined;
			blocks.push({ type: "room", key: `room:${item.id}`, event: item });
			continue;
		}
		if (item.type === "trigger" || item.type === "bot_ask") {
			group = undefined;
			blocks.push({ type: "note", key: `note:${item.id}`, note: item });
			continue;
		}
		if (item.type === "call") {
			group = undefined;
			blocks.push({ type: "call", key: `call:${item.id}`, call: item });
			continue;
		}
		if (item.type === "transcript") {
			group = undefined;
			const last = blocks[blocks.length - 1];
			const lastLine = last?.type === "spoken" ? last.lines.at(-1) : undefined;
			if (
				last?.type === "spoken" &&
				lastLine &&
				last.senderId === item.senderId &&
				item.time - lastLine.time <= windowMs
			) {
				last.lines.push(item);
			} else {
				blocks.push({
					type: "spoken",
					key: `spoken:${item.id}`,
					senderId: item.senderId,
					mine: item.mine,
					lines: [item],
				});
			}
			continue;
		}
		const previous = group?.messages[group.messages.length - 1];
		if (
			group &&
			previous &&
			group.senderId === item.senderId &&
			item.time - previous.time <= windowMs
		) {
			group.messages.push(item);
			continue;
		}
		group = {
			type: "group",
			key: `group:${item.id}`,
			senderId: item.senderId,
			mine: item.mine,
			messages: [item],
		};
		blocks.push(group);
	}
	return blocks;
}

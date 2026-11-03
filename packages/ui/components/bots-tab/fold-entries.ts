import { compareInboxCursors } from "../../hooks/use-inbox";
import { isRecord } from "../../lib/response-shape";
import type { IInboxEntry } from "../../state/backend-state/inbox-state";
import {
	type ApprovalDecision,
	approvalOf,
	approvalResponseOf,
	plainBody,
} from "./approval-entry";
import { isSelfPrincipal } from "./crew-model";

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
	answer?: ApprovalDecision;
}

function mentionsOf(entry: IInboxEntry): string[] {
	const mentions = plainBody(entry)?.mentions;
	return Array.isArray(mentions)
		? mentions.filter((id): id is string => typeof id === "string")
		: [];
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
	approval?: ConversationApproval;
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

export type TriggerSource = "schedule" | "webhook" | "email" | "event";

/** Work that woke a bot: a schedule firing, a webhook, an email or an event. */
export interface ConversationTrigger {
	type: "trigger";
	id: string;
	createdAt: string;
	time: number;
	source: TriggerSource;
	title: string;
}

export type ConversationItem =
	| ConversationMessage
	| ConversationMemberEvent
	| ConversationTrigger;

export type ConversationBlock =
	| { type: "day"; key: string; time: number }
	| { type: "member"; key: string; event: ConversationMemberEvent }
	| { type: "trigger"; key: string; trigger: ConversationTrigger }
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

const TRIGGER_SOURCES: readonly string[] = [
	"schedule",
	"webhook",
	"email",
	"event",
];

function triggerEvent(entry: IInboxEntry): ConversationTrigger | null {
	const body = plainBody(entry);
	const source = text(body?.source);
	if (!source || !TRIGGER_SOURCES.includes(source)) return null;
	return {
		type: "trigger",
		id: entry.message_id,
		createdAt: entry.created_at,
		time: entryTime(entry),
		source: source as TriggerSource,
		title: text(body?.title) ?? "",
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
 * their latest edit and aggregated reactions, and join/leave lines. Deleted messages disappear.
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

	for (const entry of ordered) {
		if (entry.kind === "message") {
			const response = approvalResponseOf(entry);
			if (response) {
				const card = messages.get(response.approval_id);
				if (card?.approval) card.approval.answer = response.decision;
				continue;
			}
			const replyTo = text(plainBody(entry)?.reply_to);
			const approval = approvalOf(entry);
			const message: ConversationMessage = {
				type: "message",
				id: entry.message_id,
				senderId: entry.sender_id,
				mine: isSelfPrincipal(entry.sender_id, me),
				createdAt: entry.created_at,
				time: entryTime(entry),
				content: inboxMessageContent(entry),
				mentions: mentionsOf(entry),
				...(replyTo ? { replyTo } : {}),
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
		if (entry.kind === "trigger") {
			const trigger = triggerEvent(entry);
			if (trigger) items.push(trigger);
			continue;
		}
		if (entry.kind === "member") {
			const event = memberEvent(entry);
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

/** Day dividers, join/leave lines, and runs of messages one sender wrote in quick succession. */
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
		if (item.type === "trigger") {
			group = undefined;
			blocks.push({
				type: "trigger",
				key: `trigger:${item.id}`,
				trigger: item,
			});
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

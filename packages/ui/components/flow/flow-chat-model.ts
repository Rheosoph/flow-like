import type { ChatMessage } from "../../hooks/use-realtime-chat";

export const CHAT_MAX_LENGTH = 500;
/** The composer shows its character counter from this length on. */
export const CHAT_COUNTER_FROM = 400;
/** Consecutive messages of one author closer than this share a group header. */
export const CHAT_GROUP_WINDOW_MS = 2 * 60_000;
/** Within this distance of the end the list counts as "at the bottom". */
export const CHAT_AT_BOTTOM_PX = 40;

const NODE_ID = "[A-Za-z0-9_-]{10,32}";
const SEGMENT_PATTERN = new RegExp(
	`\\[\\[node:(${NODE_ID})\\]\\]|https?://[^\\s<>"'\`]+`,
	"g",
);
const LINK_TAIL = new Set([".", ",", ";", ":", "!", "?", "'", '"', "»"]);

export type ChatSegment =
	| { kind: "text"; value: string }
	| { kind: "link"; value: string }
	| { kind: "node"; value: string };

/** The wire form of a node reference inside a chat message. */
export function nodeReferenceToken(nodeId: string): string {
	return `[[node:${nodeId}]]`;
}

/** Display name of a referenced node; the id tail when the node is unknown. */
export function nodeReferenceLabel(
	nodeId: string,
	resolveNodeName?: (nodeId: string) => string | undefined,
): string {
	return resolveNodeName?.(nodeId) ?? nodeId.slice(-6);
}

function count(text: string, char: string): number {
	let n = 0;
	for (const c of text) if (c === char) n++;
	return n;
}

/** Punctuation that ends a sentence is not part of the URL; a ")" only is when it closes a "(" in the URL. */
function trimLinkTail(url: string): string {
	let end = url.length;
	while (end > 0) {
		const char = url[end - 1];
		if (char === ")") {
			const head = url.slice(0, end);
			if (count(head, ")") <= count(head, "(")) break;
			end--;
		} else if (LINK_TAIL.has(char)) {
			end--;
		} else {
			break;
		}
	}
	return url.slice(0, end);
}

/** Split a message into plain text, http(s) links and `[[node:<id>]]` references. */
export function parseChatSegments(text: string): ChatSegment[] {
	const segments: ChatSegment[] = [];
	let last = 0;
	const pushText = (value: string) => {
		if (value) segments.push({ kind: "text", value });
	};
	for (const match of text.matchAll(SEGMENT_PATTERN)) {
		const start = match.index ?? 0;
		pushText(text.slice(last, start));
		if (match[1]) {
			segments.push({ kind: "node", value: match[1] });
			last = start + match[0].length;
		} else {
			const url = trimLinkTail(match[0]);
			segments.push({ kind: "link", value: url });
			last = start + url.length;
		}
	}
	pushText(text.slice(last));
	return segments;
}

function dayKey(ts: number): number {
	const date = new Date(ts);
	return date.getFullYear() * 10_000 + date.getMonth() * 100 + date.getDate();
}

export function isSameDay(a: number, b: number): boolean {
	return dayKey(a) === dayKey(b);
}

export interface ChatGroup {
	/** Id of the first message; stable across re-renders. */
	id: string;
	sub: string;
	/** Timestamp of the first message. */
	timestamp: number;
	messages: ChatMessage[];
}

export interface GroupChatOptions {
	windowMs?: number;
	/** Start a new group at this message index (the unread divider sits between groups). */
	breakBeforeIndex?: number;
}

/** Consecutive messages by one author within the window — and on the same day — share a header. */
export function groupChatMessages(
	messages: readonly ChatMessage[],
	options: GroupChatOptions = {},
): ChatGroup[] {
	const windowMs = options.windowMs ?? CHAT_GROUP_WINDOW_MS;
	const groups: ChatGroup[] = [];
	messages.forEach((message, index) => {
		const current = groups[groups.length - 1];
		const previous = current?.messages[current.messages.length - 1];
		const continues =
			current !== undefined &&
			previous !== undefined &&
			index !== options.breakBeforeIndex &&
			previous.sub === message.sub &&
			message.timestamp >= previous.timestamp &&
			message.timestamp - previous.timestamp <= windowMs &&
			isSameDay(previous.timestamp, message.timestamp);
		if (continues) {
			current.messages.push(message);
		} else {
			groups.push({
				id: message.id,
				sub: message.sub,
				timestamp: message.timestamp,
				messages: [message],
			});
		}
	});
	return groups;
}

export type DaySeparator =
	| { kind: "today" }
	| { kind: "yesterday" }
	| { kind: "date"; label: string };

export function daySeparatorLabel(
	ts: number,
	now: number,
	locale?: string,
): DaySeparator {
	if (isSameDay(ts, now)) return { kind: "today" };
	const yesterday = new Date(now);
	yesterday.setDate(yesterday.getDate() - 1);
	if (isSameDay(ts, yesterday.getTime())) return { kind: "yesterday" };
	const date = new Date(ts);
	const sameYear = date.getFullYear() === new Date(now).getFullYear();
	return {
		kind: "date",
		label: date.toLocaleDateString(locale, {
			weekday: "short",
			month: "short",
			day: "numeric",
			...(sameYear ? {} : { year: "numeric" }),
		}),
	};
}

export function formatChatTime(ts: number, locale?: string): string {
	return new Date(ts).toLocaleTimeString(locale, {
		hour: "numeric",
		minute: "2-digit",
	});
}

export function formatChatDateTime(ts: number, locale?: string): string {
	return new Date(ts).toLocaleString(locale, {
		dateStyle: "medium",
		timeStyle: "short",
	});
}

/**
 * Index of the first peer message newer than what the user last saw, or -1.
 * A user who never opened the chat has no "last seen" and gets no divider.
 */
export function unreadDividerIndex(
	messages: readonly ChatMessage[],
	lastSeenTimestamp: number | undefined,
	sub?: string,
): number {
	if (lastSeenTimestamp === undefined) return -1;
	return messages.findIndex(
		(message) => message.sub !== sub && message.timestamp > lastSeenTimestamp,
	);
}

export type ChatTimelineItem =
	| { type: "day"; key: string; separator: DaySeparator }
	| { type: "unread"; key: string }
	| { type: "group"; key: string; group: ChatGroup };

export interface ChatTimelineOptions {
	now: number;
	lastSeenTimestamp?: number;
	sub?: string;
	locale?: string;
	windowMs?: number;
}

/** Groups interleaved with day separators and, at most once, the unread divider. */
export function buildChatTimeline(
	messages: readonly ChatMessage[],
	options: ChatTimelineOptions,
): ChatTimelineItem[] {
	const dividerAt = unreadDividerIndex(
		messages,
		options.lastSeenTimestamp,
		options.sub,
	);
	const dividerId = dividerAt >= 0 ? messages[dividerAt].id : undefined;
	const groups = groupChatMessages(messages, {
		windowMs: options.windowMs,
		breakBeforeIndex: dividerAt >= 0 ? dividerAt : undefined,
	});
	const items: ChatTimelineItem[] = [];
	let previousDay: number | undefined;
	for (const group of groups) {
		if (previousDay === undefined || !isSameDay(previousDay, group.timestamp)) {
			items.push({
				type: "day",
				key: `day-${group.id}`,
				separator: daySeparatorLabel(
					group.timestamp,
					options.now,
					options.locale,
				),
			});
			previousDay = group.timestamp;
		}
		if (group.id === dividerId) items.push({ type: "unread", key: "unread" });
		items.push({ type: "group", key: group.id, group });
	}
	return items;
}

export type TypingLabelParts =
	| { kind: "none" }
	| { kind: "one"; name: string }
	| { kind: "two"; first: string; second: string }
	| { kind: "many"; count: number };

export function typingLabelParts(
	subs: readonly string[],
	nameOf: (sub: string) => string,
): TypingLabelParts {
	const unique = [...new Set(subs)];
	if (unique.length === 0) return { kind: "none" };
	if (unique.length === 1) return { kind: "one", name: nameOf(unique[0]) };
	if (unique.length === 2)
		return { kind: "two", first: nameOf(unique[0]), second: nameOf(unique[1]) };
	return { kind: "many", count: unique.length };
}

/**
 * Replace the selection [start, end) with `insert`, honouring the length cap.
 * All or nothing: a clipped emoji or node token would be garbage.
 */
export function insertAtCaret(
	text: string,
	insert: string,
	start: number,
	end: number = start,
	maxLength = CHAT_MAX_LENGTH,
): { text: string; caret: number } {
	const from = Math.max(0, Math.min(start, text.length));
	const to = Math.max(from, Math.min(end, text.length));
	const room = maxLength - (text.length - (to - from));
	if (insert.length > room) return { text, caret: from };
	return {
		text: text.slice(0, from) + insert + text.slice(to),
		caret: from + insert.length,
	};
}

/** Append a draft (e.g. a node reference) after whatever was already typed. */
export function appendDraft(
	text: string,
	draft: string,
	maxLength = CHAT_MAX_LENGTH,
): string {
	if (!draft) return text;
	const separator = text.length === 0 || /\s$/.test(text) ? "" : " ";
	return insertAtCaret(
		text,
		separator + draft,
		text.length,
		text.length,
		maxLength,
	).text;
}

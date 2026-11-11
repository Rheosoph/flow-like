import type { RealtimeReply, RealtimeReplyState } from "./types";

/** A reply a host is still writing, as far as this device has received it. */
export interface ReplyStream {
	readonly conversationId: string;
	/** The id the stored message will have. */
	readonly messageId: string;
	readonly replyTo: string;
	readonly senderId: string;
	readonly text: string;
	readonly state: RealtimeReplyState;
}

export type ReplyStreamListener = (reply: ReplyStream) => void;

const IDLE_MS = 120_000;
const SETTLED_MS = 30_000;
/** A dropped reply is written again only by a later attempt, never by a message that was late. */
const RESTART_AFTER_MS = 2_000;
const MAX_STREAMS = 64;

interface Held {
	stream: ReplyStream;
	touched: number;
}

const streams = new Map<string, Held>();
const listeners = new Map<string, Set<ReplyStreamListener>>();

const settled = (state: RealtimeReplyState) =>
	state === "done" || state === "dropped";

function prune(now: number) {
	for (const [id, held] of streams) {
		const limit = settled(held.stream.state) ? SETTLED_MS : IDLE_MS;
		if (now - held.touched > limit) streams.delete(id);
	}
}

function hold(id: string, held: Held) {
	if (!streams.has(id) && streams.size >= MAX_STREAMS) {
		const oldest = streams.keys().next().value;
		if (oldest !== undefined) streams.delete(oldest);
	}
	streams.set(id, held);
}

/** A posted reply is over, and a dropped one stays dropped for messages that were only late. */
function closed(held: Held | undefined, now: number) {
	if (!held) return false;
	if (held.stream.state === "done") return true;
	return (
		held.stream.state === "dropped" && now - held.touched < RESTART_AFTER_MS
	);
}

/** The text after a message, or undefined when the message is older or starts past what is held. */
function merged(have: string, reply: RealtimeReply) {
	if (reply.offset > have.length) return undefined;
	const text = have.slice(0, reply.offset) + reply.text;
	return text.length >= have.length ? text : undefined;
}

function started(reply: RealtimeReply, conversationId: string) {
	// Without the start there is nothing to add the rest to; the stored message covers it.
	if (reply.offset !== 0 || reply.state === "dropped") return undefined;
	const stream: ReplyStream = {
		conversationId,
		messageId: reply.message_id,
		replyTo: reply.reply_to,
		senderId: reply.sender_id,
		text: reply.text,
		state: reply.state,
	};
	return stream;
}

/** Whether a message says more about the state of the reply than what is held. */
function newer(
	previous: ReplyStream,
	reply: RealtimeReply,
	text: string | undefined,
) {
	if (settled(reply.state)) return true;
	if (text === undefined) return false;
	// Messages may arrive out of order: at the same length only a pause is newer than writing.
	return text.length > previous.text.length || reply.state === "working";
}

function advanced(previous: ReplyStream, reply: RealtimeReply) {
	const text = merged(previous.text, reply);
	const stream: ReplyStream = {
		...previous,
		text: text ?? previous.text,
		state: newer(previous, reply, text) ? reply.state : previous.state,
	};
	const same = stream.text === previous.text && stream.state === previous.state;
	return same ? undefined : stream;
}

function tell(stream: ReplyStream) {
	for (const listener of listeners.get(stream.conversationId) ?? []) {
		try {
			listener(stream);
		} catch (error) {
			console.warn("A reply stream listener failed", error);
		}
	}
}

/**
 * Takes one reply message from the realtime connection of the account. `conversationId` names
 * the conversation as this device knows it, when that differs from the id the account uses.
 */
export function deliverRealtimeReply(
	reply: RealtimeReply,
	conversationId: string = reply.conversation_id,
	now: number = Date.now(),
) {
	prune(now);
	const held = streams.get(reply.message_id);
	if (closed(held, now)) return;
	const previous =
		held && !settled(held.stream.state) ? held.stream : undefined;
	const stream = previous
		? advanced(previous, reply)
		: started(reply, conversationId);
	if (!stream) return;
	hold(reply.message_id, { stream, touched: now });
	tell(stream);
}

/** Replies being written in a conversation; ones already under way are handed over at once. */
export function watchReplyStreams(
	conversationId: string,
	listener: ReplyStreamListener,
) {
	const watching =
		listeners.get(conversationId) ?? new Set<ReplyStreamListener>();
	listeners.set(conversationId, watching);
	watching.add(listener);
	for (const { stream } of streams.values())
		if (stream.conversationId === conversationId && !settled(stream.state))
			listener(stream);
	return () => {
		watching.delete(listener);
		if (watching.size === 0 && listeners.get(conversationId) === watching)
			listeners.delete(conversationId);
	};
}

export function resetReplyStreams() {
	streams.clear();
	listeners.clear();
}

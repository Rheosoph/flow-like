import { isRecord } from "../../lib/response-shape";
import type {
	IInboxDiscussionGrant,
	IInboxDiscussionOutcome,
	IInboxDiscussionProposal,
} from "../../state/backend-state/inbox-state";

/** Bot turns a discussion may reach unless a person allowed more: 10 messages, 5 rounds. */
export const DEFAULT_DISCUSSION_HOPS = 9;
/** Bot turns the longest discussion a person may allow reaches: 40 messages, 20 rounds. */
export const MAX_DISCUSSION_HOPS = 39;
const MAX_OUTCOME_CHARS = 280;

function count(value: unknown, max: number): number | undefined {
	return typeof value === "number" &&
		Number.isInteger(value) &&
		value >= 1 &&
		value <= max
		? value
		: undefined;
}

/** A bot message's `hops`, when it is a positive whole number. */
export function readHops(value: unknown): number | undefined {
	return count(value, Number.MAX_SAFE_INTEGER);
}

/** A bot message's `discussion_hops`, when it is within what a discussion may reach. */
export function readDiscussionHops(value: unknown): number | undefined {
	return count(value, MAX_DISCUSSION_HOPS);
}

/** A bot message's `discussion_outcome`, when it is well formed. */
export function readDiscussionOutcome(
	value: unknown,
): IInboxDiscussionOutcome | undefined {
	if (
		!isRecord(value) ||
		typeof value.agreed !== "boolean" ||
		typeof value.summary !== "string"
	)
		return undefined;
	return {
		agreed: value.agreed,
		summary: value.summary.trim().slice(0, MAX_OUTCOME_CHARS),
	};
}

/** What the discussion fold reads from a room message. */
export interface DiscussionThreadMessage {
	id: string;
	senderId: string;
	time: number;
	replyTo?: string;
	mentions: readonly string[];
	hops?: number;
	discussionHops?: number;
	discussionOutcome?: IInboxDiscussionOutcome;
	discussionProposal?: IInboxDiscussionProposal;
	discussion?: IInboxDiscussionGrant;
}

export type DiscussionStatus =
	| "agreed"
	| "disagreed"
	| "waiting_for_person"
	| "rounds_used"
	| "open";

/** One message of a discussion and where it stands. */
export interface DiscussionStep {
	id: string;
	senderId: string;
	/** 1-based: the opener is message 1. */
	index: number;
	/** 1-based: a round is one message from each of two bots. */
	round: number;
}

export interface RoomDiscussion {
	/** The opener's message id. */
	id: string;
	steps: readonly DiscussionStep[];
	/** In the order they first speak, then bots the opener handed the word to. */
	bots: readonly string[];
	messageLimit: number;
	rounds: number;
	status: DiscussionStatus;
	outcome?: IInboxDiscussionOutcome;
	/** Whose turn it is while the discussion is open. */
	next?: string;
	/** A message after the discussion's last one isn't part of it. */
	superseded: boolean;
	/** When the discussion's last message was sent. */
	lastAt: number;
}

interface Thread {
	opener: DiscussionThreadMessage;
	messages: DiscussionThreadMessage[];
	index: Map<string, number>;
}

function budgetOf(
	thread: Thread,
	byId: ReadonlyMap<string, DiscussionThreadMessage>,
): number {
	for (const message of [...thread.messages].reverse()) {
		if (message.discussionHops !== undefined) return message.discussionHops;
	}
	const grant = thread.opener.replyTo
		? byId.get(thread.opener.replyTo)?.discussion
		: undefined;
	return grant
		? Math.min(grant.rounds * 2 - 1, MAX_DISCUSSION_HOPS)
		: DEFAULT_DISCUSSION_HOPS;
}

function statusOf(
	last: DiscussionThreadMessage,
	lastIndex: number,
	messageLimit: number,
): DiscussionStatus {
	if (last.discussionOutcome)
		return last.discussionOutcome.agreed ? "agreed" : "disagreed";
	if (last.discussionProposal) return "waiting_for_person";
	return lastIndex >= messageLimit ? "rounds_used" : "open";
}

/** The bot a discussion's last message hands the word to: the one it answers, else one it names. */
function nextOf(
	last: DiscussionThreadMessage,
	bots: readonly string[],
	byId: ReadonlyMap<string, DiscussionThreadMessage>,
	isBot: (id: string) => boolean,
): string | undefined {
	const answered = last.replyTo ? byId.get(last.replyTo)?.senderId : undefined;
	if (answered && answered !== last.senderId && isBot(answered))
		return answered;
	return (
		last.mentions.find((id) => id !== last.senderId && isBot(id)) ??
		bots.find((id) => id !== last.senderId)
	);
}

/**
 * The discussions between bots in a room, oldest first. A bot's message that answers another bot's
 * message continues that bot's discussion, mention or not; any other bot message may open one,
 * and does when another bot answers it or it names another bot. A message's place is its `hops`
 * where it carries them, else one after the message it answers.
 */
export function roomDiscussions(
	messages: readonly DiscussionThreadMessage[],
	isBot: (id: string) => boolean,
): RoomDiscussion[] {
	const byId = new Map(messages.map((message) => [message.id, message]));
	const threadOf = new Map<string, Thread>();
	const threads: Thread[] = [];
	for (const message of messages) {
		if (!isBot(message.senderId)) continue;
		const parent = message.replyTo ? byId.get(message.replyTo) : undefined;
		const continued =
			parent && parent.senderId !== message.senderId && isBot(parent.senderId)
				? threadOf.get(parent.id)
				: undefined;
		if (continued && parent) {
			continued.messages.push(message);
			continued.index.set(
				message.id,
				message.hops ?? (continued.index.get(parent.id) ?? 0) + 1,
			);
			threadOf.set(message.id, continued);
			continue;
		}
		const thread: Thread = {
			opener: message,
			messages: [message],
			index: new Map([[message.id, message.hops ?? 1]]),
		};
		threads.push(thread);
		threadOf.set(message.id, thread);
	}
	const latest = messages.reduce(
		(time, message) => Math.max(time, message.time),
		Number.NEGATIVE_INFINITY,
	);
	return threads.flatMap((thread) => {
		const bots = [
			...new Set([
				...thread.messages.map((message) => message.senderId),
				...thread.opener.mentions.filter(isBot),
			]),
		];
		if (bots.length < 2) return [];
		const hops = budgetOf(thread, byId);
		const messageLimit = hops + 1;
		const steps = thread.messages.map((message) => {
			const index = thread.index.get(message.id) ?? 1;
			return {
				id: message.id,
				senderId: message.senderId,
				index,
				round: Math.ceil(index / 2),
			};
		});
		const last = thread.messages.at(-1) as DiscussionThreadMessage;
		const lastIndex = steps.at(-1)?.index ?? 1;
		const status = statusOf(last, lastIndex, messageLimit);
		return [
			{
				id: thread.opener.id,
				steps,
				bots,
				messageLimit,
				rounds: Math.ceil(messageLimit / 2),
				status,
				...(last.discussionOutcome ? { outcome: last.discussionOutcome } : {}),
				...(status === "open" ? { next: nextOf(last, bots, byId, isBot) } : {}),
				superseded: latest > last.time,
				lastAt: last.time,
			},
		];
	});
}

/** Where a message stands in its discussion. */
export interface DiscussionPlace {
	discussion: RoomDiscussion;
	step: DiscussionStep;
	opens: boolean;
	ends: boolean;
}

/** Each discussion message's place, by message id. */
export function discussionPlaces(
	discussions: readonly RoomDiscussion[],
): ReadonlyMap<string, DiscussionPlace> {
	const places = new Map<string, DiscussionPlace>();
	for (const discussion of discussions) {
		discussion.steps.forEach((step, at) => {
			places.set(step.id, {
				discussion,
				step,
				opens: at === 0,
				ends: at === discussion.steps.length - 1,
			});
		});
	}
	return places;
}

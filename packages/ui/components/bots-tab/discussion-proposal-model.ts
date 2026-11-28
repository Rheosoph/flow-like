import type { IChatUsageStat } from "../../lib/llm/usage";
import { isRecord } from "../../lib/response-shape";
import type {
	IInboxDiscussionGrant,
	IInboxDiscussionProposal,
} from "../../state/backend-state/inbox-state";
import { usageBreakdown } from "../interfaces/chat-default/usage-breakdown";

/** Rounds a person may allow for one discussion between bots; the server refuses more. */
export const MAX_DISCUSSION_ROUNDS = 20;
/** Bot replies an estimate averages over, newest first. */
const ESTIMATE_SAMPLE = 12;

function rounds(value: unknown): number | undefined {
	return typeof value === "number" &&
		Number.isInteger(value) &&
		value >= 1 &&
		value <= MAX_DISCUSSION_ROUNDS
		? value
		: undefined;
}

/** A bot message's `discussion_proposal`, when it is well formed. */
export function readDiscussionProposal(
	value: unknown,
): IInboxDiscussionProposal | undefined {
	if (!isRecord(value) || typeof value.reason !== "string") return undefined;
	const proposed = rounds(value.rounds);
	return proposed === undefined
		? undefined
		: { rounds: proposed, reason: value.reason.trim() };
}

/** A person's message's `discussion`, when it is well formed. */
export function readDiscussionGrant(
	value: unknown,
): IInboxDiscussionGrant | undefined {
	if (!isRecord(value)) return undefined;
	const allowed = rounds(value.rounds);
	return allowed === undefined ? undefined : { rounds: allowed };
}

/** What the estimate and the answer of a proposal read from a room message. */
export interface DiscussionMessage {
	id: string;
	senderId: string;
	time: number;
	replyTo?: string;
	usageStats?: readonly IChatUsageStat[];
	discussion?: IInboxDiscussionGrant;
}

/** The grant that answered the proposal `proposalId`: the first later reply to it that carries one. */
export function grantFor(
	messages: readonly DiscussionMessage[],
	proposalId: string,
): { senderId: string; rounds: number } | undefined {
	const answer = messages.find(
		(message) => message.replyTo === proposalId && message.discussion,
	);
	return answer?.discussion
		? { senderId: answer.senderId, rounds: answer.discussion.rounds }
		: undefined;
}

export interface DiscussionEstimate {
	/** The average model cost of one bot reply here, from the replies that reported one. */
	perMessage: number | null;
	/** Replies the average is based on. */
	sample: number;
	/** What `rounds` rounds, two replies each, would cost at that average. */
	total: number | null;
}

/**
 * The model cost of a discussion of `rounds` rounds, estimated from the cost of the newest bot
 * replies up to the proposal. Only replies that reported a cost count; none: no estimate.
 */
export function discussionEstimate(
	messages: readonly DiscussionMessage[],
	proposalId: string,
	rounds: number,
): DiscussionEstimate {
	const upTo = messages.findIndex((message) => message.id === proposalId);
	const costs = (upTo < 0 ? messages : messages.slice(0, upTo + 1))
		.flatMap((message) => {
			if (!message.usageStats?.length) return [];
			const cost = usageBreakdown(message.usageStats).cost;
			return cost === null ? [] : [cost];
		})
		.slice(-ESTIMATE_SAMPLE);
	if (costs.length === 0) return { perMessage: null, sample: 0, total: null };
	const perMessage = costs.reduce((sum, cost) => sum + cost, 0) / costs.length;
	return { perMessage, sample: costs.length, total: perMessage * rounds * 2 };
}

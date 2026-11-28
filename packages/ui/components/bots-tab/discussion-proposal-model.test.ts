import { describe, expect, test } from "bun:test";
import type { IChatUsageStat } from "../../lib/llm/usage";
import {
	type DiscussionMessage,
	MAX_DISCUSSION_ROUNDS,
	discussionEstimate,
	grantFor,
	readDiscussionGrant,
	readDiscussionProposal,
} from "./discussion-proposal-model";

function usage(cost: number | null): IChatUsageStat[] {
	return [
		{
			step_name: "answer",
			stats: {
				usage: {
					prompt_tokens: 100,
					completion_tokens: 50,
					total_tokens: 150,
					...(cost === null ? {} : { cost }),
				},
			},
		},
	];
}

function message(
	id: string,
	senderId: string,
	extra: Partial<DiscussionMessage> = {},
): DiscussionMessage {
	return { id, senderId, time: 0, ...extra };
}

describe("discussion proposals", () => {
	test("reads well-formed proposals and grants only", () => {
		expect(
			readDiscussionProposal({ rounds: 12, reason: " Three markets " }),
		).toEqual({ rounds: 12, reason: "Three markets" });
		expect(readDiscussionProposal({ rounds: 12 })).toBeUndefined();
		expect(
			readDiscussionProposal({
				rounds: MAX_DISCUSSION_ROUNDS + 1,
				reason: "x",
			}),
		).toBeUndefined();
		expect(
			readDiscussionProposal({ rounds: 2.5, reason: "x" }),
		).toBeUndefined();
		expect(readDiscussionGrant({ rounds: 12 })).toEqual({ rounds: 12 });
		expect(readDiscussionGrant({ rounds: 0 })).toBeUndefined();
		expect(readDiscussionGrant("12")).toBeUndefined();
	});

	test("finds the grant that answered a proposal", () => {
		const messages = [
			message("p", "bot_tiro"),
			message("chat", "user-b", { replyTo: "p" }),
			message("ok", "user-a", { replyTo: "p", discussion: { rounds: 12 } }),
		];
		expect(grantFor(messages, "p")).toEqual({ senderId: "user-a", rounds: 12 });
		expect(grantFor(messages, "other")).toBeUndefined();
	});

	test("estimates from the cost of the replies up to the proposal", () => {
		const usages = (u: number | null) => usage(u);
		const messages = [
			message("a", "bot_tiro", { usageStats: usages(0.02) }),
			message("person", "user-a"),
			message("b", "bot_story", { usageStats: usages(0.04) }),
			message("unpriced", "bot_story", { usageStats: usages(null) }),
			message("p", "bot_tiro", { usageStats: usages(0.03) }),
			message("later", "bot_story", { usageStats: usages(1) }),
		];
		const estimate = discussionEstimate(messages, "p", 12);
		expect(estimate.sample).toBe(3);
		expect(estimate.perMessage).toBeCloseTo(0.03);
		expect(estimate.total).toBeCloseTo(0.72);
		expect(discussionEstimate([message("p", "bot_tiro")], "p", 12)).toEqual({
			perMessage: null,
			sample: 0,
			total: null,
		});
	});
});

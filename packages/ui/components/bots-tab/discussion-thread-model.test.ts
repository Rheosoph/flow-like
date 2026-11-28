import { describe, expect, test } from "bun:test";
import {
	DEFAULT_DISCUSSION_HOPS,
	type DiscussionThreadMessage,
	discussionPlaces,
	readDiscussionHops,
	readDiscussionOutcome,
	readHops,
	roomDiscussions,
} from "./discussion-thread-model";

const BOTS = new Set(["bot_ada", "bot_story", "bot_ledger"]);
const isBot = (id: string) => BOTS.has(id);

let clock = 0;
function message(
	id: string,
	senderId: string,
	extra: Partial<DiscussionThreadMessage> = {},
): DiscussionThreadMessage {
	clock += 1;
	return { id, senderId, time: clock, mentions: [], ...extra };
}

/** A person's question, then `count` bot messages, each answering the one before. */
function exchange(
	count: number,
	last: Partial<DiscussionThreadMessage> = {},
): DiscussionThreadMessage[] {
	const list = [message("ask", "user-a")];
	for (let at = 1; at <= count; at++) {
		const sender = at % 2 === 1 ? "bot_ada" : "bot_story";
		list.push(
			message(`m${at}`, sender, {
				replyTo: list.at(-1)?.id,
				hops: at,
				...(at === 1 ? { mentions: ["bot_story"] } : {}),
				...(at === count ? last : {}),
			}),
		);
	}
	return list;
}

describe("reading discussion fields", () => {
	test("keeps well-formed values only", () => {
		expect(readHops(3)).toBe(3);
		expect(readHops(0)).toBeUndefined();
		expect(readHops("3")).toBeUndefined();
		expect(readDiscussionHops(19)).toBe(19);
		expect(readDiscussionHops(40)).toBeUndefined();
		expect(readDiscussionHops(1.5)).toBeUndefined();
		expect(
			readDiscussionOutcome({ agreed: true, summary: " Ship Friday. " }),
		).toEqual({ agreed: true, summary: "Ship Friday." });
		expect(
			readDiscussionOutcome({ agreed: "yes", summary: "x" }),
		).toBeUndefined();
		expect(readDiscussionOutcome({ agreed: false })).toBeUndefined();
		expect(
			readDiscussionOutcome({ agreed: false, summary: "x".repeat(400) })
				?.summary.length,
		).toBe(280);
	});
});

describe("room discussions", () => {
	test("follows replies between two bots, with or without a mention", () => {
		const [discussion] = roomDiscussions(exchange(4), isBot);
		expect(discussion?.id).toBe("m1");
		expect(discussion?.bots).toEqual(["bot_ada", "bot_story"]);
		expect(discussion?.steps.map((step) => [step.index, step.round])).toEqual([
			[1, 1],
			[2, 1],
			[3, 2],
			[4, 2],
		]);
		expect(discussion?.messageLimit).toBe(DEFAULT_DISCUSSION_HOPS + 1);
		expect(discussion?.rounds).toBe(5);
		expect(discussion?.status).toBe("open");
		expect(discussion?.next).toBe("bot_ada");
		expect(discussion?.superseded).toBe(false);
	});

	test("an opener that names another bot is a discussion before it answers", () => {
		const [discussion] = roomDiscussions(exchange(1), isBot);
		expect(discussion?.steps).toHaveLength(1);
		expect(discussion?.bots).toEqual(["bot_ada", "bot_story"]);
		expect(discussion?.next).toBe("bot_story");
	});

	test("a bot answering a person alone is no discussion", () => {
		const list = [
			message("ask", "user-a"),
			message("a", "bot_ada", { replyTo: "ask", hops: 1 }),
			message("b", "bot_story", { replyTo: "ask", hops: 1 }),
		];
		expect(roomDiscussions(list, isBot)).toEqual([]);
	});

	test("ends on an outcome, a proposal or the last allowed message", () => {
		const agreed = roomDiscussions(
			exchange(3, { discussionOutcome: { agreed: true, summary: "Go." } }),
			isBot,
		)[0];
		expect(agreed?.status).toBe("agreed");
		expect(agreed?.outcome?.summary).toBe("Go.");
		expect(agreed?.next).toBeUndefined();
		expect(
			roomDiscussions(
				exchange(2, { discussionOutcome: { agreed: false, summary: "No." } }),
				isBot,
			)[0]?.status,
		).toBe("disagreed");
		expect(
			roomDiscussions(
				exchange(2, { discussionProposal: { rounds: 9, reason: "More." } }),
				isBot,
			)[0]?.status,
		).toBe("waiting_for_person");
		expect(roomDiscussions(exchange(10), isBot)[0]?.status).toBe("rounds_used");
	});

	test("reads the budget from the newest message or the grant it started from", () => {
		const carried = roomDiscussions(exchange(3, { discussionHops: 23 }), isBot);
		expect(carried[0]?.messageLimit).toBe(24);
		expect(carried[0]?.rounds).toBe(12);

		const list = exchange(2);
		list[0] = {
			...(list[0] as DiscussionThreadMessage),
			discussion: { rounds: 8 },
		};
		expect(roomDiscussions(list, isBot)[0]?.rounds).toBe(8);
	});

	test("places messages without hops after the one they answer", () => {
		const list = [
			message("ask", "user-a"),
			message("a", "bot_ada", { replyTo: "ask", mentions: ["bot_story"] }),
			message("b", "bot_story", { replyTo: "a" }),
			message("c", "bot_ada", { replyTo: "b" }),
		];
		expect(
			roomDiscussions(list, isBot)[0]?.steps.map((step) => step.index),
		).toEqual([1, 2, 3]);
	});

	test("a later message from someone else supersedes an open discussion", () => {
		const list = [...exchange(3), message("later", "user-a")];
		expect(roomDiscussions(list, isBot)[0]?.superseded).toBe(true);
	});

	test("a person's message between bots starts a new discussion", () => {
		const list = [
			...exchange(2),
			message("again", "user-a", { replyTo: "m2" }),
			message("n1", "bot_story", {
				replyTo: "again",
				hops: 1,
				mentions: ["bot_ledger"],
			}),
			message("n2", "bot_ledger", { replyTo: "n1", hops: 2 }),
		];
		const discussions = roomDiscussions(list, isBot);
		expect(discussions.map((discussion) => discussion.id)).toEqual([
			"m1",
			"n1",
		]);
		expect(discussions[1]?.bots).toEqual(["bot_story", "bot_ledger"]);
	});

	test("places mark where each message opens or ends its discussion", () => {
		const places = discussionPlaces(roomDiscussions(exchange(3), isBot));
		expect(places.get("m1")?.opens).toBe(true);
		expect(places.get("m2")?.opens).toBe(false);
		expect(places.get("m3")?.ends).toBe(true);
		expect(places.get("ask")).toBeUndefined();
	});
});

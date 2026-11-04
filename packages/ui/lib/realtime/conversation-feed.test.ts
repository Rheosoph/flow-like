import { describe, expect, test } from "bun:test";
import type { IInboxEntry } from "../../state/backend-state/inbox-state";
import {
	type ConversationFeed,
	type ConversationFeedListener,
	type ConversationWatch,
	hasConversationFeed,
	registerConversationFeed,
	watchConversation,
} from "./conversation-feed";

const entry = (micros: number): IInboxEntry =>
	({ id: `e${micros}`, cursor: `${micros}:e${micros}` }) as IInboxEntry;

function fakeFeed() {
	const watches: {
		target: ConversationWatch;
		emit: ConversationFeedListener;
	}[] = [];
	let active = 0;
	const feed: ConversationFeed = {
		watch(target, listener) {
			const item = { target, emit: listener };
			watches.push(item);
			active++;
			return () => {
				active--;
			};
		},
	};
	return { feed, watches, active: () => active };
}

describe("conversation feed", () => {
	test("is inert without a feed and attaches watchers when one registers", () => {
		registerConversationFeed(null);
		expect(hasConversationFeed()).toBe(false);
		const seen: IInboxEntry[][] = [];
		const stop = watchConversation({ conversationId: "c" }, (e) =>
			seen.push([...e]),
		);
		const one = fakeFeed();
		registerConversationFeed(one.feed);
		expect(hasConversationFeed()).toBe(true);
		expect(one.watches).toHaveLength(1);
		one.watches[0]?.emit([entry(1), entry(2)]);
		expect(seen).toHaveLength(1);
		stop();
		expect(one.active()).toBe(0);
		registerConversationFeed(null);
	});

	test("dedupes by cursor and resumes from the latest cursor on a new feed", () => {
		const one = fakeFeed();
		const two = fakeFeed();
		registerConversationFeed(one.feed);
		const seen: string[] = [];
		const stop = watchConversation(
			{ conversationId: "c", as: "bot", after: "5:e5" },
			(entries) => seen.push(...entries.map((e) => e.cursor)),
		);
		expect(one.watches[0]?.target.after).toBe("5:e5");
		one.watches[0]?.emit([entry(4), entry(6), entry(7)]);
		one.watches[0]?.emit([entry(7)]);
		expect(seen).toEqual(["6:e6", "7:e7"]);
		registerConversationFeed(two.feed);
		expect(one.active()).toBe(0);
		expect(two.watches[0]?.target).toEqual({
			conversationId: "c",
			as: "bot",
			after: "7:e7",
		});
		stop();
		expect(two.active()).toBe(0);
		registerConversationFeed(null);
	});
});

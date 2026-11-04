import { describe, expect, test } from "bun:test";
import type { IInboxEntry } from "../../state/backend-state/inbox-state";
import { ConversationFeedHub } from "./conversation-feed-hub";

const entry = (micros: number): IInboxEntry =>
	({ id: `e${micros}`, cursor: `${micros}:e${micros}` }) as IInboxEntry;

describe("ConversationFeedHub", () => {
	test("batches changes and merges targets at the oldest cursor", async () => {
		let changes = 0;
		const hub = new ConversationFeedHub(() => changes++);
		hub.watch({ conversationId: "c", after: "5:e5" }, () => {});
		hub.watch({ conversationId: "c", after: "3:e3" }, () => {});
		hub.watch({ conversationId: "d", as: "bot", after: "1:e1" }, () => {});
		await Promise.resolve();
		expect(changes).toBe(1);
		expect(hub.targets()).toEqual([
			{ conversationId: "c", as: undefined, after: "3:e3" },
			{ conversationId: "d", as: "bot", after: "1:e1" },
		]);
		hub.watch({ conversationId: "c" }, () => {});
		expect(hub.targets()[0]?.after).toBeUndefined();
		expect(hub.isWatched("d", "bot")).toBe(true);
		expect(hub.isWatched("d")).toBe(false);
	});

	test("routes entries per watcher cursor and advances it", () => {
		const hub = new ConversationFeedHub(() => {});
		const a: string[] = [];
		const b: string[] = [];
		hub.watch({ conversationId: "c", after: "1:e1" }, (e) =>
			a.push(...e.map((x) => x.id)),
		);
		const stop = hub.watch({ conversationId: "c", as: "bot" }, (e) =>
			b.push(...e.map((x) => x.id)),
		);
		hub.deliver("c", [entry(1), entry(2)], { as: undefined });
		hub.deliver("c", [entry(2), entry(3)]);
		expect(a).toEqual(["e2", "e3"]);
		expect(b).toEqual(["e2", "e3"]);
		expect(hub.targets().find((t) => t.as === "bot")?.after).toBe("3:e3");
		stop();
		hub.deliver("c", [entry(4)]);
		expect(b).toEqual(["e2", "e3"]);
	});
});

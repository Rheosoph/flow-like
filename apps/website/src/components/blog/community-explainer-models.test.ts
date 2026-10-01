import { describe, expect, test } from "bun:test";
import {
	channelTransition,
	emptyCache,
	emptyChannel,
	exampleFeed,
	runCache,
	selectFeed,
} from "./community-explainer-models";

describe("interactive article examples", () => {
	test("a mixed feed is deduplicated and ordered before applying the limit", () => {
		expect(selectFeed(exampleFeed, "", "", 3).map((entry) => entry.id)).toEqual(
			["research", "operations", "release"],
		);
		expect(
			selectFeed(exampleFeed, " RESEARCH ", "Research", 5).map(
				(entry) => entry.id,
			),
		).toEqual(["research", "undated"]);
		expect(selectFeed(exampleFeed, "absent", "", 5)).toEqual([]);
	});
	test("a cache hit skips computation, while version changes and expiry recompute", () => {
		const cold = runCache(emptyCache, "v1");
		const warm = runCache(cold, "v1");
		expect([warm.computations, warm.hits]).toEqual([1, 1]);
		const changed = runCache(warm, "v2");
		expect(changed.computations).toBe(2);
		const expired = runCache({ ...changed, clock: 1 }, "v1");
		expect(expired.computations).toBe(3);
		expect(Object.keys(emptyCache.entries)).toHaveLength(0);
	});
	test("a late or duplicate response cannot finish a different wait", () => {
		const first = channelTransition(emptyChannel, { type: "start" });
		const timedOut = channelTransition(first, { type: "timeout" });
		const late = channelTransition(timedOut, {
			type: "reply",
			request: 1,
			approved: true,
		});
		expect(late.status).toBe("timed-out");
		const next = channelTransition(late, { type: "start" });
		const stale = channelTransition(next, {
			type: "reply",
			request: 1,
			approved: true,
		});
		expect(stale.status).toBe("waiting");
		const accepted = channelTransition(stale, {
			type: "reply",
			request: 2,
			approved: false,
		});
		expect(accepted.status).toBe("declined");
		expect(
			channelTransition(accepted, { type: "reply", request: 2, approved: true })
				.status,
		).toBe("declined");
	});
});

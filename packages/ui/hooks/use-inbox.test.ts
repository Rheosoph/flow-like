import { describe, expect, test } from "bun:test";
import type { InfiniteData } from "@tanstack/react-query";
import type {
	IInboxEntriesPage,
	IInboxEntry,
} from "../state/backend-state/inbox-state";
import {
	compareInboxCursors,
	mergeNewestEntries,
	unwatchedInboxPredicate,
} from "./use-inbox";

function entry(cursor: string): IInboxEntry {
	const [micros = "0", id = cursor] = cursor.split(":");
	return {
		id,
		message_id: `m_${id}`,
		conversation_id: "c1",
		sender_id: "anna",
		kind: "message",
		envelope: { v: 1, enc: "none", body: { text: id } },
		created_at: new Date(Number(micros) / 1000).toISOString(),
		cursor,
	};
}

describe("compareInboxCursors", () => {
	test("compares the micros as numbers", () => {
		expect(compareInboxCursors("999:z", "1000:a")).toBeLessThan(0);
		expect(compareInboxCursors("1000:a", "999:z")).toBeGreaterThan(0);
		const micros = Date.UTC(2026, 10, 1) * 1000;
		expect(
			compareInboxCursors(`${micros}:e_b`, `${micros + 1}:e_a`),
		).toBeLessThan(0);
	});

	test("breaks equal micros by id code units, not locale collation", () => {
		expect(compareInboxCursors("5:a9x", "5:a10x")).toBeGreaterThan(0);
		expect(compareInboxCursors("5:a10x", "5:a9x")).toBeLessThan(0);
		expect(compareInboxCursors("5:B", "5:a")).toBeLessThan(0);
		expect(compareInboxCursors("5:a", "5:B")).toBeGreaterThan(0);
		expect(compareInboxCursors("5:e_1", "5:e_1")).toBe(0);
	});

	test("orders a cursor without a readable time before every other", () => {
		expect(compareInboxCursors("e_1", "1:e_0")).toBeLessThan(0);
		expect(compareInboxCursors("x:e_1", "1:e_0")).toBeLessThan(0);
		expect(compareInboxCursors("x:e_1", "y:e_2")).toBeLessThan(0);
	});
});

describe("mergeNewestEntries", () => {
	const pages: InfiniteData<
		IInboxEntriesPage & { tail?: string },
		string | undefined
	> = {
		pages: [
			{ entries: [entry("5:a9x")], next_before: "5:a9x", tail: "5:a9x" },
			{ entries: [entry("4:z")], next_before: null },
		],
		pageParams: [undefined, "5:a9x"],
	};

	test("puts unseen entries on the first page in server order, once each", () => {
		const merged = mergeNewestEntries(pages, [
			entry("5:a10x"),
			entry("6:a"),
			entry("5:a9x"),
		]);
		expect(merged?.pages[0]?.entries.map((item) => item.cursor)).toEqual([
			"6:a",
			"5:a9x",
			"5:a10x",
		]);
		expect(merged?.pages[0]?.next_before).toBe("5:a9x");
		expect(merged?.pages[1]).toBe(pages.pages[1]);
	});

	test("keeps the tail a server read set", () => {
		const merged = mergeNewestEntries(pages, [entry("7:mine")]);
		expect(merged?.pages[0]?.tail).toBe("5:a9x");
	});

	test("returns the same pages when nothing is new", () => {
		expect(mergeNewestEntries(pages, [entry("4:z"), entry("5:a9x")])).toBe(
			pages,
		);
		expect(mergeNewestEntries(undefined, [entry("6:a")])).toBeUndefined();
	});
});

describe("unwatchedInboxPredicate", () => {
	const matches = unwatchedInboxPredicate(
		(id, as) => id === "c1" && as === undefined,
	);

	test("skips only the entries of watched conversations", () => {
		expect(matches({ queryKey: ["inbox", "entries", "c1", "me"] })).toBe(false);
		expect(matches({ queryKey: ["inbox", "entries", "c1", "bot"] })).toBe(true);
		expect(matches({ queryKey: ["inbox", "entries", "c2", "me"] })).toBe(true);
		expect(matches({ queryKey: ["inbox", "conversations", "me"] })).toBe(true);
		expect(matches({ queryKey: ["bots"] })).toBe(false);
	});
});

import { describe, expect, test } from "bun:test";
import type { ChatMessage } from "../../hooks/use-realtime-chat";
import {
	appendDraft,
	buildChatTimeline,
	daySeparatorLabel,
	groupChatMessages,
	insertAtCaret,
	nodeReferenceLabel,
	nodeReferenceToken,
	parseChatSegments,
	typingLabelParts,
	unreadDividerIndex,
} from "./flow-chat-model";

const NOON = new Date(2026, 7, 27, 12, 0, 0).getTime();
const MINUTE = 60_000;
const DAY = 24 * 60 * MINUTE;

let nextId = 1;
const msg = (sub: string, timestamp: number, text = "hi"): ChatMessage => ({
	id: `m${nextId++}`,
	sub,
	text,
	timestamp,
});

describe("groupChatMessages", () => {
	test("merges consecutive messages of one author within the window", () => {
		const groups = groupChatMessages([
			msg("a", NOON),
			msg("a", NOON + MINUTE),
			msg("a", NOON + 2 * MINUTE),
		]);
		expect(groups).toHaveLength(1);
		expect(groups[0].messages).toHaveLength(3);
		expect(groups[0].timestamp).toBe(NOON);
	});

	test("starts a new group on another author or after the window", () => {
		const groups = groupChatMessages([
			msg("a", NOON),
			msg("b", NOON + 10_000),
			msg("b", NOON + 10_000 + 3 * MINUTE),
		]);
		expect(groups.map((g) => [g.sub, g.messages.length])).toEqual([
			["a", 1],
			["b", 1],
			["b", 1],
		]);
	});

	test("breaks at the requested index and never spans midnight", () => {
		const beforeMidnight = new Date(2026, 7, 27, 23, 59, 30).getTime();
		const split = groupChatMessages(
			[msg("a", NOON), msg("a", NOON + 1000), msg("a", NOON + 2000)],
			{ breakBeforeIndex: 1 },
		);
		expect(split.map((g) => g.messages.length)).toEqual([1, 2]);

		const midnight = groupChatMessages([
			msg("a", beforeMidnight),
			msg("a", beforeMidnight + MINUTE),
		]);
		expect(midnight).toHaveLength(2);
	});
});

describe("daySeparatorLabel", () => {
	test("today, yesterday, then a dated label", () => {
		expect(daySeparatorLabel(NOON - 5 * MINUTE, NOON)).toEqual({
			kind: "today",
		});
		expect(daySeparatorLabel(NOON - DAY, NOON)).toEqual({ kind: "yesterday" });
		const older = daySeparatorLabel(NOON - 3 * DAY, NOON, "en-US");
		expect(older.kind).toBe("date");
		if (older.kind === "date") {
			expect(older.label).toContain("24");
			expect(older.label).not.toContain("2026");
		}
	});

	test("shows the year once the message is from another year", () => {
		const label = daySeparatorLabel(NOON - 400 * DAY, NOON, "en-US");
		expect(label.kind === "date" && label.label).toContain("2025");
	});
});

describe("parseChatSegments", () => {
	test("plain text stays one segment", () => {
		expect(parseChatSegments("just words")).toEqual([
			{ kind: "text", value: "just words" },
		]);
		expect(parseChatSegments("")).toEqual([]);
	});

	test("links drop trailing punctuation but keep balanced parentheses", () => {
		expect(parseChatSegments("see https://example.com/a?b=1, ok")).toEqual([
			{ kind: "text", value: "see " },
			{ kind: "link", value: "https://example.com/a?b=1" },
			{ kind: "text", value: ", ok" },
		]);
		expect(
			parseChatSegments("(https://en.wikipedia.org/wiki/Foo_(bar))"),
		).toEqual([
			{ kind: "text", value: "(" },
			{ kind: "link", value: "https://en.wikipedia.org/wiki/Foo_(bar)" },
			{ kind: "text", value: ")" },
		]);
	});

	test("node references become chips; malformed ones stay text", () => {
		const id = "abcdefghij1234";
		expect(
			parseChatSegments(`look at ${nodeReferenceToken(id)} please`),
		).toEqual([
			{ kind: "text", value: "look at " },
			{ kind: "node", value: id },
			{ kind: "text", value: " please" },
		]);
		expect(
			parseChatSegments("[[node:short]] and [[node:has space here]]"),
		).toEqual([
			{ kind: "text", value: "[[node:short]] and [[node:has space here]]" },
		]);
	});

	test("a node chip resolves to its name, else the id tail", () => {
		expect(nodeReferenceLabel("abcdefghij1234", () => "Branch")).toBe("Branch");
		expect(nodeReferenceLabel("abcdefghij1234", () => undefined)).toBe(
			"ij1234",
		);
		expect(nodeReferenceLabel("abcdefghij1234")).toBe("ij1234");
	});
});

describe("typingLabelParts", () => {
	test("none, one, two, many — with duplicate subs folded", () => {
		const nameOf = (sub: string) => sub.toUpperCase();
		expect(typingLabelParts([], nameOf)).toEqual({ kind: "none" });
		expect(typingLabelParts(["a", "a"], nameOf)).toEqual({
			kind: "one",
			name: "A",
		});
		expect(typingLabelParts(["a", "b"], nameOf)).toEqual({
			kind: "two",
			first: "A",
			second: "B",
		});
		expect(typingLabelParts(["a", "b", "c"], nameOf)).toEqual({
			kind: "many",
			count: 3,
		});
	});
});

describe("unreadDividerIndex", () => {
	test("points at the first peer message after the last seen timestamp", () => {
		const messages = [
			msg("peer", NOON - MINUTE),
			msg("me", NOON + MINUTE),
			msg("peer", NOON + 2 * MINUTE),
			msg("peer", NOON + 3 * MINUTE),
		];
		expect(unreadDividerIndex(messages, NOON, "me")).toBe(2);
		expect(unreadDividerIndex(messages, NOON + 3 * MINUTE, "me")).toBe(-1);
		expect(unreadDividerIndex(messages, undefined, "me")).toBe(-1);
	});
});

describe("buildChatTimeline", () => {
	test("interleaves day separators and the unread divider between groups", () => {
		const messages = [
			msg("peer", NOON - DAY),
			msg("peer", NOON - DAY + MINUTE),
			msg("peer", NOON),
			msg("peer", NOON + 30_000),
		];
		const items = buildChatTimeline(messages, {
			now: NOON + MINUTE,
			lastSeenTimestamp: NOON + 10_000,
			sub: "me",
		});
		expect(items.map((item) => item.type)).toEqual([
			"day",
			"group",
			"day",
			"group",
			"unread",
			"group",
		]);
		expect(items[0].type === "day" && items[0].separator.kind).toBe(
			"yesterday",
		);
		expect(items[2].type === "day" && items[2].separator.kind).toBe("today");
		expect(items[5].type === "group" && items[5].group.messages[0].id).toBe(
			messages[3].id,
		);
	});
});

describe("composer helpers", () => {
	test("insertAtCaret replaces the selection and moves the caret", () => {
		expect(insertAtCaret("hello world", "👋", 5, 6)).toEqual({
			text: "hello👋world",
			caret: 7,
		});
		expect(insertAtCaret("ab", "x", 99)).toEqual({ text: "abx", caret: 3 });
	});

	test("insertAtCaret refuses rather than clips at the length cap", () => {
		const full = "x".repeat(499);
		expect(insertAtCaret(full, "❤️", 499, 499)).toEqual({
			text: full,
			caret: 499,
		});
		expect(insertAtCaret(full, "!", 499, 499).text).toHaveLength(500);
	});

	test("appendDraft separates from typed text with a single space", () => {
		const token = nodeReferenceToken("abcdefghij1234");
		expect(appendDraft("", `${token} `)).toBe(`${token} `);
		expect(appendDraft("see ", token)).toBe(`see ${token}`);
		expect(appendDraft("see", token)).toBe(`see ${token}`);
	});
});

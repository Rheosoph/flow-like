import { describe, expect, test } from "bun:test";
import type { IConversation } from "../../state/backend-state/inbox-state";
import {
	type Mentionable,
	filterMentionables,
	insertMention,
	mentionQuery,
	mentionablesOf,
	mentionsIn,
} from "./mentions";

const options: Mentionable[] = [
	{ id: "u1", name: "Ada", kind: "user" },
	{ id: "b1", name: "Scout", kind: "bot" },
	{ id: "b2", name: "Mascot", kind: "bot" },
];

describe("mentionQuery", () => {
	test("finds the word after @ right before the caret", () => {
		expect(mentionQuery("hi @sc", 6)).toEqual({ start: 3, query: "sc" });
		expect(mentionQuery("@", 1)).toEqual({ start: 0, query: "" });
	});

	test("ignores an @ inside a word or a finished mention", () => {
		expect(mentionQuery("mail me@x", 9)).toBeNull();
		expect(mentionQuery("hi @Ada there", 13)).toBeNull();
	});
});

describe("insertMention", () => {
	test("replaces the typed word with the name and moves the caret after it", () => {
		const range = { start: 3, query: "sc" };
		expect(insertMention("hi @sc", range, 6, "Scout")).toEqual({
			text: "hi @Scout ",
			caret: 10,
		});
	});

	test("keeps the text after the caret", () => {
		const range = { start: 0, query: "a" };
		expect(insertMention("@a rest", range, 2, "Ada")).toEqual({
			text: "@Ada  rest",
			caret: 5,
		});
	});
});

describe("filterMentionables", () => {
	test("matches anywhere in the name, prefix matches first", () => {
		expect(filterMentionables(options, "sco").map((o) => o.id)).toEqual([
			"b1",
			"b2",
		]);
	});

	test("an empty query lists everyone", () => {
		expect(filterMentionables(options, "")).toHaveLength(3);
	});
});

describe("mentionsIn", () => {
	test("keeps only the picked principals whose @name is still in the text", () => {
		const picked = new Map([
			["b1", "Scout"],
			["u1", "Ada"],
		]);
		expect(mentionsIn("hello @Scout please", picked)).toEqual(["b1"]);
		expect(mentionsIn("nobody", picked)).toEqual([]);
	});
});

describe("mentionablesOf", () => {
	const room = {
		kind: "group",
		members: [
			{ principal_id: "me", kind: "user" },
			{ principal_id: "u1", kind: "user" },
			{ principal_id: "b1", kind: "bot" },
		],
	} as unknown as IConversation;
	const directory = ((id: string) => ({ name: `name-${id}` })) as never;

	test("lists the room's other active members, bots included", () => {
		expect(mentionablesOf(room, directory, "me")).toEqual([
			{ id: "u1", name: "name-u1", kind: "user" },
			{ id: "b1", name: "name-b1", kind: "bot" },
		]);
	});

	test("a bot that is not a member is never suggested", () => {
		const ids = mentionablesOf(room, directory, "me").map((o) => o.id);
		expect(ids).not.toContain("bot-elsewhere");
	});

	test("direct chats offer nothing", () => {
		expect(
			mentionablesOf(
				{ ...room, kind: "direct" } as IConversation,
				directory,
				"me",
			),
		).toEqual([]);
	});
});

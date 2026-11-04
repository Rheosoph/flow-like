import { describe, expect, test } from "bun:test";
import type { IConversation } from "../../state/backend-state/inbox-state";
import {
	type Mentionable,
	filterMentionables,
	inlineMentions,
	insertMention,
	mentionQuery,
	mentionablesOf,
	mentionedIn,
	spokenMentions,
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

describe("mentionedIn", () => {
	test("names everyone whose @name stands in the text as a whole word", () => {
		expect(mentionedIn("hello @Scout please", options)).toEqual(["b1"]);
		expect(mentionedIn("@Ada, @Mascot!", options)).toEqual(["u1", "b2"]);
		expect(mentionedIn("nobody", options)).toEqual([]);
	});

	test("ignores a name inside a longer word or without its @", () => {
		expect(mentionedIn("@Scouting ahead", options)).toEqual([]);
		expect(mentionedIn("Scout, come here", options)).toEqual([]);
		expect(mentionedIn("mail@Scout", options)).toEqual([]);
	});

	test("reads names with spaces and regex characters literally", () => {
		const people: Mentionable[] = [
			{ id: "p1", name: "Anna Weber", kind: "user" },
			{ id: "p2", name: "C++ (bot)", kind: "bot" },
		];
		expect(mentionedIn("@Anna Weber and @C++ (bot) here", people)).toEqual([
			"p1",
			"p2",
		]);
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

describe("inlineMentions", () => {
	const scout = { id: "bot_scout", name: "Scout" };

	test("links a spelled-out @Name in place", () => {
		expect(inlineMentions("hey @Scout, look", [scout])).toBe(
			"hey [@Scout](mention://bot_scout), look",
		);
	});

	test("leads with a mention the text doesn't spell out", () => {
		expect(inlineMentions("look", [scout])).toBe(
			"[@Scout](mention://bot_scout) look",
		);
	});

	test("does not touch longer names", () => {
		expect(inlineMentions("@Scouting", [scout])).toBe(
			"[@Scout](mention://bot_scout) @Scouting",
		);
	});
});

describe("spokenMentions", () => {
	const crew: Mentionable[] = [
		...options,
		{ id: "b3", name: "Mo Mo", kind: "bot" },
		{ id: "b4", name: "Éclair", kind: "bot" },
	];

	test("matches a bot name as a whole word in any case", () => {
		expect(spokenMentions("hey scout, what do you think?", crew)).toEqual([
			"b1",
		]);
		expect(spokenMentions("SCOUT", crew)).toEqual(["b1"]);
	});

	test("does not match inside longer words", () => {
		expect(spokenMentions("scouting the mascots", crew)).toEqual([]);
		expect(spokenMentions("the Mascot is here", crew)).toEqual(["b2"]);
	});

	test("never names people", () => {
		expect(spokenMentions("Ada said hi", crew)).toEqual([]);
	});

	test("handles names with spaces and accents", () => {
		expect(spokenMentions("ask mo mo about it", crew)).toEqual(["b3"]);
		expect(spokenMentions("an éclair please", crew)).toEqual(["b4"]);
		expect(spokenMentions("peclair", crew)).toEqual([]);
	});

	test("names several bots once each", () => {
		expect(spokenMentions("Scout and Mascot, scout again", crew)).toEqual([
			"b1",
			"b2",
		]);
	});
});

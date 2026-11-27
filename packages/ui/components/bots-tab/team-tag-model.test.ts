import { describe, expect, test } from "bun:test";
import type { IOrgView } from "../../state/backend-state/bot-team-state";
import {
	type TeamTagOption,
	addressesTeam,
	insertTeamTag,
	leadingTagQuery,
	leadingTags,
	matchTeamTags,
	teamTagOptions,
} from "./team-tag-model";

const options: TeamTagOption[] = [
	{ id: "1", name: "Security", tag: "Security" },
	{ id: "2", name: "IT Support", tag: "ITSupport", leadName: "Pebble" },
	{ id: "3", name: "People", tag: "People" },
];

describe("leadingTagQuery", () => {
	test("finds the token being typed at the start", () => {
		expect(leadingTagQuery("#Sec", 4)).toEqual({
			start: 0,
			end: 4,
			query: "Sec",
		});
		expect(leadingTagQuery("#", 1)).toEqual({ start: 0, end: 1, query: "" });
	});

	test("finds the second of several leading tags", () => {
		expect(leadingTagQuery("#Security #IT", 13)).toEqual({
			start: 10,
			end: 13,
			query: "IT",
		});
	});

	test("only leading tags count", () => {
		expect(leadingTagQuery("hello #Sec", 10)).toBeNull();
		expect(leadingTagQuery("#Security hello #Sec", 20)).toBeNull();
		expect(leadingTagQuery("hello", 5)).toBeNull();
		expect(leadingTagQuery("", 0)).toBeNull();
	});

	test("a caret outside every token finds nothing", () => {
		expect(leadingTagQuery("#Security hello", 15)).toBeNull();
	});

	test("leading whitespace is skipped like on the server", () => {
		expect(leadingTagQuery("  #Sec", 6)).toEqual({
			start: 2,
			end: 6,
			query: "Sec",
		});
	});
});

describe("matchTeamTags", () => {
	test("matches the prefix case-insensitively", () => {
		expect(matchTeamTags(options, "it").map((o) => o.id)).toEqual(["2"]);
		expect(matchTeamTags(options, "SE").map((o) => o.id)).toEqual(["1"]);
	});

	test("an empty prefix lists every team in tag order", () => {
		expect(matchTeamTags(options, "").map((o) => o.tag)).toEqual([
			"ITSupport",
			"People",
			"Security",
		]);
	});

	test("a tag that is not a prefix does not match", () => {
		expect(matchTeamTags(options, "support")).toEqual([]);
	});
});

describe("insertTeamTag", () => {
	test("replaces the token and leaves the caret after the space", () => {
		expect(
			insertTeamTag("#Sec", { start: 0, end: 4, query: "Sec" }, "Security"),
		).toEqual({ text: "#Security ", caret: 10 });
	});

	test("reuses a space that already follows", () => {
		expect(
			insertTeamTag(
				"#Sec hello",
				{ start: 0, end: 4, query: "Sec" },
				"Security",
			),
		).toEqual({ text: "#Security hello", caret: 10 });
	});

	test("keeps earlier tags", () => {
		expect(
			insertTeamTag(
				"#People #S",
				{ start: 8, end: 10, query: "S" },
				"Security",
			),
		).toEqual({ text: "#People #Security ", caret: 18 });
	});
});

describe("teamTagOptions", () => {
	test("tags drop the spaces of the name, carry the lead's name and skip teams without a lead", () => {
		const org = {
			teams: [
				{ id: "t", name: "IT Support", lead_bot_id: "b" },
				{ id: "u", name: "People", lead_bot_id: null },
			],
			bots: [],
			invitations: [],
		} as unknown as IOrgView;
		expect(
			teamTagOptions(org, (id) => (id === "b" ? "Pebble" : undefined)),
		).toEqual([
			{ id: "t", name: "IT Support", tag: "ITSupport", leadName: "Pebble" },
		]);
	});
});

describe("leadingTags", () => {
	test("reads the tags a message starts with, like the server", () => {
		expect(leadingTags("#Security #ITSupport, is the VPN down?")).toEqual([
			"Security",
			"ITSupport",
		]);
		expect(leadingTags("  #People: who approves leave?")).toEqual(["People"]);
		expect(leadingTags("#A #B #C #D hi")).toEqual(["A", "B", "C"]);
		expect(leadingTags("I asked #Security yesterday")).toEqual([]);
		expect(leadingTags("# Notes")).toEqual([]);
		expect(leadingTags("")).toEqual([]);
	});
});

describe("addressesTeam", () => {
	test("is true only for a leading tag of a known team", () => {
		expect(addressesTeam("#itsupport is the VPN down?", options)).toBe(true);
		expect(addressesTeam("#Unknown hi", options)).toBe(false);
		expect(addressesTeam("ask #Security later", options)).toBe(false);
		expect(addressesTeam("#Security", [])).toBe(false);
	});
});

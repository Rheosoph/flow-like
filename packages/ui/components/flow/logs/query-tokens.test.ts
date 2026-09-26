import { describe, expect, test } from "bun:test";
import {
	type INodeRef,
	levelsAfter,
	parseLevels,
	parseQueryInput,
	quoteValue,
	resolveNode,
	tokenize,
	trailingToken,
} from "./query-tokens";

const NODES: INodeRef[] = [
	{ id: "upsert", name: "db_upsert", friendlyName: "Upsert" },
	{ id: "http", name: "http_request", friendlyName: "HTTP Request" },
	{ id: "each", name: "for_each", friendlyName: "For Each" },
];

describe("tokenize", () => {
	test("splits keys, negation, phrases and words", () => {
		const tokens = tokenize(
			'node:Upsert -level:debug "in iteration" -"x y" -word bare',
		);
		expect(
			tokens.map(({ negated, key, value, quoted }) => ({
				negated,
				key,
				value,
				quoted,
			})),
		).toEqual([
			{ negated: false, key: "node", value: "Upsert", quoted: false },
			{ negated: true, key: "level", value: "debug", quoted: false },
			{ negated: false, key: undefined, value: "in iteration", quoted: true },
			{ negated: true, key: undefined, value: "x y", quoted: true },
			{ negated: true, key: undefined, value: "word", quoted: false },
			{ negated: false, key: undefined, value: "bare", quoted: false },
		]);
	});

	test("reads quoted node names and unclosed quotes", () => {
		const [node, open] = tokenize('NODE:"HTTP Request" "still typ');
		expect(node).toMatchObject({
			key: "node",
			value: "HTTP Request",
			open: false,
		});
		expect(open).toMatchObject({
			value: "still typ",
			quoted: true,
			open: true,
		});
	});
});

describe("resolveNode", () => {
	test("matches friendly names, then catalog names, case-insensitively", () => {
		expect(resolveNode("upsert", NODES)).toBe("upsert");
		expect(resolveNode("HTTP REQUEST", NODES)).toBe("http");
		expect(resolveNode("for_each", NODES)).toBe("each");
		expect(resolveNode("Ups", NODES)).toBeUndefined();
	});
});

describe("parseLevels", () => {
	test("accepts lists and aliases", () => {
		expect(parseLevels("error")).toEqual([3]);
		expect(parseLevels("warning,ERR")).toEqual([2, 3]);
		expect(parseLevels("loud")).toBeUndefined();
		expect(parseLevels("")).toBeUndefined();
	});
});

describe("parseQueryInput", () => {
	test("turns tokens into chips and level changes", () => {
		const parsed = parseQueryInput(
			'node:Upsert -node:"For Each" level:warn,error -level:debug "in iteration" -"loop" -noise word',
			NODES,
			{ commit: true },
		);
		expect(parsed.chips).toEqual([
			{ kind: "node", value: "upsert", negated: false },
			{ kind: "node", value: "each", negated: true },
			{ kind: "text", value: "in iteration", negated: false },
			{ kind: "text", value: "loop", negated: true },
			{ kind: "text", value: "noise", negated: true },
			{ kind: "text", value: "word", negated: false },
		]);
		expect(parsed.onlyLevels).toEqual([2, 3]);
		expect(parsed.hideLevels).toEqual([0]);
	});

	test("unknown nodes and levels stay text on commit only", () => {
		const input = "node:Nope -level:loud";
		expect(parseQueryInput(input, NODES, { commit: false }).chips).toEqual([]);
		expect(parseQueryInput(input, NODES, { commit: true }).chips).toEqual([
			{ kind: "text", value: "node:Nope", negated: false },
			{ kind: "text", value: "level:loud", negated: true },
		]);
	});

	test("a lone dash and empty keys filter nothing", () => {
		expect(parseQueryInput("foo -", NODES, { commit: false }).chips).toEqual([
			{ kind: "text", value: "foo", negated: false },
		]);
		expect(
			parseQueryInput("node: level:", NODES, { commit: true }).chips,
		).toEqual([]);
	});
});

describe("levelsAfter", () => {
	test("an include replaces the hidden set, an exclude adds to it", () => {
		expect(levelsAfter([1], { onlyLevels: [3], hideLevels: [] })).toEqual([
			0, 1, 2, 4,
		]);
		expect(levelsAfter([1], { onlyLevels: [], hideLevels: [0] })).toEqual([
			0, 1,
		]);
		expect(levelsAfter([], { onlyLevels: [2, 3], hideLevels: [2] })).toEqual([
			0, 1, 2, 4,
		]);
	});
});

describe("trailingToken", () => {
	test("returns the token being typed and what precedes it", () => {
		expect(trailingToken('node:Upsert -"in it')).toMatchObject({
			prefix: "node:Upsert ",
			token: { negated: true, value: "in it", open: true },
		});
		expect(trailingToken("level:error ")).toEqual({ prefix: "level:error " });
		expect(trailingToken("")).toEqual({ prefix: "" });
	});

	test("quotes values that need it", () => {
		expect(quoteValue("Upsert")).toBe("Upsert");
		expect(quoteValue("HTTP Request")).toBe('"HTTP Request"');
	});
});

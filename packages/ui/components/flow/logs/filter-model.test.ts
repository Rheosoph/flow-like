import { describe, expect, test } from "bun:test";
import {
	EMPTY_FILTER,
	type ILogFilter,
	addChip,
	chipId,
	errorsOnly,
	excludeNode,
	filterToQuery,
	invertLevels,
	levelChip,
	onlyLevel,
	queryKey,
	removeChip,
	scopeToNode,
	setChipUpstream,
	toggleChip,
	toggleLevel,
	withTimeRange,
} from "./filter-model";

describe("filterToQuery", () => {
	test("an empty filter is an empty query", () => {
		expect(filterToQuery(EMPTY_FILTER)).toEqual({});
	});

	test("maps every chip kind to its include or exclude list", () => {
		const filter: ILogFilter = {
			levelsOff: [0, 1],
			chips: [
				{ kind: "node", value: "upsert", negated: false },
				{ kind: "node", value: "each", negated: true },
				{ kind: "text", value: "iteration", negated: false },
				{ kind: "text", value: "O'Brien 100%", negated: true },
				{ kind: "group", value: "fp1", label: "x", negated: true },
				{ kind: "group", value: "fp2", label: "y", negated: false },
			],
		};
		expect(filterToQuery(filter)).toEqual({
			exclude_levels: [0, 1],
			nodes: ["upsert"],
			exclude_nodes: ["each"],
			text: ["iteration"],
			exclude_text: ["O'Brien 100%"],
			fingerprints: ["fp2"],
			exclude_fingerprints: ["fp1"],
		});
	});

	test("expands a node chip with its upstream nodes", () => {
		const filter = setChipUpstream(
			scopeToNode(EMPTY_FILTER, "upsert"),
			"node:upsert",
			true,
		);
		const query = filterToQuery(filter, {
			upstream: (id) => (id === "upsert" ? ["setf", "each", "opendb"] : []),
		});
		expect(query.nodes).toEqual(["upsert", "setf", "each", "opendb"]);
	});

	test("carries the fold list", () => {
		const fold = [{ fingerprint: "fp", first_start: 12 }];
		expect(filterToQuery(EMPTY_FILTER, { fold }).fold).toEqual(fold);
	});

	test("hiding every level still sends an explicit exclusion", () => {
		expect(
			filterToQuery({ levelsOff: [4, 3, 2, 1, 0], chips: [] }).exclude_levels,
		).toEqual([0, 1, 2, 3, 4]);
	});
});

describe("chips", () => {
	test("adding the same chip again replaces it in place", () => {
		let filter = addChip(EMPTY_FILTER, {
			kind: "text",
			value: "a",
			negated: false,
		});
		filter = addChip(filter, { kind: "text", value: "b", negated: false });
		filter = addChip(filter, { kind: "text", value: "a", negated: true });
		expect(filter.chips).toEqual([
			{ kind: "text", value: "a", negated: true },
			{ kind: "text", value: "b", negated: false },
		]);
	});

	test("toggling negates, and a negated node chip drops upstream", () => {
		const scoped = setChipUpstream(
			scopeToNode(EMPTY_FILTER, "n"),
			"node:n",
			true,
		);
		const toggled = toggleChip(scoped, "node:n");
		expect(toggled.chips).toEqual([
			{ kind: "node", value: "n", negated: true },
		]);
		expect(removeChip(toggled, "node:n").chips).toEqual([]);
	});

	test("scoping to a node replaces other node includes and its own exclusion", () => {
		let filter = addChip(EMPTY_FILTER, {
			kind: "node",
			value: "a",
			negated: false,
		});
		filter = excludeNode(filter, "b");
		filter = excludeNode(filter, "c");
		filter = addChip(filter, { kind: "text", value: "x", negated: false });
		const scoped = scopeToNode(filter, "b");
		expect(scoped.chips.map(chipId)).toEqual(["node:c", "text:x", "node:b"]);
		expect(scoped.chips.at(-1)).toEqual({
			kind: "node",
			value: "b",
			negated: false,
		});
	});
});

describe("levels", () => {
	test("toggle, only and invert", () => {
		expect(toggleLevel(EMPTY_FILTER, 0).levelsOff).toEqual([0]);
		expect(toggleLevel({ levelsOff: [0], chips: [] }, 0).levelsOff).toEqual([]);
		expect(onlyLevel(EMPTY_FILTER, 3).levelsOff).toEqual([0, 1, 2, 4]);
		expect(
			onlyLevel({ levelsOff: [0, 1, 2, 4], chips: [] }, 3).levelsOff,
		).toEqual([]);
		expect(invertLevels({ levelsOff: [0], chips: [] }).levelsOff).toEqual([
			1, 2, 3, 4,
		]);
	});

	test("the level chip reads as the shorter list", () => {
		expect(levelChip([])).toBeUndefined();
		expect(levelChip([0, 1, 4])).toEqual({ negated: false, levels: [2, 3] });
		expect(levelChip([0])).toEqual({ negated: true, levels: [0] });
		expect(levelChip([0, 1, 2, 3, 4])).toEqual({
			negated: true,
			levels: [0, 1, 2, 3, 4],
		});
	});
});

describe("query helpers", () => {
	test("queryKey ignores key order but not array order", () => {
		expect(queryKey({ text: ["a"], nodes: ["n"] })).toBe(
			queryKey({ nodes: ["n"], text: ["a"] }),
		);
		expect(queryKey({ text: ["a", "b"] })).not.toBe(
			queryKey({ text: ["b", "a"] }),
		);
		expect(
			queryKey({ fold: [{ fingerprint: "f", first_start: 1 }] }),
		).toContain("first_start");
	});

	test("time ranges and the error narrowing keep other filters", () => {
		expect(withTimeRange({ nodes: ["n"] }, undefined, 9)).toEqual({
			nodes: ["n"],
			to: 9,
		});
		expect(errorsOnly({ exclude_levels: [4], text: ["x"] })).toEqual({
			exclude_levels: [0, 1, 2, 4],
			text: ["x"],
		});
	});
});

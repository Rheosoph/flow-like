import { describe, expect, test } from "bun:test";
import type { ILogGroup } from "../../../lib/schema/flow/log-query";
import { EMPTY_FILTER } from "./filter-model";
import type { INodeRef } from "./query-tokens";
import {
	type ISuggestionContext,
	applySuggestion,
	buildSuggestions,
} from "./suggestions";

const NODES: INodeRef[] = [
	{ id: "upsert", name: "db_upsert", friendlyName: "Upsert" },
	{ id: "each", name: "for_each", friendlyName: "For Each" },
	{ id: "http", name: "http_request", friendlyName: "HTTP Request" },
	{ id: "open", name: "open_db", friendlyName: "Open Database" },
];

const LOOP: ILogGroup = {
	fingerprint: "fp_loop",
	log_level: 3,
	template: 'Error: ExecutionFailed("x") in iteration ⟨n⟩',
	slots: [{ min: 0, max: 985 }],
	count: 986,
	first_start: 1,
	last_start: 2,
	sample: "",
};

const CTX: ISuggestionContext = {
	nodes: NODES,
	nodeCounts: { upsert: 986, each: 987, http: 2 },
	levelCounts: [991, 4, 1, 1972, 0],
	groups: [LOOP],
};

describe("buildSuggestions", () => {
	test("an empty input offers busy nodes and levels that occur", () => {
		const ids = buildSuggestions("", CTX).map((s) => s.id);
		expect(ids).toEqual([
			"node:each",
			"node:upsert",
			"node:http",
			"level:0",
			"level:1",
			"level:2",
			"level:3",
		]);
	});

	test("a node key completes names, keeping negation", () => {
		expect(buildSuggestions("-node:up", CTX)).toEqual([
			{
				id: "node:-upsert",
				kind: "node",
				nodeId: "upsert",
				name: "Upsert",
				count: 986,
				negated: true,
			},
		]);
		expect(buildSuggestions("node:", CTX).map((s) => s.id)).toEqual([
			"node:each",
			"node:upsert",
			"node:http",
		]);
	});

	test("a level key completes level names", () => {
		expect(buildSuggestions("level:e", CTX).map((s) => s.id)).toEqual([
			"level:3",
		]);
	});

	test("free text offers only/hide first, then matching groups and nodes", () => {
		const ids = buildSuggestions('-"in iter', CTX).map((s) => s.id);
		expect(ids).toEqual(["text:-in iter", "text:in iter", "group:-fp_loop"]);
		expect(buildSuggestions("data", CTX).map((s) => s.id)).toEqual([
			"text:data",
			"text:-data",
			"node:open",
		]);
	});
});

describe("applySuggestion", () => {
	test("commits the typed prefix, then the suggestion", () => {
		const next = applySuggestion(
			EMPTY_FILTER,
			'level:error node:Upsert -"in it',
			buildSuggestions('-"in it', CTX)[2],
			NODES,
			(template) => template.slice(0, 8),
		);
		expect(next.levelsOff).toEqual([0, 1, 2, 4]);
		expect(next.chips).toEqual([
			{ kind: "node", value: "upsert", negated: false },
			{ kind: "group", value: "fp_loop", label: "Error: E", negated: true },
		]);
	});

	test("a level suggestion hides or isolates that level", () => {
		const hide = applySuggestion(
			EMPTY_FILTER,
			"-level:d",
			buildSuggestions("-level:d", CTX)[0],
			NODES,
			String,
		);
		expect(hide.levelsOff).toEqual([0]);
		const only = applySuggestion(
			EMPTY_FILTER,
			"level:w",
			buildSuggestions("level:w", CTX)[0],
			NODES,
			String,
		);
		expect(only.levelsOff).toEqual([0, 1, 3, 4]);
	});
});

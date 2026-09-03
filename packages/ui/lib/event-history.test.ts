import { describe, expect, test } from "bun:test";
import type { IEventTimelineEntry } from "../state/backend-state/event-state";
import {
	aggregateNodeSeverity,
	aggregateRunsByEventVersion,
	diffTimelineEntries,
	parseBoardVersionString,
	parseDottedEventVersion,
	toDottedVersionKey,
} from "./event-history";
import type { ILogMetadata } from "./schema/flow/log-metadata";

const run = (overrides: Partial<ILogMetadata> = {}): ILogMetadata => ({
	app_id: "app-1",
	board_id: "board-1",
	event_id: "evt-1",
	event_version: "1.0.0",
	log_level: 1,
	node_id: "node-1",
	payload: [],
	run_id: `run-${Math.random().toString(36).slice(2)}`,
	start: 1_000_000,
	end: 2_000_000,
	version: "v0-0-1",
	...overrides,
});

const entry = (
	overrides: Partial<IEventTimelineEntry> = {},
): IEventTimelineEntry => ({
	version: [1, 0, 0],
	version_key: "1.0.0",
	is_live: false,
	name: "Nightly import",
	description: "",
	event_type: "cron",
	active: true,
	board_id: "board-1",
	board_version: null,
	node_id: "node-1",
	default_page_id: null,
	route: null,
	is_default: false,
	execution_mode: "Local",
	exposure: "PUBLIC",
	created_at_ms: 0,
	updated_at_ms: 0,
	board_resolves: true,
	node_resolves: true,
	variable_ids: [],
	secret_variable_ids: [],
	notes_kind: null,
	...overrides,
});

describe("version format parsers", () => {
	// The two formats coexist on every run row: swap them and every run
	// groups under "unversioned" while board chips silently vanish.
	test("each parser REJECTS the other store format", () => {
		expect(parseDottedEventVersion("v1-0-4")).toBeNull();
		expect(parseBoardVersionString("1.0.4")).toBeNull();
	});

	test("each parser accepts exactly its own format", () => {
		expect(parseDottedEventVersion("1.0.4")).toEqual([1, 0, 4]);
		expect(parseBoardVersionString("v1-0-4")).toEqual([1, 0, 4]);
	});

	test("neither parser accepts hybrids or garbage", () => {
		for (const value of ["v1.0.4", "1-0-4", "1.0", "", null, undefined]) {
			expect(parseDottedEventVersion(value)).toBeNull();
			expect(parseBoardVersionString(value)).toBeNull();
		}
	});

	test("the grouping key is dotted, never the board form", () => {
		expect(toDottedVersionKey([1, 0, 4])).toBe("1.0.4");
		expect(toDottedVersionKey([1, 0, 4])).not.toBe("v1-0-4");
		expect(parseDottedEventVersion(toDottedVersionKey([2, 3, 4]))).toEqual([
			2, 3, 4,
		]);
	});
});

describe("aggregateRunsByEventVersion", () => {
	test("groups by dotted event version, newest version first", () => {
		const aggregates = aggregateRunsByEventVersion([
			run({ event_version: "1.0.0" }),
			run({ event_version: "1.0.2" }),
			run({ event_version: "1.0.0" }),
		]);
		expect(aggregates.map((a) => a.versionKey)).toEqual(["1.0.2", "1.0.0"]);
		expect(aggregates[1].total).toBe(2);
	});

	test("counts ok, warn and fail from the run log level", () => {
		const [aggregate] = aggregateRunsByEventVersion([
			run({ log_level: 0 }),
			run({ log_level: 1 }),
			run({ log_level: 2 }),
			run({ log_level: 3 }),
			run({ log_level: 4 }),
		]);
		expect(aggregate.total).toBe(5);
		expect(aggregate.ok).toBe(2);
		expect(aggregate.warn).toBe(1);
		expect(aggregate.fail).toBe(2);
	});

	test("derives p50/p95 from end - start", () => {
		const runs = [10, 20, 30, 40, 100].map((duration) =>
			run({ start: 0, end: duration }),
		);
		const [aggregate] = aggregateRunsByEventVersion(runs);
		expect(aggregate.p50DurationUs).toBe(30);
		expect(aggregate.p95DurationUs).toBe(100);
	});

	test("tracks first/last seen and distinct board versions", () => {
		const [aggregate] = aggregateRunsByEventVersion([
			run({ start: 300, end: 400, version: "v0-0-2" }),
			run({ start: 100, end: 200, version: "v0-0-1" }),
			run({ start: 500, end: 600, version: "v0-0-2" }),
		]);
		expect(aggregate.firstSeen).toBe(100);
		expect(aggregate.lastSeen).toBe(500);
		expect(aggregate.boardVersions).toEqual(["v0-0-2", "v0-0-1"]);
	});

	test("a board-formatted event_version is NOT a version group", () => {
		// The swap trap end-to-end: a run whose event_version accidentally
		// carries the board format must land in the unversioned bucket.
		const aggregates = aggregateRunsByEventVersion([
			run({ event_version: "v1-0-0" }),
			run({ event_version: null }),
		]);
		expect(aggregates).toHaveLength(1);
		expect(aggregates[0].versionKey).toBeNull();
		expect(aggregates[0].total).toBe(2);
	});

	test("unversioned runs sort last", () => {
		const aggregates = aggregateRunsByEventVersion([
			run({ event_version: null }),
			run({ event_version: "0.0.1" }),
		]);
		expect(aggregates.map((a) => a.versionKey)).toEqual(["0.0.1", null]);
	});
});

describe("aggregateNodeSeverity", () => {
	test("counts visits and keeps the worst severity per node", () => {
		const aggregates = aggregateNodeSeverity([
			run({
				nodes: [
					["node-a", 1],
					["node-b", 3],
				],
			}),
			run({
				nodes: [
					["node-a", 2],
					["node-b", 1],
				],
			}),
			run({ nodes: null }),
		]);
		expect(aggregates).toEqual([
			{ nodeId: "node-b", visits: 2, worstLevel: 3, warnRuns: 0, failRuns: 1 },
			{ nodeId: "node-a", visits: 2, worstLevel: 2, warnRuns: 1, failRuns: 0 },
		]);
	});

	test("ignores malformed node entries", () => {
		expect(
			aggregateNodeSeverity([run({ nodes: [[1, "node-a"], ["only-id"]] })]),
		).toEqual([]);
	});
});

describe("diffTimelineEntries", () => {
	test("returns nothing for identical entries", () => {
		expect(diffTimelineEntries(entry(), entry())).toEqual([]);
	});

	test("reports changed fields with readable values", () => {
		const diffs = diffTimelineEntries(
			entry(),
			entry({
				name: "Nightly import v2",
				board_version: [0, 0, 3],
				route: "/imports",
				variable_ids: ["var-a"],
			}),
		);
		expect(diffs.map((d) => d.field)).toEqual([
			"name",
			"board_version",
			"route",
			"variables",
		]);
		const boardVersion = diffs.find((d) => d.field === "board_version");
		expect(boardVersion).toEqual({
			field: "board_version",
			from: "latest",
			to: "v0.0.3",
		});
	});
});

import { describe, expect, test } from "bun:test";
import { ILogLevel } from "../../../lib/schema/flow/log";
import type { ILogQuery } from "../../../lib/schema/flow/log-query";
import { logStart } from "./log-format";
import { TEST_META, makeLog, memoryBoardState } from "./test-fixtures";
import {
	locateRow,
	neighbourError,
	targetOf,
	targetOfSummary,
} from "./use-log-navigation";

const MS = 1_000;
const T0 = 1_000_000_000;
const STEP = 2 * MS;
const LOOPS = 986;

/** The brief's failing run: a For Each whose Upsert fails on every iteration. */
function sampleRun() {
	const logs = [
		makeLog({
			node_id: "event",
			log_level: ILogLevel.Debug,
			start: T0,
			message: "Starting Node Execution: Retrieve Entities In Area [a1f3c9e2]",
		}),
		makeLog({
			node_id: "event",
			start: T0 + 1 * MS,
			message: "Event payload received",
		}),
		makeLog({
			node_id: "http",
			log_level: ILogLevel.Debug,
			start: T0 + 7 * MS,
			message: "Starting Node Execution: HTTP Request [7c21d0b4]",
		}),
		makeLog({
			node_id: "http",
			start: T0 + 7 * MS + 200,
			message: "POST https://overpass-api.de\nstatus: 200 OK",
		}),
		makeLog({
			node_id: "parse",
			log_level: ILogLevel.Debug,
			start: T0 + 1_849 * MS,
			message: "Starting Node Execution: Parse JSON [91c2e7f0]",
		}),
		makeLog({
			node_id: "print",
			start: T0 + 1_898 * MS,
			message: '{"type":"way"}',
		}),
		makeLog({
			node_id: "each",
			log_level: ILogLevel.Debug,
			start: T0 + 1_901 * MS,
			message: "Starting Node Execution: For Each [f09e11aa]",
		}),
	];
	for (let i = 0; i < LOOPS; i++) {
		const base = T0 + 1_901_400 + i * STEP;
		logs.push(
			makeLog({
				node_id: "setf",
				log_level: ILogLevel.Debug,
				fingerprint: "fp_setf",
				start: base,
				message: "Starting Node Execution: Set Field [3d8b7e05]",
			}),
			makeLog({
				node_id: "upsert",
				log_level: ILogLevel.Error,
				fingerprint: "fp_upsert",
				start: base + 700,
				message:
					"Failed to execute node: invalid type: null, expected struct NodeDBConnection",
			}),
			makeLog({
				node_id: "each",
				log_level: ILogLevel.Error,
				fingerprint: "fp_each",
				start: base + 1_300,
				message: `Error: ExecutionFailed("invalid type") in iteration ${i}`,
			}),
		);
		if (i === 0) {
			logs.push(
				makeLog({
					node_id: "setf",
					log_level: ILogLevel.Warn,
					start: base + 1_000,
					message: 'Field "geometry" did not exist',
				}),
			);
		}
	}
	return logs;
}

const LOGS = sampleRun();
const FIRST_ERROR = LOGS.find((log) => log.log_level === ILogLevel.Error);
const FOLD: ILogQuery = {
	fold: ["fp_setf", "fp_upsert", "fp_each"].map((fingerprint) => ({
		fingerprint,
		first_start: Math.min(
			...LOGS.filter((log) => log.fingerprint === fingerprint).map(logStart),
		),
	})),
};

describe("locateRow", () => {
	test("finds the first error by counting rows before it", async () => {
		if (!FIRST_ERROR) throw new Error("sample has no error");
		const state = memoryBoardState(LOGS);
		const found = await locateRow(state, TEST_META, {}, targetOf(FIRST_ERROR));
		expect(found?.index).toBe(8);
		expect(found?.log.node_id).toBe("upsert");
		expect(state.calls).toEqual(["count", "query 0+50"]);
	});

	test("resolves a summary entry under folding", async () => {
		if (!FIRST_ERROR) throw new Error("sample has no error");
		const summaryLog = {
			node_id: "upsert",
			log_level: 3,
			start: logStart(FIRST_ERROR),
			message: "Failed to execute node: invalid type",
			fingerprint: "fp_upsert",
		};
		const found = await locateRow(
			memoryBoardState(LOGS),
			TEST_META,
			FOLD,
			targetOfSummary(summaryLog),
		);
		expect(found?.index).toBe(8);
	});

	test("reports a row the filters hide", async () => {
		if (!FIRST_ERROR) throw new Error("sample has no error");
		const found = await locateRow(
			memoryBoardState(LOGS),
			TEST_META,
			{ exclude_levels: [3] },
			targetOf(FIRST_ERROR),
		);
		expect(found).toBeUndefined();
	});

	test("places rows that share a microsecond", async () => {
		const a = makeLog({ node_id: "a", start: 5, message: "a" });
		const b = makeLog({ node_id: "b", start: 5, message: "b" });
		const logs = [makeLog({ start: 1 }), a, b];
		expect(
			(await locateRow(memoryBoardState(logs), TEST_META, {}, targetOf(b)))
				?.index,
		).toBe(2);
	});
});

describe("neighbourError", () => {
	test("steps forward and back through errors", async () => {
		if (!FIRST_ERROR) throw new Error("sample has no error");
		const state = memoryBoardState(LOGS);
		const from = logStart(FIRST_ERROR);
		const next = await neighbourError(state, TEST_META, {}, from, 1);
		expect(next?.message).toContain("in iteration 0");
		const previous = await neighbourError(state, TEST_META, {}, from, -1);
		expect(previous).toBeUndefined();
		const last = await neighbourError(state, TEST_META, {}, undefined, -1);
		expect(last?.message).toContain(`in iteration ${LOOPS - 1}`);
	});

	test("with folding, errors are one row per group", async () => {
		const state = memoryBoardState(LOGS);
		const first = await neighbourError(state, TEST_META, FOLD, undefined, 1);
		expect(first?.node_id).toBe("upsert");
		if (!first) throw new Error("no folded error");
		const second = await neighbourError(
			state,
			TEST_META,
			FOLD,
			logStart(first),
			1,
		);
		expect(second?.message).toContain("in iteration 0");
		if (!second) throw new Error("no second folded error");
		const third = await neighbourError(
			state,
			TEST_META,
			FOLD,
			logStart(second),
			1,
		);
		expect(third).toBeUndefined();
	});
});

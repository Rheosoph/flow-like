import { describe, expect, test } from "bun:test";
import {
	assertionText,
	discoverBoardTests,
	eventAliasOf,
	isTestEventAlias,
	isTestEventNode,
	runBoardTest,
} from "./board-tests";
import type { INode } from "./schema/flow/board";
import type { ILog } from "./schema/flow/log";
import type { ILogMetadata } from "./schema/flow/log-metadata";

function makeNode(overrides: Partial<INode>): INode {
	return {
		id: "node-1",
		name: "events_simple",
		friendly_name: "Simple Event",
		...overrides,
	} as INode;
}

function makeLog(message: string, logLevel = 1): ILog {
	return { message, log_level: logLevel } as unknown as ILog;
}

const META = { run_id: "run-1" } as ILogMetadata;

describe("eventAliasOf", () => {
	test("camelCases the display name like FlowScript lowering", () => {
		expect(eventAliasOf(makeNode({ friendly_name: "Test Empty Cart" }))).toBe(
			"testEmptyCart",
		);
		expect(eventAliasOf(makeNode({ friendly_name: "testEmptyCart" }))).toBe(
			"testEmptyCart",
		);
	});

	test("falls back to the node name when there is no display name", () => {
		expect(eventAliasOf(makeNode({ friendly_name: undefined }))).toBe(
			"eventsSimple",
		);
		expect(eventAliasOf(makeNode({ friendly_name: "  " }))).toBe(
			"eventsSimple",
		);
	});

	test("matches FlowScript lowering for unicode and digit-leading names", () => {
		expect(eventAliasOf(makeNode({ friendly_name: "Test Größe" }))).toBe(
			"testGröße",
		);
		expect(eventAliasOf(makeNode({ friendly_name: "2 Test" }))).toBe("_2Test");
	});
});

describe("isTestEventAlias", () => {
	test("accepts test-prefixed aliases with a word boundary", () => {
		expect(isTestEventAlias("testEmptyCart")).toBe(true);
		expect(isTestEventAlias("test")).toBe(true);
		expect(isTestEventAlias("test2Checkout")).toBe(true);
	});

	test("rejects words that merely start with test", () => {
		expect(isTestEventAlias("testimonialFeed")).toBe(false);
		expect(isTestEventAlias("tester")).toBe(false);
		expect(isTestEventAlias("contest")).toBe(false);
	});
});

describe("isTestEventNode", () => {
	test("requires the start flag", () => {
		const named = { friendly_name: "testEmptyCart" };
		expect(isTestEventNode(makeNode({ ...named, start: true }))).toBe(true);
		expect(isTestEventNode(makeNode({ ...named, start: false }))).toBe(false);
		expect(isTestEventNode(makeNode(named))).toBe(false);
	});
});

describe("discoverBoardTests", () => {
	test("collects only test events and sorts by alias", () => {
		const nodes = {
			a: makeNode({ id: "a", friendly_name: "Test Zeta", start: true }),
			b: makeNode({ id: "b", friendly_name: "testAlpha", start: true }),
			c: makeNode({ id: "c", friendly_name: "dashboardLoad", start: true }),
			d: makeNode({ id: "d", friendly_name: "testHidden" }),
		};
		expect(discoverBoardTests(nodes).map((t) => t.alias)).toEqual([
			"testAlpha",
			"testZeta",
		]);
		expect(discoverBoardTests(undefined)).toEqual([]);
	});
});

describe("assertionText", () => {
	test("strips the marker prefix", () => {
		expect(assertionText(makeLog("ASSERT_FAIL total_matches {…}"))).toBe(
			"total_matches {…}",
		);
		expect(assertionText(makeLog("ASSERT_OK total_matches"))).toBe(
			"total_matches",
		);
	});
});

describe("runBoardTest", () => {
	const testCase = {
		node: makeNode({ friendly_name: "testEmptyCart", start: true }),
		alias: "testEmptyCart",
	};

	test("passes when every assertion holds and nothing errors", async () => {
		const result = await runBoardTest(
			async (_meta, query) =>
				query.startsWith("message") ? [makeLog("ASSERT_OK empty_total")] : [],
			async () => META,
			testCase,
		);
		expect(result.verdict).toBe("pass");
		expect(result.assertOk).toBe(1);
		expect(result.assertFail).toBe(0);
		expect(result.runId).toBe("run-1");
	});

	test("fails on ASSERT_FAIL markers", async () => {
		const result = await runBoardTest(
			async (_meta, query) =>
				query.startsWith("message")
					? [makeLog("ASSERT_FAIL empty_total expected 0", 3)]
					: [],
			async () => META,
			testCase,
		);
		expect(result.verdict).toBe("fail");
		expect(result.assertFail).toBe(1);
		expect(result.failedAssertions).toHaveLength(1);
	});

	test("fails when the run logged errors without assertions", async () => {
		const result = await runBoardTest(
			async (_meta, query) =>
				query.startsWith("log_level") ? [makeLog("boom", 3)] : [],
			async () => META,
			testCase,
		);
		expect(result.verdict).toBe("fail");
		expect(result.errorLogs).toHaveLength(1);
	});

	test("errors when the execution itself throws", async () => {
		const result = await runBoardTest(
			async () => [],
			async () => {
				throw new Error("board not found");
			},
			testCase,
		);
		expect(result.verdict).toBe("error");
		expect(result.executionError).toBe("board not found");
	});

	test("errors instead of passing when the run resolves without metadata", async () => {
		const result = await runBoardTest(
			async () => [],
			async () => undefined,
			testCase,
		);
		expect(result.verdict).toBe("error");
		expect(result.executionError).toContain("no metadata");
	});

	test("survives a failing log query", async () => {
		const result = await runBoardTest(
			async () => {
				throw new Error("lance unavailable");
			},
			async () => META,
			testCase,
		);
		expect(result.verdict).toBe("pass");
		expect(result.assertOk).toBe(0);
	});
});

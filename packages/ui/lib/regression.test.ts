import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { gradeBoardRun } from "./board-tests";
import {
	PASS_EXPECTATION,
	compareToExpectation,
	errorClassOf,
	isGateFailure,
} from "./regression";
import type { ILog } from "./schema/flow/log";
import type { ILogMetadata } from "./schema/flow/log-metadata";

interface ICompareFixtureCase {
	case: string;
	baseline: {
		verdict: "pass" | "fail" | "error";
		errorClass: string | null;
	};
	replay: {
		metadata: boolean;
		assertLogs: string[];
		errorLogs: string[];
		executionError: string | null;
		logQueryFailed: boolean;
	};
	expect: {
		outcome: "ok" | "regressed" | "still_failing" | "fixed";
		errorClassChanged: boolean | null;
		gateFailure: boolean;
		replayErrorClass: string | null;
	};
}

const fixture = JSON.parse(
	readFileSync(
		new URL(
			"../../core/tests/fixtures/board-test-grading.json",
			import.meta.url,
		),
		"utf8",
	),
) as { compare: ICompareFixtureCase[] };

const META = { run_id: "run-1" } as ILogMetadata;

function makeLog(message: string): ILog {
	return { message } as unknown as ILog;
}

function gradeReplay(replay: ICompareFixtureCase["replay"]) {
	return gradeBoardRun({
		metadata: replay.metadata ? META : undefined,
		assertLogs: replay.assertLogs.map(makeLog),
		errorLogs: replay.errorLogs.map(makeLog),
		executionError: replay.executionError ?? undefined,
		logQueryFailed: replay.logQueryFailed,
	});
}

describe("conformance fixture (Rust twin: flow::regression::compare)", () => {
	test("carries the compare section", () => {
		expect(fixture.compare.length).toBeGreaterThan(0);
	});

	for (const c of fixture.compare) {
		test(c.case, () => {
			const grade = gradeReplay(c.replay);
			expect(errorClassOf(grade)).toBe(c.expect.replayErrorClass);

			const outcome = compareToExpectation(
				{ verdict: c.baseline.verdict, error_class: c.baseline.errorClass },
				grade,
			);
			expect(outcome.outcome).toBe(c.expect.outcome);
			if (outcome.outcome === "still_failing") {
				expect(outcome.error_class_changed).toBe(
					c.expect.errorClassChanged as boolean,
				);
			} else {
				expect(c.expect.errorClassChanged).toBeNull();
			}
			expect(isGateFailure(outcome)).toBe(c.expect.gateFailure);
		});
	}
});

describe("authored tests", () => {
	test("compare against the pass expectation", () => {
		const failing = gradeBoardRun({
			metadata: META,
			assertLogs: [makeLog("ASSERT_FAIL total expected 1")],
			errorLogs: [],
		});
		const outcome = compareToExpectation(PASS_EXPECTATION, failing);
		expect(outcome.outcome).toBe("regressed");
		expect(isGateFailure(outcome)).toBe(true);
	});
});

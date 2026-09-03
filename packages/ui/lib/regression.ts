import {
	type IBoardRunGrade,
	LOG_QUERY_FAILED_MESSAGE,
	NO_METADATA_MESSAGE,
} from "./board-tests";

/**
 * Verdict-vs-baseline comparison — never output-vs-output. The replay is
 * graded by `gradeBoardRun` (the one grader); the case outcome compares that
 * verdict against the baseline verdict recorded at promotion. No goldens, no
 * stored outputs.
 *
 * Twinned with `compare_to_expectation`/`error_class_of` in core's
 * `flow::regression::compare` — a rule change here must also land there, held
 * together by the `compare` section of
 * `packages/core/tests/fixtures/board-test-grading.json`.
 */

export const ERROR_CLASS_ASSERT_FAIL = "assert_fail";
export const ERROR_CLASS_EXECUTION_ERROR = "execution_error";
export const ERROR_CLASS_ERROR_LOG = "error_log";
export const ERROR_CLASS_NO_METADATA = "no_metadata";
export const ERROR_CLASS_LOG_QUERY_FAILED = "log_query_failed";

/** The baseline a replay is compared against, recorded at promotion. */
export interface IRegressionCompareBaseline {
	verdict: "pass" | "fail" | "error";
	error_class?: string | null;
}

/** An authored `test*` case's expectation: always pass. */
export const PASS_EXPECTATION: IRegressionCompareBaseline = {
	verdict: "pass",
	error_class: null,
};

/**
 * The four case outcomes, serialized exactly like the Rust `CaseOutcome`
 * (`{"outcome": "..."}`, `still_failing` carrying `error_class_changed`):
 * - `ok` — baseline passed, replay passes.
 * - `regressed` — baseline passed, replay errors. **The gate signal.**
 * - `still_failing` — baseline failed, replay fails. Neutral; a changed error
 *   class is surfaced as info, never graded.
 * - `fixed` — baseline failed, replay passes. Good news, shown as such.
 */
export type IRegressionCaseOutcome =
	| { outcome: "ok" }
	| { outcome: "regressed" }
	| { outcome: "still_failing"; error_class_changed: boolean }
	| { outcome: "fixed" };

/** Whether this outcome fails the publish/promote gate. */
export function isGateFailure(outcome: IRegressionCaseOutcome): boolean {
	return outcome.outcome === "regressed";
}

/**
 * Coarse classification of a failing grade — deliberately structural, never
 * derived from output content: a thrown/synthesized execution error outranks
 * failed assertions, which outrank plain error-level logs. `null` for a
 * passing grade.
 */
export function errorClassOf(grade: IBoardRunGrade): string | null {
	switch (grade.verdict) {
		case "pass":
			return null;
		case "error":
			switch (grade.executionError) {
				case NO_METADATA_MESSAGE:
					return ERROR_CLASS_NO_METADATA;
				case LOG_QUERY_FAILED_MESSAGE:
					return ERROR_CLASS_LOG_QUERY_FAILED;
				default:
					return ERROR_CLASS_EXECUTION_ERROR;
			}
		case "fail":
			if (grade.executionError !== undefined)
				return ERROR_CLASS_EXECUTION_ERROR;
			if (grade.assertFail > 0) return ERROR_CLASS_ASSERT_FAIL;
			return ERROR_CLASS_ERROR_LOG;
	}
}

/**
 * Compare a replay's grade against the fixture's recorded baseline. Any
 * non-pass verdict counts as failing on both sides — an ungradable replay
 * (`error`) of a passing baseline is a regression, never a shrug. Authored
 * tests compare against `PASS_EXPECTATION`.
 */
export function compareToExpectation(
	baseline: IRegressionCompareBaseline,
	grade: IBoardRunGrade,
): IRegressionCaseOutcome {
	const baselineFailed = baseline.verdict !== "pass";
	const replayFailed = grade.verdict !== "pass";
	if (!baselineFailed && !replayFailed) return { outcome: "ok" };
	if (!baselineFailed && replayFailed) return { outcome: "regressed" };
	if (baselineFailed && replayFailed) {
		return {
			outcome: "still_failing",
			error_class_changed:
				(baseline.error_class ?? null) !== errorClassOf(grade),
		};
	}
	return { outcome: "fixed" };
}

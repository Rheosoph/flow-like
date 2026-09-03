import { toModuleIdent } from "./flow-modules";
import type { INode } from "./schema/flow/board";
import type { ILog } from "./schema/flow/log";
import type { ILogMetadata } from "./schema/flow/log-metadata";

/**
 * FlowScript camelCase alias of an event node's display name, so
 * "Test Empty Cart" and "testEmptyCart" resolve to the same alias
 * FlowScript lowering renders.
 */
export function eventAliasOf(
	node: Pick<INode, "name"> & { friendly_name?: string | null },
): string {
	return toModuleIdent((node.friendly_name ?? "").trim() || node.name);
}

export function isTestEventAlias(alias: string): boolean {
	if (!alias.toLowerCase().startsWith("test")) return false;
	// Require a word boundary after "test" so `testimonialFeed` is not a test.
	const next = alias.charAt(4);
	return next === "" || next === next.toUpperCase();
}

/** A board test is an event start node whose alias starts with `test`. */
export function isTestEventNode(
	node: Pick<INode, "name" | "start"> & { friendly_name?: string | null },
): boolean {
	return node.start === true && isTestEventAlias(eventAliasOf(node));
}

export interface IBoardTestCase {
	node: INode;
	alias: string;
}

export function discoverBoardTests(
	nodes: { [key: string]: INode } | undefined,
): IBoardTestCase[] {
	return Object.values(nodes ?? {})
		.filter((node) => node.start === true)
		.map((node) => ({ node, alias: eventAliasOf(node) }))
		.filter(({ alias }) => isTestEventAlias(alias))
		.sort((a, b) => a.alias.localeCompare(b.alias));
}

export type IBoardTestVerdict = "pass" | "fail" | "error";

/** Everything the verdict rule looks at, gathered by an execute adapter. */
export interface IBoardRunEvidence {
	metadata?: ILogMetadata;
	assertLogs: ILog[];
	errorLogs: ILog[];
	executionError?: string;
	/**
	 * True when the run's log store could not be queried. Defaults to `false`,
	 * which preserves the interactive degrade-to-pass behavior (pinned by
	 * board-tests.test.ts); a regression adapter sets it so an unreadable log
	 * store yields `error`, never a green light.
	 */
	logQueryFailed?: boolean;
}

export interface IBoardRunGrade {
	verdict: IBoardTestVerdict;
	assertOk: number;
	assertFail: number;
	failedAssertions: ILog[];
	/** The caller's execution error, or a synthesized one naming why the run was ungradable. */
	executionError?: string;
}

/** Twinned with `NO_METADATA_MESSAGE` in core's `flow::regression::grade`. */
export const NO_METADATA_MESSAGE =
	"The run returned no metadata, so its logs could not be graded.";
/** Twinned with `LOG_QUERY_FAILED_MESSAGE` in core's `flow::regression::grade`. */
export const LOG_QUERY_FAILED_MESSAGE =
	"The run's logs could not be queried, so the run could not be graded.";

/**
 * The one verdict rule: `ASSERT_FAIL` markers, error-level logs or a thrown
 * execution fail the run; a run without metadata (or, when `logQueryFailed`
 * is set, without readable logs) is an `error`, never a pass. Twinned with
 * `flow::regression::grade_run` in packages/core — a rule change here must
 * also land there, held together by
 * `packages/core/tests/fixtures/board-test-grading.json`.
 */
export function gradeBoardRun(evidence: IBoardRunEvidence): IBoardRunGrade {
	const assertOk = evidence.assertLogs.filter((log) =>
		log.message?.startsWith("ASSERT_OK"),
	).length;
	const failedAssertions = evidence.assertLogs.filter((log) =>
		log.message?.startsWith("ASSERT_FAIL"),
	);
	const logQueryFailed = evidence.logQueryFailed ?? false;
	let executionError = evidence.executionError;
	const hasFailures =
		failedAssertions.length > 0 ||
		evidence.errorLogs.length > 0 ||
		executionError !== undefined;
	// A run without metadata cannot be graded (remote backends may resolve
	// undefined) — never report it as a pass.
	if (!evidence.metadata && executionError === undefined) {
		executionError = NO_METADATA_MESSAGE;
	} else if (logQueryFailed && executionError === undefined) {
		executionError = LOG_QUERY_FAILED_MESSAGE;
	}
	const verdict: IBoardTestVerdict =
		!evidence.metadata || logQueryFailed
			? "error"
			: hasFailures
				? "fail"
				: "pass";
	return {
		verdict,
		assertOk,
		assertFail: failedAssertions.length,
		failedAssertions,
		executionError,
	};
}

export type IBoardRunLogQuery = (
	meta: ILogMetadata,
	query: string,
	offset?: number,
	limit?: number,
) => Promise<ILog[]>;

/**
 * Query a run's `ASSERT_*` markers and error-level logs into grading evidence.
 * A thrown query degrades to an empty list but is reported via
 * `logQueryFailed` — the caller decides whether to forward that flag to
 * `gradeBoardRun`.
 */
export async function collectRunEvidence(
	queryRun: IBoardRunLogQuery,
	metadata: ILogMetadata | undefined,
	executionError?: string,
): Promise<IBoardRunEvidence> {
	let logQueryFailed = false;
	let assertLogs: ILog[] = [];
	let errorLogs: ILog[] = [];
	if (metadata) {
		const swallow = (): ILog[] => {
			logQueryFailed = true;
			return [];
		};
		assertLogs = await queryRun(
			metadata,
			"message LIKE 'ASSERT_%'",
			0,
			100,
		).catch(swallow);
		errorLogs = await queryRun(metadata, "log_level >= 3", 0, 10).catch(
			swallow,
		);
	}
	return { metadata, assertLogs, errorLogs, executionError, logQueryFailed };
}

export interface IBoardTestResult {
	nodeId: string;
	alias: string;
	runId?: string;
	verdict: IBoardTestVerdict;
	assertOk: number;
	assertFail: number;
	failedAssertions: ILog[];
	errorLogs: ILog[];
	executionError?: string;
	durationMs: number;
	metadata?: ILogMetadata;
}

/** Strip the marker prefix: `ASSERT_FAIL label details` → `label details`. */
export function assertionText(log: ILog): string {
	return (log.message ?? "").replace(/^ASSERT_(OK|FAIL)\s*/, "");
}

/**
 * Execute one test event and grade it: `test::assert` logs stable
 * `ASSERT_OK {label}` / `ASSERT_FAIL {label} {details}` markers and any
 * error-level log or thrown execution error fails the test. Never rejects —
 * failures land in the verdict.
 */
export async function runBoardTest(
	queryRun: IBoardRunLogQuery,
	execute: (node: INode) => Promise<ILogMetadata | undefined>,
	test: IBoardTestCase,
): Promise<IBoardTestResult> {
	const startedAt = Date.now();
	let metadata: ILogMetadata | undefined;
	let executionError: string | undefined;
	try {
		metadata = await execute(test.node);
	} catch (error) {
		executionError = error instanceof Error ? error.message : String(error);
	}

	const evidence = await collectRunEvidence(queryRun, metadata, executionError);
	// Interactive runs deliberately degrade an unreadable log store to a pass
	// (pinned by board-tests.test.ts) — only regression adapters forward the flag.
	const grade = gradeBoardRun({ ...evidence, logQueryFailed: false });

	return {
		nodeId: test.node.id,
		alias: test.alias,
		runId: metadata?.run_id,
		verdict: grade.verdict,
		assertOk: grade.assertOk,
		assertFail: grade.assertFail,
		failedAssertions: grade.failedAssertions,
		errorLogs: evidence.errorLogs,
		executionError: grade.executionError,
		durationMs: Date.now() - startedAt,
		metadata,
	};
}

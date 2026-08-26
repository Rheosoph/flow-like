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
	queryRun: (
		meta: ILogMetadata,
		query: string,
		offset?: number,
		limit?: number,
	) => Promise<ILog[]>,
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

	let assertLogs: ILog[] = [];
	let errorLogs: ILog[] = [];
	if (metadata) {
		assertLogs = await queryRun(
			metadata,
			"message LIKE 'ASSERT_%'",
			0,
			100,
		).catch(() => []);
		errorLogs = await queryRun(metadata, "log_level >= 3", 0, 10).catch(
			() => [],
		);
	}

	const assertOk = assertLogs.filter((log) =>
		log.message?.startsWith("ASSERT_OK"),
	).length;
	const failedAssertions = assertLogs.filter((log) =>
		log.message?.startsWith("ASSERT_FAIL"),
	);
	const hasFailures =
		failedAssertions.length > 0 ||
		errorLogs.length > 0 ||
		executionError !== undefined;
	// A run without metadata cannot be graded (remote backends may resolve
	// undefined) — never report it as a pass.
	if (!metadata && executionError === undefined) {
		executionError =
			"The run returned no metadata, so its logs could not be graded.";
	}
	const verdict: IBoardTestVerdict = !metadata
		? "error"
		: hasFailures
			? "fail"
			: "pass";

	return {
		nodeId: test.node.id,
		alias: test.alias,
		runId: metadata?.run_id,
		verdict,
		assertOk,
		assertFail: failedAssertions.length,
		failedAssertions,
		errorLogs,
		executionError,
		durationMs: Date.now() - startedAt,
		metadata,
	};
}

import {
	type ILogMetadata,
	type IRegressionCompareBaseline,
	collectRunEvidence,
	compareToExpectation,
	errorClassOf,
	gradeBoardRun,
} from "@flow-like/flow-like-ui";
import type {
	IRegressionCaseResult,
	IRegressionFixtureBaseline,
	IRegressionRunAccepted,
	IRegressionSuiteConfig,
	IRegressionSuiteRunDetail,
	IRegressionSuiteRunSummary,
} from "@flow-like/flow-like-ui/state/backend-state/event-state";
import { createId } from "@paralleldrive/cuid2";
import { invoke } from "@tauri-apps/api/core";
import type { TauriBackend } from "../tauri-provider";

/**
 * The desktop regression-suite RUNNER (Track D lane F): cases execute
 * client-side through the ordinary local `executeBoard` path and are graded
 * with the shared grader + compare twins; progress is persisted per case into
 * the suite's JSON run archive (`persist_regression_suite_run`), which is the
 * desktop's only store for suite runs.
 *
 * Desktop replays are fully LIVE runs — there is no shadow isolation on the
 * local runtime at all: storage writes, WASM and outbound network all execute
 * for real. The suite's `allow_live_side_effects` acknowledgement is enforced
 * before anything runs (here for the message, and again by the plan command).
 */

const CASE_TIMEOUT_MS = 120_000;
const SUITE_WALL_CLOCK_MS = 15 * 60_000;
const STALE_RUN_GRACE_MS = 5 * 60_000;
const STALE_RUN_ERROR = "stale: app closed mid-run";
const CASE_TIMEOUT_MESSAGE =
	"replay did not complete within the 120s case timeout";

export const DESKTOP_LIVE_SIDE_EFFECTS_MESSAGE =
	"This suite has not acknowledged live side effects. Desktop replays run fully live on this device — every node executes for real, with no isolation of storage, network or WASM. Acknowledge live side effects on the suite before running it.";

interface IRegressionSuitePlan {
	suite_id: string;
	board_id: string;
	cases: IRegressionPlannedCase[];
	skipped_missing_node: string[];
	truncated: number;
	grading_blind: boolean;
}

type IRegressionPlannedCase =
	| {
			kind: "recorded_fixture";
			fixture_id: string;
			payload: unknown;
			source_node_id: string;
			baseline: IRegressionFixtureBaseline;
	  }
	| { kind: "authored_test"; node_id: string; alias: string };

function compareVersions(
	a: [number, number, number],
	b: [number, number, number],
): number {
	return a[0] - b[0] || a[1] - b[1] || a[2] - b[2];
}

function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

function withCaseTimeout<T>(promise: Promise<T>): Promise<T> {
	return new Promise((resolve, reject) => {
		const timer = setTimeout(
			() => reject(new Error(CASE_TIMEOUT_MESSAGE)),
			CASE_TIMEOUT_MS,
		);
		promise.then(
			(value) => {
				clearTimeout(timer);
				resolve(value);
			},
			(error) => {
				clearTimeout(timer);
				reject(error);
			},
		);
	});
}

function updateTallies(
	run: IRegressionSuiteRunSummary,
	cases: IRegressionCaseResult[],
): void {
	run.regressed = cases.filter((c) => c.outcome === "regressed").length;
	run.fixed = cases.filter((c) => c.outcome === "fixed").length;
	run.still_failing = cases.filter((c) => c.outcome === "still_failing").length;
	run.ok = cases.filter((c) => c.outcome === "ok").length;
	run.skipped = cases.filter((c) => c.outcome === "skipped").length;
}

async function persistArchive(
	appId: string,
	suiteId: string,
	archive: IRegressionSuiteRunDetail,
): Promise<void> {
	await invoke("persist_regression_suite_run", {
		appId,
		suiteId,
		run: archive,
	});
}

/**
 * Desktop liveness recovery: the case loop lives in this webview, so a RUNNING
 * archive whose run started longer ago than the wall clock plus grace can only
 * belong to an app that closed mid-run. Flip such runs to errored before a new
 * run begins; recovery failures never block the new run.
 */
async function recoverStaleRunningRuns(
	appId: string,
	eventId: string,
	suiteId: string,
): Promise<void> {
	let summaries: IRegressionSuiteRunSummary[];
	try {
		summaries = await invoke<IRegressionSuiteRunSummary[]>(
			"list_regression_suite_runs",
			{ appId, eventId },
		);
	} catch {
		return;
	}
	const cutoffMs = Date.now() - SUITE_WALL_CLOCK_MS - STALE_RUN_GRACE_MS;
	for (const summary of summaries) {
		if (summary.status !== "running") continue;
		const startedMs = Date.parse(summary.started_at ?? summary.created_at);
		if (!Number.isFinite(startedMs) || startedMs > cutoffMs) continue;
		try {
			const archive = await invoke<IRegressionSuiteRunDetail>(
				"get_regression_suite_run",
				{ appId, eventId, suiteRunId: summary.id },
			);
			archive.run.status = "errored";
			archive.run.error = STALE_RUN_ERROR;
			archive.run.completed_at = new Date().toISOString();
			await persistArchive(appId, suiteId, archive);
		} catch (error) {
			console.error("[Regression] Failed to recover a stale suite run:", error);
		}
	}
}

/**
 * Resolve the candidate: a pinned version, the newest published version (the
 * default — the gate-shaped question is about a fresh version), or the draft
 * head (`allowDraft`).
 */
async function resolveCandidate(
	backend: TauriBackend,
	appId: string,
	boardId: string,
	options?: { boardVersion?: [number, number, number]; allowDraft?: boolean },
): Promise<{ version?: [number, number, number]; label: string }> {
	if (options?.boardVersion) {
		return {
			version: options.boardVersion,
			label: options.boardVersion.join("."),
		};
	}
	if (options?.allowDraft) {
		return { version: undefined, label: "draft" };
	}
	const versions = await backend.boardState.getBoardVersions(appId, boardId);
	if (!versions || versions.length === 0) {
		throw new Error(
			"This board has no published version yet — publish one, or run against the draft head.",
		);
	}
	const newest = [...versions].sort(compareVersions).at(-1) as [
		number,
		number,
		number,
	];
	return { version: newest, label: newest.join(".") };
}

/**
 * Start a suite run: plan the cases (Rust, core `plan_suite_cases`), persist
 * the RUNNING archive so `getRegressionRun` polls see it immediately, detach
 * the case loop and return the accepted handle — the same fire-and-forget
 * contract as the cloud's 202.
 */
export async function startRegressionSuiteRun(
	backend: TauriBackend,
	appId: string,
	eventId: string,
	suite: IRegressionSuiteConfig,
	options?: { boardVersion?: [number, number, number]; allowDraft?: boolean },
): Promise<IRegressionRunAccepted> {
	if (!suite.allow_live_side_effects) {
		throw new Error(DESKTOP_LIVE_SIDE_EFFECTS_MESSAGE);
	}

	const candidate = await resolveCandidate(
		backend,
		appId,
		suite.board_id,
		options,
	);
	const plan = await invoke<IRegressionSuitePlan>("plan_regression_suite_run", {
		appId,
		eventId,
		boardVersion: candidate.version,
	});

	await recoverStaleRunningRuns(appId, eventId, plan.suite_id);

	const suiteRunId = createId();
	const createdAt = new Date().toISOString();
	const skippedCases: IRegressionCaseResult[] = plan.skipped_missing_node.map(
		(fixtureId) => ({
			id: createId(),
			case_kind: "recorded_fixture",
			case_ref: fixtureId,
			replay_run_id: null,
			outcome: "skipped",
			grade_verdict: "skipped",
			detail: {
				reason: "source node absent from the candidate board version",
			},
			duration_ms: null,
		}),
	);
	const archive: IRegressionSuiteRunDetail = {
		run: {
			id: suiteRunId,
			board_version: candidate.label,
			trigger: "manual",
			status: "running",
			regressed: 0,
			fixed: 0,
			still_failing: 0,
			ok: 0,
			skipped: skippedCases.length,
			started_at: createdAt,
			completed_at: null,
			error: null,
			created_at: createdAt,
		},
		cases: skippedCases,
	};
	await persistArchive(appId, plan.suite_id, archive);

	// Fire-and-forget: failures land on the archived run as `errored`, never
	// on this promise.
	void executeSuiteCases(backend, appId, plan, candidate.version, archive);

	return { suite_run_id: suiteRunId, status: "running" };
}

async function executeSuiteCases(
	backend: TauriBackend,
	appId: string,
	plan: IRegressionSuitePlan,
	version: [number, number, number] | undefined,
	archive: IRegressionSuiteRunDetail,
): Promise<void> {
	const startedMs = Date.now();
	try {
		for (const plannedCase of plan.cases) {
			if (Date.now() - startedMs > SUITE_WALL_CLOCK_MS) {
				throw new Error("suite run exceeded the 15 minute wall clock");
			}
			const result = await runSuiteCase(
				backend,
				appId,
				plan.board_id,
				version,
				plan.grading_blind,
				plannedCase,
			);
			archive.cases.push(result);
			updateTallies(archive.run, archive.cases);
			await persistArchive(appId, plan.suite_id, archive);
		}
		archive.run.status = "completed";
	} catch (error) {
		archive.run.status = "errored";
		archive.run.error = errorMessage(error);
	}
	archive.run.completed_at = new Date().toISOString();
	updateTallies(archive.run, archive.cases);
	try {
		await persistArchive(appId, plan.suite_id, archive);
	} catch (error) {
		console.error(
			"[Regression] Failed to persist the finished suite run:",
			error,
		);
	}
}

/**
 * Dispatch one case through the local board-invoke path and grade it
 * verdict-vs-baseline. Every failure mode lands as a graded case row — a case
 * never disappears silently.
 */
async function runSuiteCase(
	backend: TauriBackend,
	appId: string,
	boardId: string,
	version: [number, number, number] | undefined,
	gradingBlind: boolean,
	plannedCase: IRegressionPlannedCase,
): Promise<IRegressionCaseResult> {
	const isRecorded = plannedCase.kind === "recorded_fixture";
	const nodeId = isRecorded ? plannedCase.source_node_id : plannedCase.node_id;
	// A recorded Null payload replays as null, mirroring the cloud runner.
	const payload = isRecorded ? (plannedCase.payload ?? null) : {};
	const baseline: IRegressionCompareBaseline = isRecorded
		? {
				verdict: plannedCase.baseline.verdict,
				error_class: plannedCase.baseline.error_class ?? null,
			}
		: { verdict: "pass", error_class: null };

	const startedAt = Date.now();
	let runId: string | undefined;
	let metadata: ILogMetadata | undefined;
	let executionError: string | undefined;
	try {
		metadata = await withCaseTimeout(
			backend.boardState.executeBoard(
				appId,
				boardId,
				{ id: nodeId, payload, version },
				false,
				(id) => {
					runId = id;
				},
				() => {},
				false,
			),
		);
	} catch (error) {
		executionError = errorMessage(error);
	}

	// Recover the run's metadata by run id when execution resolved without it
	// (the runtime executor's adapter pattern), so the run can still be graded.
	if (!metadata && runId) {
		const recoveryId = runId;
		metadata = await backend.boardState
			.listRuns(
				appId,
				boardId,
				nodeId,
				(startedAt - 60_000) * 1000,
				undefined,
				undefined,
				undefined,
				0,
				100,
			)
			.then((runs) => runs.find((run) => run.run_id === recoveryId))
			.catch(() => undefined);
	}

	const evidence = await collectRunEvidence(
		(meta, query, offset, limit) =>
			backend.boardState.queryRun(meta, query, offset, limit),
		metadata,
		executionError,
	);
	// Regression adapters forward `logQueryFailed`: an unreadable log store
	// grades `error`, never the interactive degrade-to-pass.
	const grade = gradeBoardRun(evidence);
	const outcome = compareToExpectation(baseline, grade);

	const detail: Record<string, unknown> = {
		error_class: errorClassOf(grade),
		baseline_verdict: baseline.verdict,
		baseline_error_class: baseline.error_class ?? null,
		assert_ok: grade.assertOk,
		assert_fail: grade.assertFail,
		failed_assertions: grade.failedAssertions.map((log) => log.message ?? ""),
		execution_error: grade.executionError ?? null,
	};
	if (outcome.outcome === "still_failing") {
		detail.error_class_changed = outcome.error_class_changed;
	}
	if (gradingBlind) {
		detail.grading_blind = true;
	}
	if (!isRecorded) {
		detail.alias = plannedCase.alias;
	}

	return {
		id: createId(),
		case_kind: plannedCase.kind,
		case_ref: isRecorded ? plannedCase.fixture_id : plannedCase.node_id,
		replay_run_id: metadata?.run_id ?? runId ?? null,
		outcome: outcome.outcome,
		grade_verdict: grade.verdict,
		detail,
		duration_ms: Date.now() - startedAt,
	};
}

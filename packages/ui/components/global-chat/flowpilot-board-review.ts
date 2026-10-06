import type {
	BoardEditJob,
	BoardEditJobResolution,
} from "../../lib/schema/copilot";
import type { IBoardState } from "../../state/backend-state/board-state";
import type { FlowPilotBoardRunActivity } from "./flowpilot-board-activity";

type ReviewBackend = Pick<
	IBoardState,
	"listBoardEditJobs" | "getBoardEditJob" | "resolveBoardEditJob"
>;

export function boardReviewSummary(job: BoardEditJob) {
	return {
		job_id: job.jobId,
		app_id: job.appId,
		board_id: job.boardId,
		phase: job.phase,
		applied:
			job.phase === "applied" || job.phase === "applied_pending_delivery",
		command_count: job.review.commandCount,
		command_counts: job.review.commandCounts,
		destructive_effects: job.review.destructiveEffects,
		replacement_mode: job.review.replacementMode,
		draft_id: job.token.draft_id,
		revision: job.token.revision,
		created_at_ms: job.createdAtMs,
		updated_at_ms: job.updatedAtMs,
		error: job.error,
		runtime_verified: false,
	};
}

/** Resolve a retained compiler batch without starting another board specialist. */
export async function executeFlowPilotBoardReview(options: {
	boardState: ReviewBackend;
	appId: string;
	boardId: string;
	jobId?: string;
	action: string;
	getVisibleAppIds: () => Promise<Set<string>>;
	assertActive: () => void;
	getActiveRun?: (
		appId: string,
		boardId: string,
	) => FlowPilotBoardRunActivity | undefined;
	approve: (
		job: BoardEditJob,
		action: "apply" | "dismiss",
	) => Promise<boolean | null>;
	onJob: (job: BoardEditJob) => void;
	onApplied: (
		resolution: BoardEditJobResolution,
	) => Promise<Record<string, unknown>>;
}): Promise<Record<string, unknown>> {
	const { boardState, appId, boardId, jobId, action } = options;
	const fail = (code: string, message: string) => ({
		status: "error",
		code,
		message,
	});
	if (
		!appId ||
		!boardId ||
		!["list", "status", "apply", "dismiss"].includes(action)
	) {
		return fail(
			"BOARD_REVIEW_INVALID_TARGET",
			"Specify action, app_id, and board_id for the retained review.",
		);
	}
	if (action !== "list" && !jobId) {
		return fail(
			"BOARD_REVIEW_JOB_REQUIRED",
			"List the board reviews, then pass the exact job_id.",
		);
	}
	if (!(await options.getVisibleAppIds()).has(appId)) {
		return fail(
			"BOARD_REVIEW_APP_OUT_OF_SCOPE",
			"The requested app is not visible in the current profile.",
		);
	}
	options.assertActive();
	const matchesTarget = (job: BoardEditJob) =>
		job.appId === appId && job.boardId === boardId;
	const finishApplied = async (resolution: BoardEditJobResolution) => {
		try {
			return await options.onApplied(resolution);
		} catch (error) {
			return {
				persisted_readback_verified: false,
				diagnostics: [
					`The native apply completed, but receipt delivery or readback failed: ${error instanceof Error ? error.message : String(error)}`,
				],
			};
		}
	};
	if (action === "list") {
		if (!boardState.listBoardEditJobs) {
			return fail(
				"BOARD_REVIEW_UNAVAILABLE",
				"This backend cannot list retained workflow reviews.",
			);
		}
		// Include terminal jobs so a later turn can discover that Apply already completed.
		const jobs = (await boardState.listBoardEditJobs(appId, boardId, true))
			.filter(matchesTarget)
			.sort((a, b) => b.createdAtMs - a.createdAtMs);
		options.assertActive();
		const activeRun = options.getActiveRun?.(appId, boardId);
		return {
			status: "ok",
			app_id: appId,
			board_id: boardId,
			reviews: jobs.map(boardReviewSummary),
			...(activeRun
				? {
						active_run: activeRun,
						message:
							"A board operation is still running. This list covers retained native reviews; generation may still be preparing a review. Use flowpilot_board mode=inspect to check its progress; wait for its result before retrying an edit.",
					}
				: {}),
		};
	}
	if (!boardState.getBoardEditJob) {
		return fail(
			"BOARD_REVIEW_UNAVAILABLE",
			"This backend cannot recover retained workflow reviews.",
		);
	}
	const retained = await boardState.getBoardEditJob(jobId ?? "");
	options.assertActive();
	if (!retained || !matchesTarget(retained) || retained.jobId !== jobId) {
		return fail(
			"BOARD_REVIEW_NOT_FOUND",
			"No retained review matches this job_id, app_id, and board_id.",
		);
	}
	let job: BoardEditJob = retained;
	options.onJob(job);
	if (action === "status") {
		return { status: "ok", ...boardReviewSummary(job) };
	}
	const applied = () =>
		job.phase === "applied" || job.phase === "applied_pending_delivery";
	if (applied()) {
		return {
			status: "applied",
			...boardReviewSummary(job),
			message:
				"This review was already applied. No board commands were repeated.",
			...(action === "apply"
				? await finishApplied({ job, transitioned: false })
				: {}),
		};
	}
	if (
		job.phase !== "awaiting_approval" &&
		job.phase !== "failed" &&
		!(action === "apply" && job.phase === "applying") &&
		!(action === "dismiss" && job.phase === "stale")
	) {
		return {
			status: job.phase,
			...boardReviewSummary(job),
			message: `The retained review is ${job.phase}.`,
		};
	}
	if (!boardState.resolveBoardEditJob) {
		return fail(
			"BOARD_REVIEW_UNAVAILABLE",
			"This backend cannot resolve retained workflow reviews.",
		);
	}
	const approved = await options.approve(job, action as "apply" | "dismiss");
	options.assertActive();
	if (approved !== true) {
		return {
			status: approved === false ? "denied" : "awaiting_approval",
			...boardReviewSummary(job),
			message: "The requested review action was not approved.",
		};
	}
	const resolution = await boardState.resolveBoardEditJob(
		job.jobId,
		action === "apply",
		action === "apply",
	);
	job = resolution.job;
	if (!matchesTarget(job) || job.jobId !== jobId) {
		return fail(
			"BOARD_REVIEW_IDENTITY_MISMATCH",
			"The native resolution returned a different review identity.",
		);
	}
	options.onJob(job);
	return {
		status: applied() ? "applied" : job.phase,
		...boardReviewSummary(job),
		...(applied() ? await finishApplied(resolution) : {}),
	};
}

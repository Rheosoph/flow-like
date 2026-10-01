import type { IBoardState } from "../../state/backend-state/board-state";
import type { BoardEditJob } from "../schema/copilot";
import { deliverBoardEditJobReceipt } from "./board-edit-job-delivery";

type BoardEditJobRecoveryBackend = Pick<
	IBoardState,
	| "getBoardEditJob"
	| "resolveBoardEditJob"
	| "claimBoardEditJobDelivery"
	| "ackBoardEditJobDelivery"
>;

// Match board_edit_job_reserves_mutation in the native board_jobs module.
const UNAPPLIED_FAILURE_CODES = new Set([
	"IR_COMMIT_PERSISTENCE_UNAVAILABLE",
	"IR_COMMIT_CATALOG_UNAVAILABLE",
	"IR_COMMIT_APP_UNAVAILABLE",
	"IR_COMMIT_DESTRUCTIVE_APPROVAL_DENIED",
	"IR_COMMIT_APPLY_FAILED",
	"IR_COMMIT_FINGERPRINT_FAILED",
	"IR_COMMIT_RECEIPT_PERSISTENCE_FAILED",
]);

export function isBlockingBoardEditJob(job: BoardEditJob): boolean {
	if (job.phase === "failed") {
		return !(
			job.result?.status === "error" &&
			!job.result.replayed &&
			UNAPPLIED_FAILURE_CODES.has(job.result.code ?? "")
		);
	}
	return job.phase === "applying" || job.phase === "applied_pending_delivery";
}

function isRecoverableBoardEditJob(job: BoardEditJob): boolean {
	return (
		job.phase === "applying" ||
		job.phase === "failed" ||
		job.phase === "applied_pending_delivery"
	);
}

/** Recover a board's retained edit after its original chat or panel has closed. */
export async function recoverBoardEditJob({
	boardState,
	job,
	action,
	replayReceipt,
}: {
	boardState: BoardEditJobRecoveryBackend;
	job: BoardEditJob;
	action: "retry" | "dismiss";
	replayReceipt: Parameters<
		typeof deliverBoardEditJobReceipt
	>[0]["replayReceipt"];
}): Promise<{ job: BoardEditJob; message?: string }> {
	let current = job;
	if (boardState.getBoardEditJob) {
		const latest = await boardState.getBoardEditJob(job.jobId);
		if (!latest) {
			throw new Error(
				"This FlowPilot edit is no longer available. Refresh the board.",
			);
		}
		current = latest;
	}
	if (!isRecoverableBoardEditJob(current)) return { job: current };

	if (current.phase !== "applied_pending_delivery") {
		if (action === "dismiss" && current.phase !== "failed") {
			return {
				job: current,
				message:
					"This FlowPilot edit may still be applying. Retry recovery before dismissing it.",
			};
		}
		const resolveJob = boardState.resolveBoardEditJob;
		if (!resolveJob) {
			throw new Error(
				"This backend cannot recover the retained FlowPilot edit.",
			);
		}
		const approved = action === "retry";
		// The recovery UI presents the retained destructive review before Retry.
		const resolution = await resolveJob.call(
			boardState,
			current.jobId,
			approved,
			approved,
		);
		current = resolution.job;
	}

	if (current.phase === "applied_pending_delivery") {
		const delivery = await deliverBoardEditJobReceipt({
			boardState,
			job: current,
			replayReceipt,
			historyMode: "invalidate",
		});
		return {
			job: delivery.job,
			...(delivery.status !== "delivered" && delivery.status !== "settled"
				? { message: delivery.message }
				: {}),
		};
	}

	return {
		job: current,
		...(isRecoverableBoardEditJob(current)
			? {
					message:
						current.error ||
						current.result?.message ||
						"This FlowPilot edit still needs recovery. Try again after the current attempt finishes.",
				}
			: {}),
	};
}

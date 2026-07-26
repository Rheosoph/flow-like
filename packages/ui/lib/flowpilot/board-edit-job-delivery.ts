import type {
	IApplyFlowIrCommitResponse,
	IBoardState,
} from "../../state/backend-state/board-state";
import type {
	BoardEditJob,
	BoardEditJobResolution,
	FlowIrCommitToken,
} from "../schema/copilot";

type BoardEditJobDeliveryBackend = Pick<
	IBoardState,
	"claimBoardEditJobDelivery" | "ackBoardEditJobDelivery"
>;

export const DIRECT_FLOWPILOT_BOARD_EDIT_REQUEST_PREFIX = "flowpilot:";

export type BoardEditReceiptHistoryMode = "append" | "invalidate";

/** One bounded identity for native replay, remote idempotency, and renderer history. */
export function flowIrCommitDeliveryId(token: FlowIrCommitToken): string {
	return `flowpilot-board-edit:claim:${token.claim_id}`;
}

/**
 * Append undo only when this resolver performed the native mutation now. A
 * durable receipt replay has an uncertain prior ordering boundary, so its old
 * inverse batch must never be placed on top of newer history.
 */
export function boardEditJobResolutionHistoryMode(
	resolution: BoardEditJobResolution,
): BoardEditReceiptHistoryMode {
	return resolution.transitioned && !resolution.job.result?.replayed
		? "append"
		: "invalidate";
}

export function isDirectFlowPilotBoardEditJob(job: BoardEditJob): boolean {
	return Boolean(
		job.requestId?.startsWith(DIRECT_FLOWPILOT_BOARD_EDIT_REQUEST_PREFIX),
	);
}

export type BoardEditJobDeliveryOutcome =
	| {
			status: "delivered";
			job: BoardEditJob;
			receipt: IApplyFlowIrCommitResponse;
	  }
	| {
			status: "settled" | "busy" | "not_ready" | "unsupported";
			job: BoardEditJob;
			message: string;
	  }
	| {
			status: "replay_failed";
			job: BoardEditJob;
			receipt: IApplyFlowIrCommitResponse;
			message: string;
	  };

/**
 * Finish the renderer-owned half of a native board edit under one active lease.
 *
 * The native mutation has already happened when a job reaches
 * `applied_pending_delivery`. A short native lease elects one renderer to replay
 * the cached receipt through its normal sync/history adapter. Only that renderer
 * may acknowledge the job as fully applied; crashes leave it durable and
 * retryable after the lease expires. Receipt replay therefore remains
 * at-least-once across a crash between replay and acknowledgement, so replay
 * adapters must remain idempotent.
 */
export async function deliverBoardEditJobReceipt({
	boardState,
	job,
	replayReceipt,
	historyMode = "invalidate",
}: {
	boardState: BoardEditJobDeliveryBackend;
	job: BoardEditJob;
	replayReceipt: (
		token: FlowIrCommitToken,
		deliveryId: string,
		historyMode: BoardEditReceiptHistoryMode,
	) => Promise<IApplyFlowIrCommitResponse>;
	/** Immediate, continuously gated applies may append undo; crash recovery must invalidate it. */
	historyMode?: BoardEditReceiptHistoryMode;
}): Promise<BoardEditJobDeliveryOutcome> {
	const deliveryId = flowIrCommitDeliveryId(job.token);
	if (job.phase === "applied") {
		return {
			status: "settled",
			job,
			message: "The native board-edit receipt was already delivered.",
		};
	}
	if (job.phase !== "applied_pending_delivery") {
		return {
			status: "not_ready",
			job,
			message: `The board-edit job is not ready for receipt delivery (${job.phase}).`,
		};
	}

	const claimDelivery = boardState.claimBoardEditJobDelivery;
	const acknowledgeDelivery = boardState.ackBoardEditJobDelivery;
	if (!claimDelivery || !acknowledgeDelivery) {
		return {
			status: "unsupported",
			job,
			message:
				"This backend cannot safely synchronize and acknowledge the applied board-edit receipt.",
		};
	}

	const claim = await claimDelivery.call(boardState, job.jobId);
	if (claim.job.phase === "applied") {
		return {
			status: "settled",
			job: claim.job,
			message: "Another renderer already delivered the board-edit receipt.",
		};
	}
	if (claim.job.phase !== "applied_pending_delivery") {
		return {
			status: "not_ready",
			job: claim.job,
			message: `The board-edit job changed before receipt delivery (${claim.job.phase}).`,
		};
	}
	if (!claim.claimed || !claim.deliveryLeaseId) {
		return {
			status: "busy",
			job: claim.job,
			message:
				"Another renderer is synchronizing the applied board-edit receipt. It remains durable until delivery completes.",
		};
	}

	const receipt = await replayReceipt(claim.job.token, deliveryId, historyMode);
	if (receipt.status !== "applied" || receipt.delivery_complete !== true) {
		return {
			status: "replay_failed",
			job: claim.job,
			receipt,
			message:
				receipt.message ||
				"The native mutation succeeded, but its renderer receipt is not durably synchronized yet.",
		};
	}

	const acknowledgedJob = await acknowledgeDelivery.call(
		boardState,
		job.jobId,
		claim.deliveryLeaseId,
	);
	return { status: "delivered", job: acknowledgedJob, receipt };
}

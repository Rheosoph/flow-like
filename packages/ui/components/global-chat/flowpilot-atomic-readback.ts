import type { BoardEditJobDeliveryOutcome } from "../../lib/flowpilot/board-edit-job-delivery";
import type {
	IApplyFlowIrCommitResponse,
	IBoardState,
} from "../../state/backend-state/board-state";

type Verification =
	| { verified: true }
	| { verified: false; diagnostic: string };

/** Successful jobs omit their command receipt; delivery retrieves it under the native lease. */
export async function verifyAtomicBoardDeliveryReadback({
	delivery,
	...target
}: Omit<Parameters<typeof verifyAtomicBoardReadback>[0], "result"> & {
	delivery: BoardEditJobDeliveryOutcome;
}): Promise<Verification> {
	if (
		delivery.job.appId !== target.appId ||
		delivery.job.boardId !== target.boardId
	) {
		return {
			verified: false,
			diagnostic:
				"PERSISTED_BOARD_IDENTITY_MISMATCH: The delivery job belongs to another app or board.",
		};
	}
	return await verifyAtomicBoardReadback({
		...target,
		result:
			"receipt" in delivery
				? delivery.receipt
				: delivery.status === "settled" && delivery.job.phase === "applied"
					? {
							status: "applied",
							persisted_board_fingerprint:
								delivery.job.persistedBoardFingerprint,
						}
					: undefined,
	});
}

/** Verify this receipt's saved graph, including mutations invisible in canonical FlowScript. */
export async function verifyAtomicBoardReadback({
	boardState,
	appId,
	boardId,
	result,
}: {
	boardState: Pick<IBoardState, "readFlowIrCommitBoard">;
	appId: string;
	boardId: string;
	result:
		| Pick<IApplyFlowIrCommitResponse, "status" | "persisted_board_fingerprint">
		| undefined;
}): Promise<Verification> {
	const expectedFingerprint = result?.persisted_board_fingerprint;
	if (
		result?.status !== "applied" ||
		typeof expectedFingerprint !== "string" ||
		!/^flowpilot-board-v[12]:[0-9a-f]{64}$/.test(expectedFingerprint)
	) {
		return {
			verified: false,
			diagnostic:
				"PERSISTED_BOARD_RECEIPT_MISSING: Atomic apply returned no supported graph fingerprint. The saved state remains unverified; inspect the exact board before retrying.",
		};
	}
	if (!boardState.readFlowIrCommitBoard) {
		return {
			verified: false,
			diagnostic:
				"PERSISTED_BOARD_READBACK_UNAVAILABLE: This backend cannot independently reload the native saved graph.",
		};
	}
	try {
		const readback = await boardState.readFlowIrCommitBoard(appId, boardId);
		if (readback.app_id !== appId || readback.board_id !== boardId) {
			return {
				verified: false,
				diagnostic:
					"PERSISTED_BOARD_IDENTITY_MISMATCH: The saved board readback belongs to another app or board.",
			};
		}
		const fingerprint = expectedFingerprint.startsWith("flowpilot-board-v1:")
			? (readback.legacy_graph_fingerprint ?? readback.graph_fingerprint)
			: readback.graph_fingerprint;
		if (fingerprint !== expectedFingerprint) {
			return {
				verified: false,
				diagnostic:
					"PERSISTED_BOARD_MISMATCH: The saved graph does not match this atomic receipt. It may have changed since apply; inspect the current board before making another edit.",
			};
		}
		if (typeof readback.flowscript !== "string") {
			return {
				verified: false,
				diagnostic:
					"PERSISTED_FLOWSCRIPT_READBACK_FAILED: The saved graph matched, but its canonical FlowScript was not returned.",
			};
		}
		return { verified: true };
	} catch (error) {
		return {
			verified: false,
			diagnostic: `PERSISTED_BOARD_READBACK_FAILED: Atomic apply succeeded, but verification could not reload the saved board: ${error instanceof Error ? error.message : String(error)}`,
		};
	}
}

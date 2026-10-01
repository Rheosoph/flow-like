import { describe, expect, it, vi } from "vitest";
import type { BoardEditJob, BoardEditJobPhase } from "../schema/copilot";
import {
	isBlockingBoardEditJob,
	recoverBoardEditJob,
} from "./board-edit-job-recovery";

const failedJob: BoardEditJob = {
	schemaVersion: "flowpilot.board-edit-job/v1",
	jobId: "job-1",
	appId: "app-1",
	boardId: "board-1",
	requestId: "flowpilot:request-1",
	phase: "failed",
	createdAtMs: 1,
	updatedAtMs: 2,
	expiresAtMs: 10_000,
	token: {
		board_id: "board-1",
		draft_id: "draft-1",
		revision: 1,
		base_fingerprint: "fingerprint-1",
		claim_id: "claim-1",
	},
	approval: {
		kind: "execute",
		title: "Approve board edit",
		description: "Apply the retained batch.",
		sessionKey: "flowpilot_board",
		timing: "before_apply",
	},
	review: {
		commandCount: 1,
		commandCounts: { AddNode: 1 },
		commandSummaries: ["Add node"],
		replacementMode: false,
		destructiveEffects: [],
	},
};
const pendingJob: BoardEditJob = {
	...failedJob,
	phase: "applied_pending_delivery",
};
const appliedJob: BoardEditJob = { ...failedJob, phase: "applied" };
const appliedReceipt = {
	status: "applied" as const,
	delivery_complete: true,
	message: "Applied.",
	commands: [],
	board_commands: [],
	diagnostics: [],
};
const unappliedFailure: NonNullable<BoardEditJob["result"]> = {
	...appliedReceipt,
	status: "error",
	code: "IR_COMMIT_APPLY_FAILED",
	message: "The apply failed before saving changes.",
};

function recoveryBackend(job: BoardEditJob = failedJob) {
	return {
		getBoardEditJob: vi.fn(async () => job),
		resolveBoardEditJob: vi.fn(async () => ({
			job: pendingJob,
			transitioned: true,
		})),
		claimBoardEditJobDelivery: vi.fn(async () => ({
			job: pendingJob,
			claimed: true,
			deliveryLeaseId: "lease-1",
		})),
		ackBoardEditJobDelivery: vi.fn(async () => appliedJob),
	};
}

describe("isBlockingBoardEditJob", () => {
	it.each([
		["preparing", false],
		["awaiting_approval", false],
		["applying", true],
		["applied_pending_delivery", true],
		["applied", false],
		["denied", false],
		["stale", false],
		["failed", true],
		["cancelled", false],
	] satisfies [BoardEditJobPhase, boolean][])(
		"recognizes %s as blocking: %s regardless of chat origin",
		(phase, expected) => {
			for (const requestId of ["flowpilot:request-1", "request-1", undefined]) {
				expect(isBlockingBoardEditJob({ ...failedJob, phase, requestId })).toBe(
					expected,
				);
			}
		},
	);

	it.each([
		"IR_COMMIT_PERSISTENCE_UNAVAILABLE",
		"IR_COMMIT_CATALOG_UNAVAILABLE",
		"IR_COMMIT_APP_UNAVAILABLE",
		"IR_COMMIT_DESTRUCTIVE_APPROVAL_DENIED",
		"IR_COMMIT_APPLY_FAILED",
		"IR_COMMIT_FINGERPRINT_FAILED",
		"IR_COMMIT_RECEIPT_PERSISTENCE_FAILED",
	])("does not reserve the board after an unapplied %s failure", (code) => {
		for (const replayed of [false, undefined]) {
			expect(
				isBlockingBoardEditJob({
					...failedJob,
					result: { ...unappliedFailure, code, replayed },
				}),
			).toBe(false);
		}
	});

	it.each([
		{ ...unappliedFailure, replayed: true },
		{ ...unappliedFailure, status: "applied" },
		{ ...unappliedFailure, status: "stale" },
		{ ...unappliedFailure, code: "IR_COMMIT_SAVE_FAILED" },
		{ ...unappliedFailure, code: undefined },
	] satisfies NonNullable<BoardEditJob["result"]>[])(
		"keeps reserving the board when persistence remains uncertain: %j",
		(result) => {
			expect(isBlockingBoardEditJob({ ...failedJob, result })).toBe(true);
		},
	);

	it.each(["applying", "applied_pending_delivery"] as const)(
		"keeps reserving a %s job regardless of a retained failed result",
		(phase) => {
			expect(
				isBlockingBoardEditJob({
					...failedJob,
					phase,
					result: unappliedFailure,
				}),
			).toBe(true);
		},
	);
});

describe("recoverBoardEditJob", () => {
	it.each(["failed", "applying"] as const)(
		"retries a direct %s job and invalidates history during delivery",
		async (phase) => {
			const job = { ...failedJob, phase };
			const boardState = recoveryBackend(job);
			const replayReceipt = vi.fn(async () => appliedReceipt);

			const result = await recoverBoardEditJob({
				boardState,
				job,
				action: "retry",
				replayReceipt,
			});

			expect(boardState.getBoardEditJob).toHaveBeenCalledWith(job.jobId);
			expect(boardState.resolveBoardEditJob).toHaveBeenCalledWith(
				job.jobId,
				true,
				true,
			);
			expect(replayReceipt).toHaveBeenCalledWith(
				job.token,
				"flowpilot-board-edit:claim:claim-1",
				"invalidate",
			);
			expect(boardState.ackBoardEditJobDelivery).toHaveBeenCalledWith(
				job.jobId,
				"lease-1",
			);
			expect(result).toEqual({ job: appliedJob });
		},
	);

	it.each(["retry", "dismiss"] as const)(
		"allows %s after a failure that no longer blocks the board",
		async (action) => {
			const job = { ...failedJob, result: unappliedFailure };
			const settledJob: BoardEditJob = {
				...job,
				phase: action === "retry" ? "applied" : "denied",
			};
			const boardState = recoveryBackend(job);
			boardState.resolveBoardEditJob.mockResolvedValue({
				job: settledJob,
				transitioned: true,
			});
			const result = await recoverBoardEditJob({
				boardState,
				job,
				action,
				replayReceipt: async () => appliedReceipt,
			});
			expect(boardState.resolveBoardEditJob).toHaveBeenCalledWith(
				job.jobId,
				action === "retry",
				action === "retry",
			);
			expect(result).toEqual({ job: settledJob });
		},
	);

	it("delivers an authoritative pending receipt without resolving the edit again", async () => {
		const boardState = recoveryBackend(pendingJob);
		const result = await recoverBoardEditJob({
			boardState,
			job: failedJob,
			action: "retry",
			replayReceipt: async () => appliedReceipt,
		});

		expect(boardState.resolveBoardEditJob).not.toHaveBeenCalled();
		expect(result).toEqual({ job: appliedJob });
	});

	it("dismisses a failed job without replaying a receipt", async () => {
		const deniedJob: BoardEditJob = { ...failedJob, phase: "denied" };
		const boardState = recoveryBackend();
		boardState.resolveBoardEditJob.mockResolvedValue({
			job: deniedJob,
			transitioned: true,
		});
		const replayReceipt = vi.fn(async () => appliedReceipt);

		const result = await recoverBoardEditJob({
			boardState,
			job: failedJob,
			action: "dismiss",
			replayReceipt,
		});

		expect(boardState.resolveBoardEditJob).toHaveBeenCalledWith(
			failedJob.jobId,
			false,
			false,
		);
		expect(replayReceipt).not.toHaveBeenCalled();
		expect(result).toEqual({ job: deniedJob });
	});

	it("delivers the durable receipt if dismiss discovers the edit already applied", async () => {
		const boardState = recoveryBackend();
		const replayReceipt = vi.fn(async () => appliedReceipt);
		const result = await recoverBoardEditJob({
			boardState,
			job: failedJob,
			action: "dismiss",
			replayReceipt,
		});

		expect(boardState.resolveBoardEditJob).toHaveBeenCalledWith(
			failedJob.jobId,
			false,
			false,
		);
		expect(replayReceipt).toHaveBeenCalledWith(
			failedJob.token,
			"flowpilot-board-edit:claim:claim-1",
			"invalidate",
		);
		expect(result).toEqual({ job: appliedJob });
	});

	it("does not dismiss a job that is still applying", async () => {
		const applyingJob: BoardEditJob = { ...failedJob, phase: "applying" };
		const boardState = recoveryBackend(applyingJob);
		const replayReceipt = vi.fn(async () => appliedReceipt);
		const result = await recoverBoardEditJob({
			boardState,
			job: failedJob,
			action: "dismiss",
			replayReceipt,
		});

		expect(boardState.resolveBoardEditJob).not.toHaveBeenCalled();
		expect(replayReceipt).not.toHaveBeenCalled();
		expect(result.job).toBe(applyingJob);
		expect(result.message).toContain("may still be applying");
	});

	it.each(["applied", "denied", "stale", "cancelled"] as const)(
		"returns an authoritative %s job without mutating it",
		async (phase) => {
			const settledJob = { ...failedJob, phase };
			const boardState = recoveryBackend(settledJob);
			const replayReceipt = vi.fn(async () => appliedReceipt);
			const result = await recoverBoardEditJob({
				boardState,
				job: failedJob,
				action: "retry",
				replayReceipt,
			});

			expect(boardState.resolveBoardEditJob).not.toHaveBeenCalled();
			expect(boardState.claimBoardEditJobDelivery).not.toHaveBeenCalled();
			expect(replayReceipt).not.toHaveBeenCalled();
			expect(result).toEqual({ job: settledJob });
		},
	);

	it("rejects a missing authoritative job without using the stale local copy", async () => {
		const boardState = {
			...recoveryBackend(),
			getBoardEditJob: vi.fn(async () => undefined),
		};
		await expect(
			recoverBoardEditJob({
				boardState,
				job: failedJob,
				action: "retry",
				replayReceipt: async () => appliedReceipt,
			}),
		).rejects.toThrow("no longer available");
		expect(boardState.resolveBoardEditJob).not.toHaveBeenCalled();
	});

	it("supports backends without authoritative lookup", async () => {
		const boardState = { ...recoveryBackend(), getBoardEditJob: undefined };
		const result = await recoverBoardEditJob({
			boardState,
			job: failedJob,
			action: "retry",
			replayReceipt: async () => appliedReceipt,
		});
		expect(result).toEqual({ job: appliedJob });
	});

	it("preserves a failed recovery's error for the board UI", async () => {
		const job = { ...failedJob, error: "The retained batch is unavailable." };
		const boardState = recoveryBackend();
		boardState.resolveBoardEditJob.mockResolvedValue({
			job,
			transitioned: false,
		});
		const result = await recoverBoardEditJob({
			boardState,
			job: failedJob,
			action: "retry",
			replayReceipt: async () => appliedReceipt,
		});
		expect(result).toEqual({ job, message: job.error });
		expect(boardState.claimBoardEditJobDelivery).not.toHaveBeenCalled();
	});

	it("reports an unapplied retry failure without claiming the board remains blocked", async () => {
		const job = { ...failedJob, result: unappliedFailure };
		const boardState = recoveryBackend();
		boardState.resolveBoardEditJob.mockResolvedValue({
			job,
			transitioned: true,
		});
		const result = await recoverBoardEditJob({
			boardState,
			job: failedJob,
			action: "retry",
			replayReceipt: async () => appliedReceipt,
		});
		expect(result).toEqual({ job, message: unappliedFailure.message });
		expect(isBlockingBoardEditJob(result.job)).toBe(false);
	});

	it("preserves delivery failures without acknowledging the receipt", async () => {
		const boardState = recoveryBackend(pendingJob);
		const result = await recoverBoardEditJob({
			boardState,
			job: pendingJob,
			action: "retry",
			replayReceipt: async () => ({
				...appliedReceipt,
				delivery_complete: false,
				message: "Board synchronization failed.",
			}),
		});
		expect(result).toEqual({
			job: pendingJob,
			message: "Board synchronization failed.",
		});
		expect(boardState.ackBoardEditJobDelivery).not.toHaveBeenCalled();
	});

	it("returns quietly when another renderer finishes receipt delivery", async () => {
		const boardState = recoveryBackend(pendingJob);
		boardState.claimBoardEditJobDelivery.mockResolvedValue({
			job: appliedJob,
			claimed: false,
			deliveryLeaseId: "",
		});
		const replayReceipt = vi.fn(async () => appliedReceipt);
		const result = await recoverBoardEditJob({
			boardState,
			job: pendingJob,
			action: "retry",
			replayReceipt,
		});
		expect(result).toEqual({ job: appliedJob });
		expect(replayReceipt).not.toHaveBeenCalled();
	});
});

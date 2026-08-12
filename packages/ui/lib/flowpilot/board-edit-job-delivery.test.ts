import { describe, expect, it, vi } from "vitest";

import type { BoardEditJob } from "../schema/copilot";
import {
	boardEditJobResolutionHistoryMode,
	deliverBoardEditJobReceipt,
	flowIrCommitDeliveryId,
	isDirectFlowPilotBoardEditJob,
} from "./board-edit-job-delivery";

const pendingJob: BoardEditJob = {
	schemaVersion: "flowpilot.board-edit-job/v1",
	jobId: "job-1",
	appId: "app-1",
	boardId: "board-1",
	requestId: "request-1",
	phase: "applied_pending_delivery",
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

const appliedReceipt = {
	status: "applied" as const,
	delivery_complete: true,
	message: "Applied.",
	commands: [],
	board_commands: [],
	diagnostics: [],
};

describe("deliverBoardEditJobReceipt", () => {
	it("appends history only for a newly performed native apply", () => {
		expect(
			boardEditJobResolutionHistoryMode({
				job: { ...pendingJob, result: { ...appliedReceipt, replayed: false } },
				transitioned: true,
			}),
		).toBe("append");
		expect(
			boardEditJobResolutionHistoryMode({
				job: { ...pendingJob, result: { ...appliedReceipt, replayed: true } },
				transitioned: true,
			}),
		).toBe("invalidate");
		expect(
			boardEditJobResolutionHistoryMode({
				job: pendingJob,
				transitioned: false,
			}),
		).toBe("invalidate");
	});

	it("recognizes direct-panel review ownership", () => {
		expect(
			isDirectFlowPilotBoardEditJob({
				...pendingJob,
				requestId: "flowpilot:request-1",
			}),
		).toBe(true);
		expect(isDirectFlowPilotBoardEditJob(pendingJob)).toBe(false);
	});

	it("uses one bounded identity for direct native replay and history", () => {
		expect(flowIrCommitDeliveryId(pendingJob.token)).toBe(
			"flowpilot-board-edit:claim:claim-1",
		);
	});

	it("claims, replays, and acknowledges in order", async () => {
		const calls: string[] = [];
		const appliedJob = { ...pendingJob, phase: "applied" as const };
		const boardState = {
			claimBoardEditJobDelivery: vi.fn(async () => {
				calls.push("claim");
				return {
					job: pendingJob,
					claimed: true,
					deliveryLeaseId: "lease-1",
				};
			}),
			ackBoardEditJobDelivery: vi.fn(
				async (jobId: string, deliveryLeaseId: string) => {
					calls.push("ack");
					expect(jobId).toBe(pendingJob.jobId);
					expect(deliveryLeaseId).toBe("lease-1");
					return appliedJob;
				},
			),
		};
		const replayReceipt = vi.fn(
			async (
				_token: BoardEditJob["token"],
				deliveryId: string,
				historyMode: "append" | "invalidate",
			) => {
				calls.push("replay");
				expect(deliveryId).toBe("flowpilot-board-edit:claim:claim-1");
				expect(historyMode).toBe("invalidate");
				return appliedReceipt;
			},
		);

		const outcome = await deliverBoardEditJobReceipt({
			boardState,
			job: pendingJob,
			replayReceipt,
		});

		expect(calls).toEqual(["claim", "replay", "ack"]);
		expect(outcome.status).toBe("delivered");
		expect(outcome.job.phase).toBe("applied");
	});

	it("does not replay while another renderer owns the lease", async () => {
		const replayReceipt = vi.fn(async () => appliedReceipt);
		const acknowledge = vi.fn();

		const outcome = await deliverBoardEditJobReceipt({
			boardState: {
				claimBoardEditJobDelivery: async () => ({
					job: pendingJob,
					claimed: false,
				}),
				ackBoardEditJobDelivery: acknowledge,
			},
			job: pendingJob,
			replayReceipt,
		});

		expect(outcome.status).toBe("busy");
		expect(replayReceipt).not.toHaveBeenCalled();
		expect(acknowledge).not.toHaveBeenCalled();
	});

	it("leaves the job unacknowledged when receipt replay fails", async () => {
		const acknowledge = vi.fn();
		const failedReceipt = {
			...appliedReceipt,
			status: "error" as const,
			message: "Renderer sync failed.",
		};

		const outcome = await deliverBoardEditJobReceipt({
			boardState: {
				claimBoardEditJobDelivery: async () => ({
					job: pendingJob,
					claimed: true,
					deliveryLeaseId: "lease-1",
				}),
				ackBoardEditJobDelivery: acknowledge,
			},
			job: pendingJob,
			replayReceipt: async () => failedReceipt,
		});

		expect(outcome.status).toBe("replay_failed");
		expect(acknowledge).not.toHaveBeenCalled();
	});

	it("does not acknowledge a locally applied but incompletely delivered receipt", async () => {
		const acknowledge = vi.fn();
		const outcome = await deliverBoardEditJobReceipt({
			boardState: {
				claimBoardEditJobDelivery: async () => ({
					job: pendingJob,
					claimed: true,
					deliveryLeaseId: "lease-1",
				}),
				ackBoardEditJobDelivery: acknowledge,
			},
			job: pendingJob,
			replayReceipt: async () => ({
				...appliedReceipt,
				delivery_complete: false,
			}),
		});

		expect(outcome.status).toBe("replay_failed");
		expect(acknowledge).not.toHaveBeenCalled();
	});

	it("settles an already delivered job without claiming again", async () => {
		const claim = vi.fn();
		const replayReceipt = vi.fn(async () => appliedReceipt);
		const outcome = await deliverBoardEditJobReceipt({
			boardState: { claimBoardEditJobDelivery: claim },
			job: { ...pendingJob, phase: "applied" },
			replayReceipt,
		});

		expect(outcome.status).toBe("settled");
		expect(claim).not.toHaveBeenCalled();
		expect(replayReceipt).not.toHaveBeenCalled();
	});
});

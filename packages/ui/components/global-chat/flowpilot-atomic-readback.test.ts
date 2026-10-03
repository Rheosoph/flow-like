import { describe, expect, mock, test } from "bun:test";
import { deliverBoardEditJobReceipt } from "../../lib/flowpilot/board-edit-job-delivery";
import type { BoardEditJob } from "../../lib/schema/copilot";
import type { IFlowIrCommitReadback } from "../../state/backend-state/board-state";
import {
	verifyAtomicBoardDeliveryReadback,
	verifyAtomicBoardReadback,
} from "./flowpilot-atomic-readback";

const applied = {
	status: "applied" as const,
	persisted_board_fingerprint: `flowpilot-board-v1:${"a".repeat(64)}`,
};
const snapshot = {
	app_id: "app",
	board_id: "board",
	graph_fingerprint: applied.persisted_board_fingerprint,
	flowscript: "on eventsGeneric submitTicket(payload: Struct) {}",
};

function fixture(overrides: Partial<IFlowIrCommitReadback> = {}) {
	const readFlowIrCommitBoard = mock(async () => ({
		...snapshot,
		...overrides,
	}));
	return {
		boardState: { readFlowIrCommitBoard },
		appId: "app",
		boardId: "board",
		result: applied,
	};
}

function pendingJob(): BoardEditJob {
	return {
		schemaVersion: "flowpilot.board-edit-job/v1",
		jobId: "job",
		appId: "app",
		boardId: "board",
		phase: "applied_pending_delivery",
		createdAtMs: 1,
		updatedAtMs: 2,
		expiresAtMs: 10_000,
		token: {
			board_id: "board",
			draft_id: "draft",
			revision: 1,
			base_fingerprint: "base",
			claim_id: "claim",
		},
		approval: {
			kind: "mutating",
			title: "Apply workflow",
			description: "Apply the retained batch.",
			sessionKey: "flowpilot_board",
		},
		review: {
			commandCount: 1,
			commandCounts: { AddNode: 1 },
			commandSummaries: ["Add a node"],
			replacementMode: false,
			destructiveEffects: [],
		},
	};
}

describe("atomic workflow persistence verification", () => {
	test("verifies new receipts with the current fingerprint", async () => {
		const fingerprint = `flowpilot-board-v2:${"b".repeat(64)}`;
		const f = fixture({
			graph_fingerprint: fingerprint,
			legacy_graph_fingerprint: applied.persisted_board_fingerprint,
		});
		f.result = { ...applied, persisted_board_fingerprint: fingerprint };
		expect(await verifyAtomicBoardReadback(f)).toEqual({ verified: true });
	});

	test("verifies older receipts with the unchanged legacy algorithm", async () => {
		expect(
			await verifyAtomicBoardReadback(
				fixture({
					graph_fingerprint: `flowpilot-board-v2:${"b".repeat(64)}`,
					legacy_graph_fingerprint: applied.persisted_board_fingerprint,
				}),
			),
		).toEqual({ verified: true });
	});

	test("does not compare an old receipt against a different algorithm", async () => {
		const f = fixture({
			graph_fingerprint: `flowpilot-board-v2:${"a".repeat(64)}`,
			legacy_graph_fingerprint: `flowpilot-board-v1:${"b".repeat(64)}`,
		});
		expect(await verifyAtomicBoardReadback(f)).toMatchObject({
			verified: false,
			diagnostic: expect.stringContaining("PERSISTED_BOARD_MISMATCH:"),
		});
	});

	test("does not use legacy proof for a current receipt", async () => {
		const f = fixture({
			graph_fingerprint: `flowpilot-board-v2:${"b".repeat(64)}`,
			legacy_graph_fingerprint: applied.persisted_board_fingerprint,
		});
		f.result = {
			...applied,
			persisted_board_fingerprint: `flowpilot-board-v2:${"a".repeat(64)}`,
		};
		expect(await verifyAtomicBoardReadback(f)).toMatchObject({
			verified: false,
			diagnostic: expect.stringContaining("PERSISTED_BOARD_MISMATCH:"),
		});
	});

	test("verifies the leased receipt when the real job wrapper omits successful results", async () => {
		const job = pendingJob();
		const settledJob: BoardEditJob = { ...job, phase: "applied" };
		const receipt = {
			...applied,
			replayed: true,
			delivery_complete: true,
			message: "Applied exact batch.",
			commands: [],
			board_commands: [],
			diagnostics: [],
		};
		const delivery = await deliverBoardEditJobReceipt({
			job,
			boardState: {
				claimBoardEditJobDelivery: async () => ({
					job,
					claimed: true,
					deliveryLeaseId: "lease",
				}),
				ackBoardEditJobDelivery: async () => settledJob,
			},
			replayReceipt: async () => receipt,
		});
		expect(job.result).toBeUndefined();
		expect(delivery.job.result).toBeUndefined();
		expect(delivery.status).toBe("delivered");
		expect(
			await verifyAtomicBoardDeliveryReadback({ ...fixture(), delivery }),
		).toEqual({ verified: true });
		expect(
			await verifyAtomicBoardDeliveryReadback({
				...fixture({
					graph_fingerprint: `flowpilot-board-v1:${"b".repeat(64)}`,
				}),
				delivery,
			}),
		).toMatchObject({
			verified: false,
			diagnostic: expect.stringContaining("PERSISTED_BOARD_MISMATCH:"),
		});
	});

	test("an already settled job without receipt evidence remains unverified", async () => {
		const replayReceipt = mock(async () => {
			throw new Error("A settled job cannot be replayed");
		});
		const delivery = await deliverBoardEditJobReceipt({
			job: { ...pendingJob(), phase: "applied" },
			boardState: {},
			replayReceipt,
		});
		expect(delivery.status).toBe("settled");
		expect(replayReceipt).not.toHaveBeenCalled();
		expect(
			await verifyAtomicBoardDeliveryReadback({ ...fixture(), delivery }),
		).toMatchObject({
			verified: false,
			diagnostic: expect.stringContaining("PERSISTED_BOARD_RECEIPT_MISSING:"),
		});
	});

	for (const raced of [false, true]) {
		test(`verifies compact proof from a ${raced ? "racing" : "previously"} settled job`, async () => {
			const job = pendingJob();
			const settled: BoardEditJob = {
				...job,
				phase: "applied",
				persistedBoardFingerprint: applied.persisted_board_fingerprint,
			};
			const replayReceipt = mock(async () => {
				throw new Error("A settled job cannot be replayed");
			});
			const delivery = await deliverBoardEditJobReceipt({
				job: raced ? job : settled,
				boardState: {
					claimBoardEditJobDelivery: async () => ({
						job: settled,
						claimed: false,
					}),
					ackBoardEditJobDelivery: async () => {
						throw new Error("The winner already acknowledged delivery");
					},
				},
				replayReceipt,
			});
			expect(delivery.status).toBe("settled");
			expect(delivery.job.result).toBeUndefined();
			expect(replayReceipt).not.toHaveBeenCalled();
			expect(
				await verifyAtomicBoardDeliveryReadback({ ...fixture(), delivery }),
			).toEqual({ verified: true });
			expect(
				await verifyAtomicBoardDeliveryReadback({
					...fixture({
						graph_fingerprint: `flowpilot-board-v1:${"b".repeat(64)}`,
					}),
					delivery,
				}),
			).toMatchObject({
				verified: false,
				diagnostic: expect.stringContaining("PERSISTED_BOARD_MISMATCH:"),
			});
		});
	}

	test("does not accept compact proof from another delivery target", async () => {
		const delivery = await deliverBoardEditJobReceipt({
			job: {
				...pendingJob(),
				appId: "other-app",
				phase: "applied",
				persistedBoardFingerprint: applied.persisted_board_fingerprint,
			},
			boardState: {},
			replayReceipt: async () => {
				throw new Error("A settled job cannot be replayed");
			},
		});
		const f = fixture();
		expect(
			await verifyAtomicBoardDeliveryReadback({ ...f, delivery }),
		).toMatchObject({
			verified: false,
			diagnostic: expect.stringContaining("PERSISTED_BOARD_IDENTITY_MISMATCH:"),
		});
		expect(f.boardState.readFlowIrCommitBoard).not.toHaveBeenCalled();
	});

	test("accepts exact graph readback even when the canonical text stays unchanged", async () => {
		const first = fixture();
		expect(await verifyAtomicBoardReadback(first)).toEqual({ verified: true });
		const nextFingerprint = `flowpilot-board-v1:${"b".repeat(64)}`;
		const second = fixture({ graph_fingerprint: nextFingerprint });
		second.result = {
			...applied,
			persisted_board_fingerprint: nextFingerprint,
		};
		expect(await verifyAtomicBoardReadback(second)).toEqual({ verified: true });
		expect(second.boardState.readFlowIrCommitBoard).toHaveBeenCalledWith(
			"app",
			"board",
		);
	});

	test("rejects an old replay against a newer graph even when source is identical", async () => {
		const f = fixture({
			graph_fingerprint: `flowpilot-board-v1:${"b".repeat(64)}`,
		});
		expect(await verifyAtomicBoardReadback(f)).toMatchObject({
			verified: false,
			diagnostic: expect.stringContaining("PERSISTED_BOARD_MISMATCH:"),
		});
	});

	for (const key of ["app_id", "board_id"] as const) {
		test(`rejects a readback with another ${key}`, async () => {
			expect(
				await verifyAtomicBoardReadback(fixture({ [key]: "other" })),
			).toMatchObject({
				verified: false,
				diagnostic: expect.stringContaining(
					"PERSISTED_BOARD_IDENTITY_MISMATCH:",
				),
			});
		});
	}

	test("fails closed for legacy or unsuccessful receipts without reading", async () => {
		for (const result of [
			undefined,
			{ status: "applied" as const },
			{ ...applied, status: "error" as const },
			{
				...applied,
				persisted_board_fingerprint: `flowpilot-board-v3:${"a".repeat(64)}`,
			},
		]) {
			const f = fixture();
			expect(await verifyAtomicBoardReadback({ ...f, result })).toMatchObject({
				verified: false,
				diagnostic: expect.stringContaining("PERSISTED_BOARD_RECEIPT_MISSING:"),
			});
			expect(f.boardState.readFlowIrCommitBoard).not.toHaveBeenCalled();
		}
	});

	test("distinguishes missing read authority from a failed storage read", async () => {
		expect(
			await verifyAtomicBoardReadback({ ...fixture(), boardState: {} }),
		).toMatchObject({
			verified: false,
			diagnostic: expect.stringContaining(
				"PERSISTED_BOARD_READBACK_UNAVAILABLE:",
			),
		});
		const f = fixture();
		f.boardState.readFlowIrCommitBoard.mockRejectedValue(
			new Error("missing saved object"),
		);
		expect(await verifyAtomicBoardReadback(f)).toMatchObject({
			verified: false,
			diagnostic: expect.stringContaining("PERSISTED_BOARD_READBACK_FAILED:"),
		});
	});

	test("does not claim canonical readback when it is absent", async () => {
		expect(
			await verifyAtomicBoardReadback(
				fixture({ flowscript: undefined as unknown as string }),
			),
		).toMatchObject({
			verified: false,
			diagnostic: expect.stringContaining(
				"PERSISTED_FLOWSCRIPT_READBACK_FAILED:",
			),
		});
	});
});

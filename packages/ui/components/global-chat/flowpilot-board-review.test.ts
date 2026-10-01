import { describe, expect, test } from "bun:test";
import type { BoardEditJob } from "../../lib/schema/copilot";
import { executeFlowPilotBoardReview } from "./flowpilot-board-review";

function review(
	phase: BoardEditJob["phase"] = "awaiting_approval",
): BoardEditJob {
	return {
		schemaVersion: "flowpilot.board-edit-job/v1",
		jobId: "review",
		appId: "app",
		boardId: "board",
		phase,
		createdAtMs: 1,
		updatedAtMs: 2,
		expiresAtMs: 100,
		token: {
			board_id: "board",
			draft_id: "repair",
			revision: 3,
			base_fingerprint: "base",
			claim_id: "claim",
		},
		approval: {
			kind: "execute",
			title: "Apply",
			description: "Repair",
			sessionKey: "review",
		},
		review: {
			commandCount: 68,
			commandCounts: { AddNode: 20, ConnectPins: 28, UpdateNodePin: 20 },
			commandSummaries: [],
			replacementMode: false,
			destructiveEffects: [],
		},
		flowscriptSource: "eventsGeneric() {}",
	};
}

function harness(job = review()) {
	const calls: string[] = [];
	const options: Parameters<typeof executeFlowPilotBoardReview>[0] = {
		appId: "app",
		boardId: "board",
		jobId: "review",
		action: "apply",
		boardState: {
			listBoardEditJobs: async (appId, boardId, terminal) => {
				calls.push(`list:${appId}:${boardId}:${terminal}`);
				return [job, { ...job, jobId: "foreign", appId: "other" }];
			},
			getBoardEditJob: async () => {
				calls.push("get");
				return job;
			},
			resolveBoardEditJob: async (id, approved, destructive) => {
				calls.push(`resolve:${id}:${approved}:${destructive}`);
				return {
					job: {
						...job,
						phase: approved ? "applied_pending_delivery" : "denied",
					},
					transitioned: true,
				};
			},
		},
		getVisibleAppIds: async () => new Set(["app"]),
		assertActive: () => {},
		approve: async (_, action) => {
			calls.push(`approve:${action}`);
			return true;
		},
		onJob: (value) => {
			calls.push(`source:${value.phase}`);
		},
		onApplied: async ({ transitioned }) => {
			calls.push(`deliver:${transitioned}`);
			return { persisted_readback_verified: true };
		},
	};
	return { options, calls };
}

describe("retained board review tool", () => {
	test("lists applied reviews and excludes foreign jobs and source payloads", async () => {
		const { options, calls } = harness(review("applied"));
		const result = await executeFlowPilotBoardReview({
			...options,
			action: "list",
			jobId: undefined,
		});
		expect(calls).toEqual(["list:app:board:true"]);
		expect(result).toMatchObject({
			status: "ok",
			reviews: [{ job_id: "review", phase: "applied", applied: true }],
		});
		expect(JSON.stringify(result)).not.toContain("eventsGeneric");
	});

	test("status reads native state and restores source without approval or mutation", async () => {
		const { options, calls } = harness(review("applied"));
		const result = await executeFlowPilotBoardReview({
			...options,
			action: "status",
		});
		expect(calls).toEqual(["get", "source:applied"]);
		expect(result).toMatchObject({
			status: "ok",
			phase: "applied",
			runtime_verified: false,
		});
	});

	test("applies the exact approved review and delivers its receipt in the same call", async () => {
		const { options, calls } = harness();
		const result = await executeFlowPilotBoardReview(options);
		expect(calls).toEqual([
			"get",
			"source:awaiting_approval",
			"approve:apply",
			"resolve:review:true:true",
			"source:applied_pending_delivery",
			"deliver:true",
		]);
		expect(result).toMatchObject({
			status: "applied",
			command_count: 68,
			persisted_readback_verified: true,
			runtime_verified: false,
		});
	});

	test("does not request approval or repeat mutation for an already applied job", async () => {
		const { options, calls } = harness(review("applied"));
		const result = await executeFlowPilotBoardReview(options);
		expect(calls).toEqual(["get", "source:applied", "deliver:false"]);
		expect(result.status).toBe("applied");
	});

	test("dismiss uses the native denial path only after approval", async () => {
		const { options, calls } = harness();
		const result = await executeFlowPilotBoardReview({
			...options,
			action: "dismiss",
		});
		expect(calls).toContain("resolve:review:false:false");
		expect(calls).not.toContain("deliver:true");
		expect(result.phase).toBe("denied");
	});

	test("refuses an app or board mismatch before showing source or requesting approval", async () => {
		const { options, calls } = harness({
			...review(),
			boardId: "another-board",
		});
		const result = await executeFlowPilotBoardReview(options);
		expect(result.code).toBe("BOARD_REVIEW_NOT_FOUND");
		expect(calls).toEqual(["get"]);
	});

	test("requires exact job identity and visible app before reading reviews", async () => {
		const { options, calls } = harness();
		expect(
			(await executeFlowPilotBoardReview({ ...options, jobId: undefined }))
				.code,
		).toBe("BOARD_REVIEW_JOB_REQUIRED");
		expect(
			(
				await executeFlowPilotBoardReview({
					...options,
					getVisibleAppIds: async () => new Set(),
				})
			).code,
		).toBe("BOARD_REVIEW_APP_OUT_OF_SCOPE");
		expect(calls).toEqual([]);
	});

	test("a denied action leaves the retained review unchanged", async () => {
		const { options, calls } = harness();
		const result = await executeFlowPilotBoardReview({
			...options,
			approve: async () => false,
		});
		expect(result).toMatchObject({
			status: "denied",
			phase: "awaiting_approval",
			applied: false,
		});
		expect(calls).toEqual(["get", "source:awaiting_approval"]);
	});

	test("cancellation while waiting for approval prevents a late apply", async () => {
		const { options, calls } = harness();
		let cancelled = false;
		await expect(
			executeFlowPilotBoardReview({
				...options,
				approve: async () => {
					cancelled = true;
					return true;
				},
				assertActive: () => {
					if (cancelled) throw new Error("expired");
				},
			}),
		).rejects.toThrow("expired");
		expect(calls).toEqual(["get", "source:awaiting_approval"]);
	});

	test("stale and denied jobs return their actual state without another apply", async () => {
		for (const phase of ["stale", "denied"] as const) {
			const { options, calls } = harness(review(phase));
			expect((await executeFlowPilotBoardReview(options)).status).toBe(phase);
			expect(calls).toEqual(["get", `source:${phase}`]);
		}
	});

	test("recovers an interrupted apply through the native idempotent resolver", async () => {
		const { options, calls } = harness(review("applying"));
		expect((await executeFlowPilotBoardReview(options)).status).toBe("applied");
		expect(calls).toContain("resolve:review:true:true");
	});

	test("dismisses a stale review to release its retained claim", async () => {
		const { options, calls } = harness(review("stale"));
		expect(
			(await executeFlowPilotBoardReview({ ...options, action: "dismiss" }))
				.phase,
		).toBe("denied");
		expect(calls).toContain("resolve:review:false:false");
	});

	test("delivery failure preserves the fact that native application completed", async () => {
		const { options } = harness();
		const result = await executeFlowPilotBoardReview({
			...options,
			onApplied: async () => {
				throw new Error("sync failed");
			},
		});
		expect(result).toMatchObject({
			status: "applied",
			applied: true,
			persisted_readback_verified: false,
			runtime_verified: false,
		});
		expect(String(result.diagnostics)).toContain("sync failed");
	});
});

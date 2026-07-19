import {
	BoardEditCoordinator,
	type BoardEditRecoveryStorage,
	BoardEditRecoveryStore,
	CreatedArtifactJournal,
	type CreatedArtifactRequestIdentity,
	FrontendRequestExecutionFence,
	assessFlowScriptReadback,
	boardEditInterruptionResult,
	boardEditLockKey,
	boardEditRecoveryKey,
	creationRequestFingerprint,
	flowScriptSnapshotChanged,
	hasActiveFrontendRequestOwnership,
	isCreatedAppBuildTargetMismatch,
	resolveFrontendToolExecutionDeadline,
	retainedFlowScriptRecoveryInstruction,
	retryCreatedAppReadiness,
	safeFlowScriptPlanReasoning,
} from "@flow-like/flow-like-ui/components/flowpilot/board-edit-guard";
import { ApiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import { sanitizeFlowScriptForPersistence } from "@flow-like/flow-like-ui/lib/flowscript-persistence";
import { describe, expect, test, vi } from "vitest";

class MemoryRecoveryStorage implements BoardEditRecoveryStorage {
	readonly values = new Map<string, string>();

	getItem(key: string) {
		return this.values.get(key) ?? null;
	}

	setItem(key: string, value: string) {
		this.values.set(key, value);
	}

	removeItem(key: string) {
		this.values.delete(key);
	}
}

const completeSupportFlow = `@secret
const IMAP_HOST: string = ""
@secret
const SMTP_HOST: string = ""

function pollInbox() {
  const connection = emailImapConnect({ host: IMAP_HOST })
  const inbox = mailImapInbox({ connection: connection })
  const unread = mailImapList({ inbox: inbox })
  logInfo({ message: unread })
}

function requestApproval() {
  const smtp = emailSmtpConnect({ host: SMTP_HOST })
  emailSend({ connection: smtp })
}

eventsSimple() {
  pollInbox()
  requestApproval()
}`;

describe("board edit guard", () => {
	test("retained FlowScript recovery overrides delegated tiny diagnostic fallbacks", () => {
		const instruction =
			retainedFlowScriptRecoveryInstruction(completeSupportFlow);

		expect(instruction).toContain("HOST RECOVERY POLICY");
		expect(instruction).toContain("active production workspace");
		expect(instruction).toContain('"minimal diagnostic"');
		expect(instruction).toContain("not an end-user scope change");
		expect(instruction).toContain("Own the edit_flowscript validation loop");
		expect(instruction).toContain(completeSupportFlow);
		expect(retainedFlowScriptRecoveryInstruction("   ")).toBe("");
	});

	test("uses the backend per-tool deadline without an extra board or widget cap", () => {
		const dispatchedAtMs = 1_783_972_859_608;
		const backendDeadlineAtMs = dispatchedAtMs + 600_000;

		expect(
			resolveFrontendToolExecutionDeadline({
				toolName: "flowpilot_board",
				backendDeadlineAtMs,
			}),
		).toBe(backendDeadlineAtMs - 5_000);
		expect(
			resolveFrontendToolExecutionDeadline({
				toolName: "flowpilot_widget",
				backendDeadlineAtMs,
			}),
		).toBe(backendDeadlineAtMs - 5_000);
	});

	test("does not replace a missing backend deadline with an arbitrary sentinel", () => {
		expect(
			resolveFrontendToolExecutionDeadline({
				toolName: "flowpilot_board",
			}),
		).toBeUndefined();
	});

	test("keeps the backend safety margin for ordinary frontend tools", () => {
		expect(
			resolveFrontendToolExecutionDeadline({
				toolName: "database_tool",
				backendDeadlineAtMs: 90_000,
			}),
		).toBe(85_000);
		expect(
			resolveFrontendToolExecutionDeadline({
				toolName: "database_tool",
			}),
		).toBeUndefined();
	});

	test("retains request ownership while the owner or a nested child is active", () => {
		expect(
			hasActiveFrontendRequestOwnership("board-request", [
				{ requestId: "board-request" },
			]),
		).toBe(true);
		expect(
			hasActiveFrontendRequestOwnership("board-request", [
				{
					requestId: "database-request",
					parentRequestId: "board-request",
				},
			]),
		).toBe(true);
		expect(
			hasActiveFrontendRequestOwnership("board-request", [
				{ requestId: "unrelated-request" },
			]),
		).toBe(false);
	});

	test("retains an invalidated execution fence past the former eviction bound until settlement", () => {
		const fence = new FrontendRequestExecutionFence<{ label: string }>();
		const oldest = fence.begin({
			request: { label: "oldest" },
			requestId: "request-0",
		});
		fence.invalidate(oldest.requestId);

		const later = Array.from({ length: 300 }, (_, index) =>
			fence.begin({
				request: { label: `later-${index}` },
				requestId: `request-${index + 1}`,
			}),
		);

		expect(fence.isInvalidated(oldest)).toBe(true);
		expect(oldest.controller.signal.aborted).toBe(true);
		expect(fence.size).toBe(301);
		for (const execution of later) fence.settle(execution);
		expect(fence.size).toBe(1);
		expect(fence.isInvalidated(oldest)).toBe(true);

		fence.settle(oldest);
		expect(fence.size).toBe(0);
	});

	test("keeps a cancelled request generation fenced until its execution tree is quiescent", () => {
		const fence = new FrontendRequestExecutionFence<{ label: string }>();
		const root = fence.begin({
			request: { label: "root" },
			requestId: "root",
		});
		const child = fence.begin({
			request: { label: "child" },
			requestId: "child",
			parentRequestId: root.requestId,
		});

		fence.invalidate(root.requestId);
		fence.settle(root);
		expect(fence.size).toBe(2);
		const lateChild = fence.begin({
			request: { label: "late-child" },
			requestId: "late-child",
			parentRequestId: root.requestId,
		});
		expect(fence.isInvalidated(lateChild)).toBe(true);

		fence.settle(child);
		fence.settle(lateChild);
		expect(fence.size).toBe(0);
	});

	test("a duplicate active request id cannot authorize a second side effect", () => {
		const fence = new FrontendRequestExecutionFence<{ label: string }>();
		const first = fence.begin({
			request: { label: "first" },
			requestId: "duplicate",
		});
		const second = fence.begin({
			request: { label: "second" },
			requestId: "duplicate",
		});

		expect(first.generation).toBe(0);
		expect(second.generation).toBe(1);
		expect(fence.isInvalidated(first)).toBe(true);
		expect(fence.isInvalidated(second)).toBe(true);
		expect(second.controller.signal.aborted).toBe(true);
		fence.settle(first);
		expect(fence.getLatest("duplicate")).toBe(second);

		fence.settle(second);
		expect(fence.size).toBe(0);
	});

	test("a cancellation delivered before begin is consumed by that execution lifecycle", () => {
		const fence = new FrontendRequestExecutionFence<{ label: string }>();
		fence.invalidate("reordered-request");
		expect(fence.pendingInvalidationCount).toBe(1);

		const reordered = fence.begin({
			request: { label: "reordered" },
			requestId: "reordered-request",
		});
		expect(fence.pendingInvalidationCount).toBe(0);
		expect(fence.isInvalidated(reordered)).toBe(true);

		fence.settle(reordered);
		expect(fence.size).toBe(0);
	});

	test("pre-registration cancellation overflow fails closed instead of evicting", () => {
		const fence = new FrontendRequestExecutionFence<{ label: string }>();
		for (let index = 0; index <= 1_024; index += 1) {
			fence.invalidate(`missing-${index}`);
		}
		expect(fence.isFailClosed).toBe(true);
		expect(fence.pendingInvalidationCount).toBe(0);

		const later = fence.begin({
			request: { label: "later" },
			requestId: "otherwise-new",
		});
		expect(fence.isInvalidated(later)).toBe(true);
		fence.settle(later);
	});

	test("serializes edits for the same app and board", async () => {
		const coordinator = new BoardEditCoordinator();
		const key = boardEditLockKey("app-1", "board-1");
		const releaseFirst = await coordinator.acquire(key);
		let secondAcquired = false;
		const second = coordinator.acquire(key).then((release) => {
			secondAcquired = true;
			return release;
		});

		await Promise.resolve();
		expect(secondAcquired).toBe(false);
		releaseFirst();
		const releaseSecond = await second;
		expect(secondAcquired).toBe(true);
		releaseSecond();
	});

	test("computes one stable lock key per target", () => {
		expect(boardEditLockKey("app-1", "board-1")).toBe(
			boardEditLockKey("app-1", "board-1"),
		);
		expect(boardEditLockKey("app-1", "board-1")).not.toBe(
			boardEditLockKey("app-1", "board-2"),
		);
		expect(boardEditLockKey("app-1", "board-1")).not.toBe(
			boardEditLockKey("app-1"),
		);
		// An unknown target ("" or undefined) always falls back to the app-scoped key.
		expect(boardEditLockKey("app-1", "")).toBe(boardEditLockKey("app-1"));
		expect(boardEditLockKey("app-1", undefined)).toBe(
			boardEditLockKey("app-1"),
		);
	});

	test("edits on different boards of the same app overlap", async () => {
		const coordinator = new BoardEditCoordinator();
		const releaseFirst = await coordinator.acquire(
			boardEditLockKey("app-1", "board-1"),
		);
		const releaseSecond = await coordinator.acquire(
			boardEditLockKey("app-1", "board-2"),
		);
		releaseSecond();
		releaseFirst();
	});

	test("create-mode runs without a board target serialize per app", async () => {
		const coordinator = new BoardEditCoordinator();
		const releaseFirst = await coordinator.acquire(boardEditLockKey("app-1"));
		let secondAcquired = false;
		const second = coordinator
			.acquire(boardEditLockKey("app-1", undefined))
			.then((release) => {
				secondAcquired = true;
				return release;
			});
		await Promise.resolve();
		expect(secondAcquired).toBe(false);
		releaseFirst();
		const releaseSecond = await second;
		expect(secondAcquired).toBe(true);
		releaseSecond();
	});

	test("an unresolved run that resolves to an explicit board's target serializes with it", async () => {
		// Mirrors the bridge protocol: a create-mode run holds the app-scoped key, resolves its
		// concrete board, then must also acquire that board's key before touching the board.
		const coordinator = new BoardEditCoordinator();
		const releaseExplicit = await coordinator.acquire(
			boardEditLockKey("app-1", "board-1"),
		);

		const releaseAppScoped = await coordinator.acquire(
			boardEditLockKey("app-1"),
		);
		let boardScopedAcquired = false;
		const boardScoped = coordinator
			.acquire(boardEditLockKey("app-1", "board-1"))
			.then((release) => {
				boardScopedAcquired = true;
				return release;
			});
		await Promise.resolve();
		expect(boardScopedAcquired).toBe(false);

		releaseExplicit();
		const releaseBoardScoped = await boardScoped;
		expect(boardScopedAcquired).toBe(true);
		releaseBoardScoped();
		releaseAppScoped();
	});

	test("does not serialize unrelated apps", async () => {
		const coordinator = new BoardEditCoordinator();
		const releaseFirst = await coordinator.acquire(
			boardEditLockKey("app-1", "board-1"),
		);
		const releaseSecond = await coordinator.acquire(
			boardEditLockKey("app-2", "board-2"),
		);
		releaseSecond();
		releaseFirst();
	});

	test("keeps failed repair identity stable across assistant turns", () => {
		const firstTurnKey = boardEditRecoveryKey("app-1", "board-1");
		const retryTurnKey = boardEditRecoveryKey("app-1", "board-1");
		let now = 1_000;
		const recovery = new BoardEditRecoveryStore(100, 2, () => now, null);
		const retained = {
			source: completeSupportFlow,
			status: "validation_errors",
		};
		recovery.set(firstTurnKey, retained);

		expect(retryTurnKey).toBe(firstTurnKey);
		expect(boardEditRecoveryKey("app-1", "board-2")).not.toBe(firstTurnKey);
		expect(recovery.get(retryTurnKey)).toEqual(retained);
		now += 100;
		expect(recovery.get(retryTurnKey)).toBeUndefined();

		const bounded = new BoardEditRecoveryStore(1_000, 2, () => now, null);
		bounded.set("app:a", retained);
		bounded.set("app:b", retained);
		bounded.set("app:c", retained);
		expect(bounded.get("app:a")).toBeUndefined();
		expect(bounded.get("app:b")).toEqual(retained);
		expect(bounded.get("app:c")).toEqual(retained);
	});

	test("redacts canonical @secret values before durable recovery or plan persistence", () => {
		const source = `@secret
const IMAP_HOST: string = "imap.private.example"
@secret
const IMAP_USERNAME: string = "support@private.example"
@secret
const IMAP_PORT: int = 993

eventsSimple() {
  logInfo({ message: "safe" })
}`;
		const sanitized = sanitizeFlowScriptForPersistence(source);
		expect(sanitized.safe).toBe(true);
		if (!sanitized.safe) throw new Error(sanitized.reason);
		expect(sanitized.redactedDeclarations).toBe(3);
		expect(sanitized.source).toContain('const IMAP_HOST: string = ""');
		expect(sanitized.source).toContain('const IMAP_USERNAME: string = ""');
		expect(sanitized.source).toContain("const IMAP_PORT: int = 0");
		expect(sanitized.source).not.toContain("imap.private.example");
		expect(sanitized.source).not.toContain("support@private.example");
		expect(sanitized.source).not.toContain("993");

		const reasoning = safeFlowScriptPlanReasoning(source);
		expect(reasoning).toContain("Structure:");
		expect(reasoning).toContain('const IMAP_HOST: string = ""');
		expect(reasoning).not.toContain("imap.private.example");
		expect(reasoning).not.toContain("support@private.example");
	});

	test("redacts @secret declarations with canonical decorators stacked after the marker", () => {
		const source = `@secret
@category("IMAP")
@readonly
const imap_password: string = "mailbox-secret"

@secret
@description("SMTP credential")
@runtime
const smtp_password: string = "smtp-secret"

eventsSimple() {
  logInfo({ message: "safe" })
}`;

		const sanitized = sanitizeFlowScriptForPersistence(source);

		expect(sanitized.safe).toBe(true);
		if (!sanitized.safe) throw new Error(sanitized.reason);
		expect(sanitized.redactedDeclarations).toBe(2);
		expect(sanitized.source).toContain('@category("IMAP")');
		expect(sanitized.source).toContain("@readonly");
		expect(sanitized.source).toContain('const imap_password: string = ""');
		expect(sanitized.source).toContain('const smtp_password: string = ""');
		expect(sanitized.source).not.toMatch(/mailbox-secret|smtp-secret/);
	});

	test("accepts anchored @secret declarations with and without initializers", () => {
		const source = `@secret
const configured: string = "must-redact"   //@v:secret123

@secret
@runtime
const runtimeProvided: string   //@v:secret456`;

		const sanitized = sanitizeFlowScriptForPersistence(source);

		expect(sanitized.safe).toBe(true);
		if (!sanitized.safe) throw new Error(sanitized.reason);
		expect(sanitized.redactedDeclarations).toBe(2);
		expect(sanitized.source).toContain(
			'const configured: string = ""   //@v:secret123',
		);
		expect(sanitized.source).toContain(
			"const runtimeProvided: string   //@v:secret456",
		);
		expect(sanitized.source).not.toContain("must-redact");
	});

	test("accepts parser-valid whitespace and container types when no secret value exists", () => {
		const source = `@secret

@runtime
const runtimeTokens: string[]   //@v:secretArray

@secret
const tokenMap: Map<string, string>`;

		const sanitized = sanitizeFlowScriptForPersistence(source);

		expect(sanitized.safe).toBe(true);
		if (!sanitized.safe) throw new Error(sanitized.reason);
		expect(sanitized.redactedDeclarations).toBe(2);
		expect(sanitized.source).toBe(source);
	});

	test("does not cross comments or unknown decorators after @secret", () => {
		for (const source of [
			'@secret\n// do not detach secrecy\nconst innocuous: string = "opaque"',
			'@secret\n@custom("unknown")\nconst innocuous: string = "opaque"',
		]) {
			const sanitized = sanitizeFlowScriptForPersistence(source);
			expect(sanitized.safe).toBe(false);
		}
	});

	test("refuses unsafe secret syntax instead of partially persisting it", () => {
		const source = `@secret
const IMAP_PASSWORD: string = resolveSecret("must-not-leak")

eventsSimple() {
  logInfo({ message: "ready" })
}`;
		const sanitized = sanitizeFlowScriptForPersistence(source);
		expect(sanitized.safe).toBe(false);
		expect(safeFlowScriptPlanReasoning(source)).toContain(
			"source omitted from persisted plan details",
		);
		expect(safeFlowScriptPlanReasoning(source)).not.toContain("must-not-leak");

		const storage = new MemoryRecoveryStorage();
		const active = new BoardEditRecoveryStore(1_000, 64, () => 10, storage);
		const priorSafe = `eventsSimple() {
  logInfo({ message: "previous safe recovery" })
}`;
		active.set("app:board", {
			source: priorSafe,
			status: "validation_errors",
		});
		active.set("app:board", { source, status: "validation_errors" });
		// The active renderer can still repair it, but no raw or partial source reaches storage.
		expect(active.get("app:board")?.source).toBe(source);
		expect([...storage.values.values()].join("\n")).not.toContain(
			"must-not-leak",
		);
		const reloaded = new BoardEditRecoveryStore(1_000, 64, () => 10, storage);
		expect(reloaded.get("app:board")?.source).toBe(priorSafe);
	});

	test("recovers across reload, keeps the richer draft, expires it, and durably deletes it", () => {
		let now = 1_000;
		const storage = new MemoryRecoveryStorage();
		const key = boardEditRecoveryKey("app-reload", "board-reload");
		const rich = completeSupportFlow
			.replace(
				'const IMAP_HOST: string = ""',
				'const IMAP_HOST: string = "imap.private"',
			)
			.replace(
				'const SMTP_HOST: string = ""',
				'const SMTP_HOST: string = "smtp.private"',
			);
		const tiny = `eventsSimple() {
  logInfo({ message: "test" })
}`;
		const first = new BoardEditRecoveryStore(100, 64, () => now, storage);
		first.set(key, { source: rich, status: "validation_errors" });
		first.set(key, { source: tiny, status: "queued" });

		const reloaded = new BoardEditRecoveryStore(100, 64, () => now, storage);
		const recovered = reloaded.get(key);
		expect(recovered?.source).toContain("function pollInbox");
		expect(recovered?.source).toContain('const IMAP_HOST: string = ""');
		expect(recovered?.source).not.toContain("imap.private");
		expect(recovered?.source).not.toContain("smtp.private");

		reloaded.delete(key);
		const afterDelete = new BoardEditRecoveryStore(100, 64, () => now, storage);
		expect(afterDelete.get(key)).toBeUndefined();

		first.set(key, { source: rich, status: "validation_errors" });
		now += 100;
		const expired = new BoardEditRecoveryStore(100, 64, () => now, storage);
		expect(expired.get(key)).toBeUndefined();
		expect([...storage.values.values()].join("\n")).not.toContain(
			"imap.private",
		);
	});

	test("pins build mutations to the app created in this turn", () => {
		expect(
			isCreatedAppBuildTargetMismatch({
				createdAppId: "new-app",
				requestedAppId: "older-similar-app",
				toolName: "flowpilot_board",
				mode: "edit",
			}),
		).toBe(true);
		expect(
			isCreatedAppBuildTargetMismatch({
				createdAppId: "new-app",
				requestedAppId: "older-similar-app",
				toolName: "database_tool",
				operation: "create_table",
			}),
		).toBe(true);
		expect(
			isCreatedAppBuildTargetMismatch({
				createdAppId: "new-app",
				requestedAppId: "older-similar-app",
				toolName: "upsert_event",
			}),
		).toBe(true);
		expect(
			isCreatedAppBuildTargetMismatch({
				createdAppId: "new-app",
				requestedAppId: "older-similar-app",
				toolName: "flowpilot_board",
				mode: "explain",
			}),
		).toBe(false);
		expect(
			isCreatedAppBuildTargetMismatch({
				createdAppId: "new-app",
				requestedAppId: "new-app",
				toolName: "flowpilot_board",
				mode: "edit",
			}),
		).toBe(false);
	});

	test("retries a transient 404 only for the app created by the owning turn", async () => {
		let attempts = 0;
		const delays: number[] = [];
		const result = await retryCreatedAppReadiness(
			async () => {
				attempts += 1;
				if (attempts < 3) {
					throw new ApiResponseError({
						status: 404,
						message: "Not Found",
					});
				}
				return "ready";
			},
			{
				appId: "new-app",
				createdAppId: "new-app",
				backoffMs: [10, 20],
				wait: async (delayMs) => {
					delays.push(delayMs);
				},
			},
		);

		expect(result).toBe("ready");
		expect(attempts).toBe(3);
		expect(delays).toEqual([10, 20]);

		attempts = 0;
		await expect(
			retryCreatedAppReadiness(
				async () => {
					attempts += 1;
					throw new ApiResponseError({
						status: 404,
						message: "Not Found",
					});
				},
				{
					appId: "older-app",
					createdAppId: "new-app",
					wait: async () => undefined,
				},
			),
		).rejects.toMatchObject({ status: 404 });
		expect(attempts).toBe(1);
	});

	test("treats an empty initial-board list as transient readiness, not permission to create a duplicate", async () => {
		let attempts = 0;
		const boards = await retryCreatedAppReadiness(
			async () => {
				attempts += 1;
				return attempts < 3 ? [] : [{ id: "initial-board" }];
			},
			{
				appId: "new-app",
				createdAppId: "new-app",
				backoffMs: [10, 20],
				wait: async () => undefined,
				isReady: (value) => value.length > 0,
			},
		);

		expect(attempts).toBe(3);
		expect(boards).toEqual([{ id: "initial-board" }]);
	});

	test("created-app readiness retry respects abort and deadline budgets", async () => {
		const abort = new AbortController();
		abort.abort();
		await expect(
			retryCreatedAppReadiness(async () => "unreachable", {
				appId: "new-app",
				createdAppId: "new-app",
				signal: abort.signal,
			}),
		).rejects.toMatchObject({ name: "AbortError" });

		let attempts = 0;
		const notReady = new ApiResponseError({
			status: 404,
			message: "Not Found",
		});
		await expect(
			retryCreatedAppReadiness(
				async () => {
					attempts += 1;
					throw notReady;
				},
				{
					appId: "new-app",
					createdAppId: "new-app",
					deadlineAtMs: 1_050,
					now: () => 1_000,
					backoffMs: [100],
					wait: async () => {
						throw new Error("deadline-aware retry must not sleep");
					},
				},
			),
		).rejects.toBe(notReady);
		expect(attempts).toBe(1);
	});

	test("cancels a queued acquisition without poisoning the lock queue", async () => {
		const coordinator = new BoardEditCoordinator();
		const key = boardEditLockKey("app-1", "board-1");
		const releaseFirst = await coordinator.acquire(key);
		const abort = new AbortController();
		const cancelled = coordinator.acquire(key, { signal: abort.signal });
		abort.abort();
		await expect(cancelled).rejects.toMatchObject({ name: "AbortError" });

		releaseFirst();
		const releaseNext = await coordinator.acquire(key);
		releaseNext();
	});

	test("keeps an acquired owner after abort until its mutation explicitly settles", async () => {
		const coordinator = new BoardEditCoordinator();
		const key = boardEditLockKey("app-1", "board-1");
		const ownerAbort = new AbortController();
		let ownerInvalidated = false;
		const releaseOwner = await coordinator.acquire(key, {
			signal: ownerAbort.signal,
			leaseMs: 1_000,
			onInvalidated: () => {
				ownerInvalidated = true;
			},
		});

		let retryAcquired = false;
		const retry = coordinator.acquire(key).then((release) => {
			retryAcquired = true;
			return release;
		});
		ownerAbort.abort();
		await Promise.resolve();

		expect(ownerInvalidated).toBe(true);
		expect(retryAcquired).toBe(false);

		releaseOwner();
		const releaseRetry = await retry;
		expect(retryAcquired).toBe(true);
		releaseRetry();
	});

	test("does not expire a healthy active board owner on an arbitrary default lease", async () => {
		vi.useFakeTimers();
		try {
			const coordinator = new BoardEditCoordinator();
			const key = boardEditLockKey("app-long", "board-long");
			let ownerInvalidated = false;
			const releaseOwner = await coordinator.acquire(key, {
				onInvalidated: () => {
					ownerInvalidated = true;
				},
			});
			let nextAcquired = false;
			const next = coordinator.acquire(key).then((release) => {
				nextAcquired = true;
				return release;
			});

			vi.advanceTimersByTime(24 * 60 * 60_000);
			await Promise.resolve();
			expect(ownerInvalidated).toBe(false);
			expect(nextAcquired).toBe(false);

			releaseOwner();
			const releaseNext = await next;
			expect(nextAcquired).toBe(true);
			releaseNext();
		} finally {
			vi.useRealTimers();
		}
	});

	test("expires an acquired lease so the next edit can proceed", async () => {
		const coordinator = new BoardEditCoordinator();
		const key = boardEditLockKey("app-1", "board-1");
		let staleOwnerInvalidated = false;
		const releaseExpired = await coordinator.acquire(key, {
			leaseMs: 10,
			onInvalidated: () => {
				staleOwnerInvalidated = true;
			},
		});
		let nextAcquired = false;
		const next = coordinator.acquire(key).then((release) => {
			nextAcquired = true;
			return release;
		});

		await new Promise((resolve) => setTimeout(resolve, 20));
		const releaseNext = await next;
		expect(nextAcquired).toBe(true);
		expect(staleOwnerInvalidated).toBe(true);
		// A stale owner cannot release the new owner's lease.
		releaseExpired();
		releaseNext();
	});

	test("detects a stale authoritative snapshot before apply", () => {
		expect(
			flowScriptSnapshotChanged(
				"eventsSimple() {\n  pollInbox()\n}",
				'eventsSimple() {\n  printInfo({ message: "other run" })\n}',
			),
		).toBe(true);
		expect(
			flowScriptSnapshotChanged("eventsSimple() {}\r\n", "eventsSimple() {}\n"),
		).toBe(false);
	});

	test("rejects command-only success when persisted FlowScript did not change", () => {
		expect(
			assessFlowScriptReadback({
				before: "eventsSimple() {}",
				expected: completeSupportFlow,
				actual: "eventsSimple() {}",
			}),
		).toMatchObject({ ok: false, code: "not_persisted" });
	});

	test("rejects a persisted two-node stub in place of the complete workflow", () => {
		const assessment = assessFlowScriptReadback({
			before: "",
			expected: completeSupportFlow,
			actual: `eventsSimple() {
  printInfo({ message: "test" })
}`,
		});

		expect(assessment.ok).toBe(false);
		expect(assessment.code).toMatch(/completeness_regression|scope_missing/);
	});

	test("accepts persisted readback that retains the validated workflow scope", () => {
		const assessment = assessFlowScriptReadback({
			before: "",
			expected: completeSupportFlow,
			actual: completeSupportFlow.replace(
				"function pollInbox()",
				"//@n:poll\nfunction pollInbox()",
			),
		});

		expect(assessment).toEqual({ ok: true });
	});

	test("returns an actionable retained workspace when a board run times out", () => {
		const result = boardEditInterruptionResult({
			status: "timeout",
			code: "frontend_execution_deadline",
			message: "deadline exceeded",
			candidate: {
				source: completeSupportFlow,
				status: "validation_errors",
			},
		});

		expect(result).toMatchObject({
			status: "timeout",
			code: "frontend_execution_deadline",
			flowscript_status: "retained_for_repair",
			retained_status: "validation_errors",
		});
		expect(result.retained_flowscript).toContain("pollInbox");
		expect(result.next_action).toContain("Repair it in place");
	});
});

describe("created artifact journal", () => {
	const DAY_MS = 24 * 60 * 60_000;

	const identity = (overrides: Partial<CreatedArtifactRequestIdentity> = {}) =>
		({
			conversationId: "conversation-1",
			toolName: "create_app",
			instruction: "Weather App\nShows the forecast",
			...overrides,
		}) satisfies CreatedArtifactRequestIdentity;

	test("normalizes instruction phrasing into one creation fingerprint", () => {
		expect(
			creationRequestFingerprint({ instruction: "  Weather   App \n now " }),
		).toBe(creationRequestFingerprint({ instruction: "weather app now" }));
		expect(
			creationRequestFingerprint({
				scope: "app-a",
				instruction: "build page",
			}),
		).not.toBe(
			creationRequestFingerprint({ scope: "app-b", instruction: "build page" }),
		);
		expect(creationRequestFingerprint({ instruction: "   " })).toBeUndefined();
		expect(
			creationRequestFingerprint({
				instruction: "anything",
				idempotencyKey: "key-1",
			}),
		).toBe("key:key-1");
	});

	test("records a creation and answers the equivalent retried request", () => {
		const journal = new CreatedArtifactJournal(
			DAY_MS,
			8,
			() => 1_000,
			new MemoryRecoveryStorage(),
		);
		expect(
			journal.record(identity(), { appId: "app-123" }, "request-1"),
		).toBeDefined();

		const hit = journal.find(
			identity({ instruction: "  weather APP \n shows the forecast  " }),
		);
		expect(hit?.artifacts.appId).toBe("app-123");
		expect(hit?.toolCallId).toBe("request-1");

		expect(
			journal.find(identity({ conversationId: "conversation-2" })),
		).toBeUndefined();
		expect(
			journal.find(identity({ toolName: "flowpilot_widget" })),
		).toBeUndefined();
		expect(
			journal.find(identity({ instruction: "Todo App\nTracks tasks" })),
		).toBeUndefined();
	});

	test("prefers an explicit idempotency key over the instruction hash", () => {
		const journal = new CreatedArtifactJournal(
			DAY_MS,
			8,
			() => 1_000,
			new MemoryRecoveryStorage(),
		);
		journal.record(identity({ idempotencyKey: "stable-key" }), {
			appId: "app-key",
		});

		expect(
			journal.find(
				identity({
					instruction: "completely different phrasing",
					idempotencyKey: "stable-key",
				}),
			)?.artifacts.appId,
		).toBe("app-key");
		// A distinct key forces a genuinely separate creation even for the same instruction.
		expect(
			journal.find(identity({ idempotencyKey: "another-key" })),
		).toBeUndefined();
		// Without the key the instruction hash is a different fingerprint.
		expect(journal.find(identity())).toBeUndefined();
	});

	test("expires entries after the journal ttl", () => {
		let now = 0;
		const journal = new CreatedArtifactJournal(
			7 * DAY_MS,
			8,
			() => now,
			new MemoryRecoveryStorage(),
		);
		journal.record(identity(), { appId: "app-123" });

		now = 7 * DAY_MS - 1;
		expect(journal.find(identity())?.artifacts.appId).toBe("app-123");
		now = 7 * DAY_MS;
		expect(journal.find(identity())).toBeUndefined();
		expect(journal.size).toBe(0);
	});

	test("caps the journal and evicts the oldest entries first", () => {
		let now = 0;
		const journal = new CreatedArtifactJournal(
			7 * DAY_MS,
			2,
			() => now,
			new MemoryRecoveryStorage(),
		);
		journal.record(identity({ instruction: "first app" }), { appId: "app-1" });
		now += 1;
		journal.record(identity({ instruction: "second app" }), {
			appId: "app-2",
		});
		now += 1;
		journal.record(identity({ instruction: "third app" }), { appId: "app-3" });

		expect(journal.size).toBe(2);
		expect(
			journal.find(identity({ instruction: "first app" })),
		).toBeUndefined();
		expect(
			journal.find(identity({ instruction: "second app" }))?.artifacts.appId,
		).toBe("app-2");
		expect(
			journal.find(identity({ instruction: "third app" }))?.artifacts.appId,
		).toBe("app-3");
	});

	test("survives a renderer restart through persisted storage", () => {
		const storage = new MemoryRecoveryStorage();
		let now = 1_000;
		const journal = new CreatedArtifactJournal(
			7 * DAY_MS,
			8,
			() => now,
			storage,
		);
		journal.record(identity(), { appId: "app-123", boardId: "board-456" });
		journal.record(
			identity({ toolName: "flowpilot_widget", scope: "app-123" }),
			{
				appId: "app-123",
				pageId: "page-1",
				widgetIds: ["widget-1", "widget-2"],
			},
		);

		const restarted = new CreatedArtifactJournal(
			7 * DAY_MS,
			8,
			() => now,
			storage,
		);
		expect(restarted.find(identity())?.artifacts).toEqual({
			appId: "app-123",
			boardId: "board-456",
		});
		expect(
			restarted.find(
				identity({ toolName: "flowpilot_widget", scope: "app-123" }),
			)?.artifacts.widgetIds,
		).toEqual(["widget-1", "widget-2"]);

		// Entries past the ttl are dropped on hydrate instead of being revived.
		now = 1_000 + 7 * DAY_MS;
		const expired = new CreatedArtifactJournal(
			7 * DAY_MS,
			8,
			() => now,
			storage,
		);
		expect(expired.size).toBe(0);
	});

	test("ignores corrupt or unsafe persisted journal payloads", () => {
		const storage = new MemoryRecoveryStorage();
		storage.setItem("flowpilot.created-artifact-journal.v1", "{not json");
		const corrupt = new CreatedArtifactJournal(
			7 * DAY_MS,
			8,
			() => 1_000,
			storage,
		);
		expect(corrupt.size).toBe(0);
		expect(storage.getItem("flowpilot.created-artifact-journal.v1")).toBeNull();

		storage.setItem(
			"flowpilot.created-artifact-journal.v1",
			JSON.stringify({
				version: 1,
				entries: [
					{
						conversationId: "conversation-1",
						toolName: "create_app",
						requestFingerprint: "hash:abc",
						artifacts: { appId: "javascript:alert(1)//" },
						createdAtMs: 500,
					},
				],
			}),
		);
		const unsafe = new CreatedArtifactJournal(
			7 * DAY_MS,
			8,
			() => 1_000,
			storage,
		);
		expect(unsafe.size).toBe(0);
	});

	test("duplicate-create short-circuit flow returns the recorded ids", () => {
		// Mirrors the bridge flow: consult before creating, record after success.
		const journal = new CreatedArtifactJournal(
			7 * DAY_MS,
			8,
			() => 1_000,
			new MemoryRecoveryStorage(),
		);
		const request = identity({
			toolName: "flowpilot_board",
			scope: "app-123",
			instruction: "Build the intake workflow",
		});

		const createBoard = () => {
			const existing = journal.find(request)?.artifacts.boardId;
			if (existing) return { boardId: existing, created: false };
			const boardId = "board-1";
			journal.record(request, { appId: "app-123", boardId });
			return { boardId, created: true };
		};

		expect(createBoard()).toEqual({ boardId: "board-1", created: true });
		// The crash-retry of the same conversation + instruction reuses the recorded board.
		expect(createBoard()).toEqual({ boardId: "board-1", created: false });
	});
});

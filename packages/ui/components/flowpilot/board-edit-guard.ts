import { ApiResponseError } from "../../lib/api-error";
import { sanitizeFlowScriptForPersistence } from "../../lib/flowscript-persistence";
import {
	type FlowScriptWorkspaceCandidate,
	detectFlowScriptCandidateRegression,
	profileFlowScriptCandidate,
	selectBestRecoverableFlowScriptCandidate,
} from "./flowscript-workspace-candidates";

const CREATED_APP_READINESS_BACKOFF_MS = [100, 250, 500, 1_000] as const;

export const FRONTEND_TOOL_DEADLINE_SAFETY_MARGIN_MS = 5_000;

export interface FrontendRequestOwnershipActivity {
	requestId: string;
	parentRequestId?: string;
}

/** Keep correlation while either the owning request or one of its nested tools is still active. */
export function hasActiveFrontendRequestOwnership(
	ownerRequestId: string,
	activeRequests: Iterable<FrontendRequestOwnershipActivity>,
): boolean {
	for (const active of activeRequests) {
		if (
			active.requestId === ownerRequestId ||
			active.parentRequestId === ownerRequestId
		) {
			return true;
		}
	}
	return false;
}

export interface FrontendRequestExecutionLease<TRequest extends object> {
	readonly request: TRequest;
	readonly requestId: string;
	readonly parentRequestId?: string;
	readonly generation: number;
	readonly controller: AbortController;
	invalidated: boolean;
	settled: boolean;
}

/**
 * Tracks the lifetime of the actual frontend-tool execution, not just the response delivered to
 * its caller. A timed-out request can return a response while an underlying non-abortable mutation
 * is still unwinding; its invalidation fence therefore remains authoritative until that execution
 * and every active descendant have settled.
 *
 * Generations keep a late completion for a duplicated request id from removing or re-authorizing a
 * newer execution. There is no elapsed-time eviction: settled/quiescent executions are the only
 * entries removed, so the registry stays bounded by work that can still produce side effects.
 */
export class FrontendRequestExecutionFence<TRequest extends object> {
	private static readonly MAX_PENDING_INVALIDATIONS = 1_024;
	private readonly executions = new Map<
		string,
		Map<number, FrontendRequestExecutionLease<TRequest>>
	>();
	private readonly nextGeneration = new Map<string, number>();
	// A cancellation can be delivered just before its request event. Consume that tombstone when
	// the execution registers, then retain the resulting invalidated lease until it settles.
	private readonly pendingInvalidations = new Set<string>();
	// A broken/reordered producer must not grow tombstones forever. Overflow fails closed instead of
	// evicting an older cancellation and accidentally re-authorizing it.
	private rejectUnregisteredExecutions = false;

	begin(options: {
		request: TRequest;
		requestId: string;
		parentRequestId?: string;
	}): FrontendRequestExecutionLease<TRequest> {
		const generation = this.nextGeneration.get(options.requestId) ?? 0;
		this.nextGeneration.set(options.requestId, generation + 1);
		const priorExecutions = this.executions.get(options.requestId);
		const blockedByOverlappingGeneration = Boolean(
			priorExecutions &&
				Array.from(priorExecutions.values()).some(
					(execution) => !execution.settled,
				),
		);
		const blockedByPriorInvalidation = Boolean(
			priorExecutions &&
				Array.from(priorExecutions.values()).some(
					(execution) => execution.invalidated,
				),
		);
		for (const prior of priorExecutions?.values() ?? []) {
			if (prior.settled) continue;
			prior.invalidated = true;
			prior.controller.abort();
		}
		const controller = new AbortController();
		const execution: FrontendRequestExecutionLease<TRequest> = {
			request: options.request,
			requestId: options.requestId,
			parentRequestId: options.parentRequestId,
			generation,
			controller,
			invalidated:
				this.rejectUnregisteredExecutions ||
				this.pendingInvalidations.delete(options.requestId) ||
				blockedByOverlappingGeneration ||
				blockedByPriorInvalidation ||
				this.hasInvalidatedAncestor(options.parentRequestId),
			settled: false,
		};
		let generations = priorExecutions;
		if (!generations) {
			generations = new Map();
			this.executions.set(options.requestId, generations);
		}
		generations.set(generation, execution);
		if (execution.invalidated) controller.abort();
		return execution;
	}

	invalidate(requestId: string): void {
		if (!this.executions.has(requestId)) {
			if (
				this.pendingInvalidations.size >=
				FrontendRequestExecutionFence.MAX_PENDING_INVALIDATIONS
			) {
				this.pendingInvalidations.clear();
				this.rejectUnregisteredExecutions = true;
			} else if (!this.rejectUnregisteredExecutions) {
				this.pendingInvalidations.add(requestId);
			}
		}
		const requestIds = new Set([requestId]);
		let foundDescendant = true;
		while (foundDescendant) {
			foundDescendant = false;
			for (const generations of this.executions.values()) {
				for (const execution of generations.values()) {
					if (
						execution.parentRequestId &&
						requestIds.has(execution.parentRequestId) &&
						!requestIds.has(execution.requestId)
					) {
						requestIds.add(execution.requestId);
						foundDescendant = true;
					}
				}
			}
		}

		for (const id of requestIds) {
			for (const execution of this.executions.get(id)?.values() ?? []) {
				execution.invalidated = true;
				execution.controller.abort();
			}
		}
	}

	isInvalidated(execution: FrontendRequestExecutionLease<TRequest>): boolean {
		return execution.invalidated || execution.controller.signal.aborted;
	}

	settle(execution: FrontendRequestExecutionLease<TRequest>): void {
		const retained = this.executions
			.get(execution.requestId)
			?.get(execution.generation);
		if (retained !== execution) return;
		execution.settled = true;
		this.pruneSettledExecutions();
	}

	getLatest(
		requestId: string,
	): FrontendRequestExecutionLease<TRequest> | undefined {
		const generations = this.executions.get(requestId);
		if (!generations) return undefined;
		let latest: FrontendRequestExecutionLease<TRequest> | undefined;
		for (const execution of generations.values()) {
			if (!latest || execution.generation > latest.generation)
				latest = execution;
		}
		return latest;
	}

	activeExecutions(): FrontendRequestExecutionLease<TRequest>[] {
		const active: FrontendRequestExecutionLease<TRequest>[] = [];
		for (const generations of this.executions.values()) {
			for (const execution of generations.values()) {
				if (!execution.settled) active.push(execution);
			}
		}
		return active;
	}

	get size(): number {
		let size = 0;
		for (const generations of this.executions.values()) {
			size += generations.size;
		}
		return size;
	}

	get pendingInvalidationCount(): number {
		return this.pendingInvalidations.size;
	}

	get isFailClosed(): boolean {
		return this.rejectUnregisteredExecutions;
	}

	private hasInvalidatedAncestor(parentRequestId?: string): boolean {
		const visited = new Set<string>();
		let candidate = parentRequestId;
		while (candidate && !visited.has(candidate)) {
			visited.add(candidate);
			if (this.pendingInvalidations.has(candidate)) return true;
			const generations = this.executions.get(candidate);
			if (!generations) return false;
			let nextParent: string | undefined;
			for (const execution of generations.values()) {
				if (execution.invalidated) return true;
				nextParent ??= execution.parentRequestId;
			}
			candidate = nextParent;
		}
		return false;
	}

	private pruneSettledExecutions(): void {
		let removed = true;
		while (removed) {
			removed = false;
			for (const [requestId, generations] of this.executions) {
				for (const [generation, execution] of generations) {
					if (!execution.settled) continue;
					if (
						execution.invalidated &&
						this.hasUnsettledDescendant(execution.requestId)
					) {
						continue;
					}
					generations.delete(generation);
					removed = true;
				}
				if (generations.size === 0) {
					this.executions.delete(requestId);
					this.nextGeneration.delete(requestId);
				}
			}
		}
	}

	private hasUnsettledDescendant(requestId: string): boolean {
		const descendants = new Set([requestId]);
		let foundDescendant = true;
		while (foundDescendant) {
			foundDescendant = false;
			for (const generations of this.executions.values()) {
				for (const execution of generations.values()) {
					if (
						execution.parentRequestId &&
						descendants.has(execution.parentRequestId) &&
						!descendants.has(execution.requestId)
					) {
						if (!execution.settled) return true;
						descendants.add(execution.requestId);
						foundDescendant = true;
					}
				}
			}
		}
		return false;
	}
}

export interface FrontendToolExecutionDeadlineOptions {
	toolName: string;
	backendDeadlineAtMs?: number;
	backendSafetyMarginMs?: number;
}

/**
 * Resolve the renderer deadline for a frontend tool.
 *
 * The renderer only subtracts a small settlement margin from the backend's per-tool watchdog. It
 * must not invent a shorter elapsed-time cap: active delegated provider runs can legitimately take
 * longer, and the backend owns cancellation/owner-loss policy.
 */
export function resolveFrontendToolExecutionDeadline(
	options: FrontendToolExecutionDeadlineOptions,
): number | undefined {
	const backendDeadlineAtMs = Number.isFinite(options.backendDeadlineAtMs)
		? options.backendDeadlineAtMs
		: undefined;
	const backendSafetyMarginMs = Number.isFinite(options.backendSafetyMarginMs)
		? Math.max(0, options.backendSafetyMarginMs ?? 0)
		: FRONTEND_TOOL_DEADLINE_SAFETY_MARGIN_MS;
	const safeBackendDeadline =
		backendDeadlineAtMs === undefined
			? undefined
			: backendDeadlineAtMs - backendSafetyMarginMs;
	return safeBackendDeadline;
}

export interface CreatedAppReadinessRetryOptions<T = unknown> {
	/** The app returned by create_app for the owning assistant message. */
	createdAppId?: string;
	/** The app targeted by this board read. */
	appId: string;
	signal?: AbortSignal;
	deadlineAtMs?: number;
	backoffMs?: readonly number[];
	/** Injectable clock/waiter keep the retry contract deterministic in tests. */
	now?: () => number;
	wait?: (delayMs: number, signal?: AbortSignal) => Promise<void>;
	/** Optional readiness check for APIs that temporarily return an empty success payload. */
	isReady?: (value: T) => boolean;
}

function readinessAbortError(message: string) {
	const error = new Error(message);
	error.name = "AbortError";
	return error;
}

function waitForCreatedAppReadiness(
	delayMs: number,
	signal?: AbortSignal,
): Promise<void> {
	if (signal?.aborted) {
		return Promise.reject(
			readinessAbortError("Created-app readiness retry was cancelled."),
		);
	}
	return new Promise((resolve, reject) => {
		let settled = false;
		const finish = (error?: Error) => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			signal?.removeEventListener("abort", abort);
			if (error) reject(error);
			else resolve();
		};
		const abort = () =>
			finish(readinessAbortError("Created-app readiness retry was cancelled."));
		const timer = setTimeout(() => finish(), delayMs);
		signal?.addEventListener("abort", abort, { once: true });
	});
}

/** True only for a missing-resource response that can be caused by app/board propagation. */
export function isCreatedAppReadinessError(error: unknown): boolean {
	if (error instanceof ApiResponseError) return error.status === 404;
	if (error && typeof error === "object" && "status" in error) {
		return (error as { status?: unknown }).status === 404;
	}
	const message = error instanceof Error ? error.message : String(error ?? "");
	return /(?:\bHTTP[_ -]?404\b|\b404\b|\bnot found\b)/i.test(message);
}

/**
 * Retry the brief 404 window after create_app without turning arbitrary board failures into a
 * polling loop. The retry is enabled only when the requested id exactly matches the id created by
 * the same owning assistant message. It never discovers or substitutes a different app.
 */
export async function retryCreatedAppReadiness<T>(
	operation: () => Promise<T>,
	options: CreatedAppReadinessRetryOptions<T>,
): Promise<T> {
	const retryEnabled =
		Boolean(options.createdAppId) && options.createdAppId === options.appId;
	const backoff = options.backoffMs ?? CREATED_APP_READINESS_BACKOFF_MS;
	const now = options.now ?? Date.now;
	const wait = options.wait ?? waitForCreatedAppReadiness;

	for (let attempt = 0; ; attempt += 1) {
		if (options.signal?.aborted) {
			throw readinessAbortError("Created-app readiness retry was cancelled.");
		}
		if (options.deadlineAtMs !== undefined && now() >= options.deadlineAtMs) {
			throw readinessAbortError(
				"Created-app readiness retry reached the request deadline.",
			);
		}

		try {
			const value = await operation();
			if (!retryEnabled || !options.isReady || options.isReady(value)) {
				return value;
			}
			throw new ApiResponseError({
				status: 404,
				code: "CREATED_APP_NOT_READY",
				message:
					"The app was created, but its initial board is not readable yet.",
			});
		} catch (error) {
			const delayMs = backoff[attempt];
			if (
				!retryEnabled ||
				delayMs === undefined ||
				!isCreatedAppReadinessError(error)
			) {
				throw error;
			}

			// Do not sleep away the remaining parent/tool budget. Preserve the original 404 when
			// there is not enough time for both the backoff and another read attempt.
			if (
				options.deadlineAtMs !== undefined &&
				now() + delayMs >= options.deadlineAtMs
			) {
				throw error;
			}
			await wait(delayMs, options.signal);
		}
	}
}

export function boardEditLockKey(appId: string, boardId?: string) {
	// App-wide serialization is intentionally conservative: a request without a board id can
	// create/select a board while another request already targets that board explicitly.
	void boardId;
	return appId;
}

/**
 * Failed FlowScript drafts belong to the persisted board, not to one assistant message.
 *
 * Keeping the owner/message id in this key made the closest repair draft disappear as soon as a
 * timed-out tool call was retried in a new turn. The board then looked like its old Event + log
 * stub again and the next specialist had no richer candidate to continue from.
 */
export function boardEditRecoveryKey(appId: string, boardId: string) {
	return `${appId}:${boardId}`;
}

/**
 * Build the host-owned recovery instruction for a nested board specialist.
 *
 * The delegated platform model can lose a long tool response and then mistakenly request a tiny
 * diagnostic replacement. Put recovery policy immediately next to the retained workspace so the
 * specialist can distinguish an actual new user scope change from the caller's own fallback idea.
 */
export function retainedFlowScriptRecoveryInstruction(source: string) {
	const retained = source.trim();
	if (!retained) return "";
	return `

HOST RECOVERY POLICY — this takes precedence over any conflicting diagnostic fallback proposed by the delegating platform assistant:
- The retained document below is the active production workspace even when the persisted/live board is empty.
- Repair this exact document in place from validator diagnostics. Preserve its helpers, variables, Events, and requested capabilities; do not restart discovery.
- An instruction above saying "minimal diagnostic", "start fresh", "empty Event", or "single log/notify" is not an end-user scope change. Ignore that reduction and continue repairing the retained candidate. Only a clearly quoted NEW request from the actual end user may reduce or discard it.
- Own the edit_flowscript validation loop in this run. Do not ask the user to choose a stub or partial replacement.

RECOVERED CANDIDATE FROM AN EARLIER INTERRUPTED ATTEMPT ON THIS BOARD:
\`\`\`flowscript
${retained}
\`\`\``;
}

interface BoardEditRecoveryEntry {
	candidate: FlowScriptWorkspaceCandidate;
	retainedAtMs: number;
}

export interface BoardEditRecoveryStorage {
	getItem(key: string): string | null;
	setItem(key: string, value: string): void;
	removeItem(key: string): void;
}

interface PersistedBoardEditRecovery {
	version: 1;
	entries: Array<{
		key: string;
		candidate: FlowScriptWorkspaceCandidate;
		retainedAtMs: number;
	}>;
}

const BOARD_EDIT_RECOVERY_STORAGE_KEY = "flowpilot.board-edit-recovery.v1";
const MAX_RECOVERY_SOURCE_CHARS = 256 * 1024;
const MAX_RECOVERY_STORAGE_CHARS = 2 * 1024 * 1024;

function browserBoardEditRecoveryStorage(): BoardEditRecoveryStorage | null {
	if (typeof window === "undefined") return null;
	try {
		return window.localStorage;
	} catch {
		return null;
	}
}

function persistedCandidate(
	candidate: FlowScriptWorkspaceCandidate,
): FlowScriptWorkspaceCandidate | undefined {
	if (
		!candidate.source.trim() ||
		candidate.source.length > MAX_RECOVERY_SOURCE_CHARS
	) {
		return undefined;
	}
	const sanitized = sanitizeFlowScriptForPersistence(candidate.source);
	if (!sanitized.safe) return undefined;
	const safeToken = (value: string | undefined) =>
		value && /^[a-z0-9_-]{1,64}$/i.test(value) ? value : undefined;
	return {
		source: sanitized.source,
		...(safeToken(candidate.status) ? { status: candidate.status } : {}),
		...(safeToken(candidate.completion)
			? { completion: candidate.completion }
			: {}),
	};
}

/**
 * Bounded board-scoped recovery across assistant messages and renderer/app reloads.
 *
 * Full candidates stay in memory for the active process. The durable local copy is conservative:
 * every canonical `@secret` initializer is replaced with a type-safe empty value, and a document
 * with any unparseable secret declaration is not written at all.
 */
export class BoardEditRecoveryStore {
	private readonly entries = new Map<string, BoardEditRecoveryEntry>();
	/** Sanitized subset; kept separate so an unsafe newer draft cannot erase a safe recovery. */
	private readonly durableEntries = new Map<string, BoardEditRecoveryEntry>();
	private readonly storage: BoardEditRecoveryStorage | null;

	constructor(
		private readonly ttlMs = 30 * 60_000,
		private readonly maxEntries = 64,
		private readonly now: () => number = Date.now,
		storage: BoardEditRecoveryStorage | null = browserBoardEditRecoveryStorage(),
		private readonly storageKey = BOARD_EDIT_RECOVERY_STORAGE_KEY,
	) {
		this.storage = storage;
		this.hydrate();
	}

	private hydrate() {
		if (!this.storage) return;
		try {
			const raw = this.storage.getItem(this.storageKey);
			if (!raw) return;
			const parsed = JSON.parse(raw) as Partial<PersistedBoardEditRecovery>;
			if (parsed.version !== 1 || !Array.isArray(parsed.entries)) {
				this.storage.removeItem(this.storageKey);
				return;
			}
			const now = this.now();
			for (const entry of parsed.entries.slice(-this.maxEntries)) {
				if (
					!entry ||
					typeof entry.key !== "string" ||
					entry.key.length > 512 ||
					typeof entry.retainedAtMs !== "number" ||
					!Number.isFinite(entry.retainedAtMs) ||
					now - entry.retainedAtMs >= this.ttlMs ||
					!entry.candidate ||
					typeof entry.candidate.source !== "string"
				) {
					continue;
				}
				// Re-sanitize storage input before it is ever inserted into a repair prompt.
				const safe = persistedCandidate(entry.candidate);
				if (!safe) continue;
				const existing = this.entries.get(entry.key);
				if (!existing || existing.retainedAtMs <= entry.retainedAtMs) {
					const recovered = {
						candidate: safe,
						retainedAtMs: entry.retainedAtMs,
					};
					this.entries.set(entry.key, recovered);
					this.durableEntries.set(entry.key, recovered);
				}
			}
			this.persist();
		} catch {
			// Corrupt/blocked local storage must never prevent the board bridge from mounting.
			try {
				this.storage.removeItem(this.storageKey);
			} catch {
				// Storage itself may be unavailable (privacy mode/quota/security error).
			}
		}
	}

	private removeExpired() {
		const now = this.now();
		let changed = false;
		for (const [entryKey, entry] of this.entries) {
			if (now - entry.retainedAtMs >= this.ttlMs) {
				this.entries.delete(entryKey);
				changed = true;
			}
		}
		for (const [entryKey, entry] of this.durableEntries) {
			if (now - entry.retainedAtMs >= this.ttlMs) {
				this.durableEntries.delete(entryKey);
				changed = true;
			}
		}
		return changed;
	}

	private persist() {
		if (!this.storage) return;
		try {
			this.removeExpired();
			let entries = [...this.durableEntries]
				.map(([key, entry]) => {
					const candidate = persistedCandidate(entry.candidate);
					return candidate
						? { key, candidate, retainedAtMs: entry.retainedAtMs }
						: undefined;
				})
				.filter((entry): entry is NonNullable<typeof entry> => Boolean(entry));
			let serialized = JSON.stringify({ version: 1, entries });
			while (
				entries.length > 0 &&
				serialized.length > MAX_RECOVERY_STORAGE_CHARS
			) {
				entries = entries.slice(1);
				serialized = JSON.stringify({ version: 1, entries });
			}
			if (entries.length === 0) {
				this.storage.removeItem(this.storageKey);
			} else {
				this.storage.setItem(this.storageKey, serialized);
			}
		} catch {
			// Recovery persistence is best-effort; the in-memory candidate remains authoritative.
		}
	}

	get(key: string): FlowScriptWorkspaceCandidate | undefined {
		const entry = this.entries.get(key) ?? this.durableEntries.get(key);
		if (!entry) return undefined;
		if (this.now() - entry.retainedAtMs >= this.ttlMs) {
			this.entries.delete(key);
			this.durableEntries.delete(key);
			this.persist();
			return undefined;
		}
		return entry.candidate;
	}

	set(key: string, candidate: FlowScriptWorkspaceCandidate) {
		if (!candidate.source.trim()) return;
		let changed = this.removeExpired();
		const existing = this.get(key);
		const replaceMemory =
			!existing ||
			selectBestRecoverableFlowScriptCandidate([existing, candidate]) ===
				candidate;
		const now = this.now();
		if (replaceMemory) {
			// Refresh insertion order so bounded eviction removes the least recently retained entry.
			this.entries.delete(key);
			this.entries.set(key, { candidate, retainedAtMs: now });
			changed = true;
		}

		const safe = persistedCandidate(candidate);
		if (safe) {
			const existingDurable = this.durableEntries.get(key)?.candidate;
			const replaceDurable =
				!existingDurable ||
				selectBestRecoverableFlowScriptCandidate([existingDurable, safe]) ===
					safe;
			if (replaceDurable) {
				this.durableEntries.delete(key);
				this.durableEntries.set(key, {
					candidate: safe,
					retainedAtMs: now,
				});
				changed = true;
			}
		}

		while (this.entries.size > this.maxEntries) {
			const oldest = this.entries.keys().next().value;
			if (typeof oldest !== "string") break;
			this.entries.delete(oldest);
			changed = true;
		}
		while (this.durableEntries.size > this.maxEntries) {
			const oldest = this.durableEntries.keys().next().value;
			if (typeof oldest !== "string") break;
			this.durableEntries.delete(oldest);
			changed = true;
		}
		if (changed) this.persist();
	}

	delete(key: string) {
		const removedMemory = this.entries.delete(key);
		const removedDurable = this.durableEntries.delete(key);
		const changed = removedMemory || removedDurable;
		if (changed) this.persist();
	}
}

/** Structural + redacted source for plan steps that are checkpointed into the chat database. */
export function safeFlowScriptPlanReasoning(
	source: string,
	maxSourceChars = 6_000,
): string {
	const profile = profileFlowScriptCandidate(source);
	const structure = `Structure: ${profile.eventEntries} Event entr${profile.eventEntries === 1 ? "y" : "ies"}, ${profile.helperFunctions.length} helper function${profile.helperFunctions.length === 1 ? "" : "s"}, ${profile.callSites} call site${profile.callSites === 1 ? "" : "s"}, ${profile.meaningfulStatements} meaningful statement${profile.meaningfulStatements === 1 ? "" : "s"}.`;
	const sanitized = sanitizeFlowScriptForPersistence(source);
	if (!sanitized.safe) {
		return `${structure}\n\nFlowScript source omitted from persisted plan details: ${sanitized.reason}`;
	}
	return `${structure}\n\n\`\`\`flowscript\n${sanitized.source.slice(0, maxSourceChars)}\n\`\`\``;
}

const DATABASE_MUTATION_OPERATIONS = new Set([
	"create_table",
	"insert",
	"add_items",
	"delete",
	"remove_items",
	"update",
	"build_index",
	"drop_index",
	"optimize",
	"add_column",
	"drop_columns",
	"alter_column",
]);

export interface CreatedAppTargetMismatchOptions {
	createdAppId?: string;
	requestedAppId?: string;
	toolName: string;
	mode?: string;
	operation?: string;
}

/**
 * A whole-app build is pinned to the id returned by create_app for the rest of that assistant
 * turn. A transient fetch failure must not make the agent continue mutating a similarly named,
 * older app whose board may only contain a smoke-test stub.
 */
export function isCreatedAppBuildTargetMismatch(
	options: CreatedAppTargetMismatchOptions,
): boolean {
	if (
		!options.createdAppId ||
		!options.requestedAppId ||
		options.createdAppId === options.requestedAppId
	) {
		return false;
	}

	switch (options.toolName) {
		case "flowpilot_board":
		case "flowpilot_widget":
			return options.mode !== "explain";
		case "database_tool":
			return DATABASE_MUTATION_OPERATIONS.has(options.operation ?? "");
		case "upsert_event":
		case "delete_event":
		case "set_page_load_event":
			return true;
		default:
			return false;
	}
}

const DEFAULT_BOARD_EDIT_LEASE_MS = Number.POSITIVE_INFINITY;

export interface BoardEditAcquireOptions {
	/** Cancels a queued acquisition. Once acquired, it invalidates but does not release the owner. */
	signal?: AbortSignal;
	/** Optional hard maximum ownership time. Omitted owners remain serialized until settled. */
	leaseMs?: number;
	/** Absolute request deadline. It cancels a waiter and invalidates an acquired owner. */
	deadlineAtMs?: number;
	/** Runs when a waiter/owner is invalidated, allowing callers to block stale writes. */
	onInvalidated?: () => void;
}

interface BoardEditWaiter {
	token: symbol;
	resolve: (release: () => void) => void;
	reject: (error: Error) => void;
	signal?: AbortSignal;
	deadlineAtMs: number;
	deadlineTimer?: ReturnType<typeof setTimeout>;
	hardLeaseMs: number;
	hardLeaseTimer?: ReturnType<typeof setTimeout>;
	abortHandler?: () => void;
	onInvalidated?: () => void;
	invalidated: boolean;
	granted: boolean;
	settled: boolean;
	release?: () => void;
}

interface BoardEditLock {
	owner?: symbol;
	waiters: BoardEditWaiter[];
}

function boardEditAbortError(message: string) {
	const error = new Error(message);
	error.name = "AbortError";
	return error;
}

/**
 * In-renderer serialization for long-running board specialists targeting the same app.
 *
 * Request cancellation/deadline rejects queued waiters, but only invalidates an acquired owner: it
 * cannot release that owner while its non-abortable mutation may still be settling. Explicit
 * completion releases it normally. Callers may opt into a finite hard lease; active provider runs
 * have no default elapsed-time lease.
 */
export class BoardEditCoordinator {
	private readonly locks = new Map<string, BoardEditLock>();

	acquire(
		key: string,
		options: BoardEditAcquireOptions = {},
	): Promise<() => void> {
		const requestedLeaseMs = options.leaseMs ?? DEFAULT_BOARD_EDIT_LEASE_MS;
		const hardLeaseMs = Number.isFinite(requestedLeaseMs)
			? Math.max(1, requestedLeaseMs)
			: Number.POSITIVE_INFINITY;
		const deadlineAtMs = options.deadlineAtMs ?? Number.POSITIVE_INFINITY;
		if (options.signal?.aborted) {
			return Promise.reject(
				boardEditAbortError(
					`Board edit acquisition for '${key}' was cancelled.`,
				),
			);
		}
		if (deadlineAtMs <= Date.now()) {
			return Promise.reject(
				boardEditAbortError(`Board edit lease for '${key}' already expired.`),
			);
		}

		return new Promise<() => void>((resolve, reject) => {
			const lock = this.locks.get(key) ?? { waiters: [] };
			this.locks.set(key, lock);
			const waiter: BoardEditWaiter = {
				token: Symbol(key),
				resolve,
				reject,
				signal: options.signal,
				deadlineAtMs,
				hardLeaseMs,
				onInvalidated: options.onInvalidated,
				invalidated: false,
				granted: false,
				settled: false,
			};

			const invalidate = (reason: "cancelled" | "deadline") => {
				if (waiter.settled) return;
				if (!waiter.invalidated) {
					waiter.invalidated = true;
					try {
						waiter.onInvalidated?.();
					} catch {
						// Invalidating the queue must not be prevented by observer diagnostics.
					}
				}
				if (waiter.granted) {
					// The request may be gone while an underlying board mutation is still running.
					// Keep ownership until it explicitly settles or the independent hard lease fires.
					this.cleanupRequestInvalidation(waiter);
					return;
				}
				waiter.settled = true;
				this.removeWaiter(key, lock, waiter);
				this.cleanupWaiter(waiter);
				reject(
					boardEditAbortError(
						reason === "cancelled"
							? `Board edit acquisition for '${key}' was cancelled.`
							: `Board edit acquisition for '${key}' reached its deadline.`,
					),
				);
			};
			waiter.abortHandler = () => invalidate("cancelled");
			options.signal?.addEventListener("abort", waiter.abortHandler, {
				once: true,
			});
			if (Number.isFinite(deadlineAtMs)) {
				waiter.deadlineTimer = setTimeout(
					() => invalidate("deadline"),
					Math.max(0, deadlineAtMs - Date.now()),
				);
			}
			lock.waiters.push(waiter);
			// Close the small race between the initial check and listener registration.
			if (options.signal?.aborted) {
				invalidate("cancelled");
				return;
			}
			this.grantNext(key, lock);
		});
	}

	private cleanupRequestInvalidation(waiter: BoardEditWaiter) {
		if (waiter.deadlineTimer !== undefined) {
			clearTimeout(waiter.deadlineTimer);
			waiter.deadlineTimer = undefined;
		}
		if (waiter.abortHandler) {
			waiter.signal?.removeEventListener("abort", waiter.abortHandler);
			waiter.abortHandler = undefined;
		}
	}

	private cleanupWaiter(waiter: BoardEditWaiter) {
		this.cleanupRequestInvalidation(waiter);
		if (waiter.hardLeaseTimer !== undefined) {
			clearTimeout(waiter.hardLeaseTimer);
			waiter.hardLeaseTimer = undefined;
		}
	}

	private removeWaiter(
		key: string,
		lock: BoardEditLock,
		waiter: BoardEditWaiter,
	) {
		const index = lock.waiters.indexOf(waiter);
		if (index >= 0) lock.waiters.splice(index, 1);
		if (!lock.owner && lock.waiters.length === 0) this.locks.delete(key);
	}

	private grantNext(key: string, lock: BoardEditLock) {
		if (lock.owner) return;
		while (lock.waiters.length > 0) {
			const waiter = lock.waiters.shift();
			if (!waiter || waiter.settled) continue;
			if (waiter.signal?.aborted || waiter.deadlineAtMs <= Date.now()) {
				waiter.settled = true;
				this.cleanupWaiter(waiter);
				waiter.reject(
					boardEditAbortError(`Board edit acquisition for '${key}' expired.`),
				);
				continue;
			}

			lock.owner = waiter.token;
			waiter.granted = true;
			let released = false;
			const release = () => {
				if (released) return;
				released = true;
				waiter.settled = true;
				this.cleanupWaiter(waiter);
				if (lock.owner === waiter.token) lock.owner = undefined;
				if (!lock.owner && lock.waiters.length === 0) {
					this.locks.delete(key);
					return;
				}
				this.grantNext(key, lock);
			};
			waiter.release = release;
			if (Number.isFinite(waiter.hardLeaseMs)) {
				waiter.hardLeaseTimer = setTimeout(() => {
					if (waiter.settled) return;
					if (!waiter.invalidated) {
						waiter.invalidated = true;
						try {
							waiter.onInvalidated?.();
						} catch {
							// A diagnostic callback cannot prevent the hard lease escape hatch.
						}
					}
					release();
				}, waiter.hardLeaseMs);
			}
			waiter.resolve(release);
			return;
		}
		this.locks.delete(key);
	}
}

export const boardEditCoordinator = new BoardEditCoordinator();

export interface BoardEditInterruptionResult {
	status: "error" | "timeout";
	code: string;
	message: string;
	flowscript_status: "retained_for_repair" | "no_recoverable_candidate";
	retained_flowscript?: string;
	retained_status?: string;
	next_action: string;
}

export function boardEditInterruptionResult(options: {
	status: "error" | "timeout";
	code: string;
	message: string;
	candidate?: FlowScriptWorkspaceCandidate;
}): BoardEditInterruptionResult {
	const retained = options.candidate
		? sanitizeFlowScriptForPersistence(options.candidate.source)
		: undefined;
	return {
		status: options.status,
		code: options.code,
		message: options.message,
		flowscript_status: options.candidate
			? "retained_for_repair"
			: "no_recoverable_candidate",
		...(options.candidate
			? {
					...(retained?.safe ? { retained_flowscript: retained.source } : {}),
					retained_status: options.candidate.status ?? "interrupted",
				}
			: {}),
		next_action: options.candidate
			? retained?.safe
				? "Continue from the board-scoped retained FlowScript in a new serialized flowpilot_board run. Repair it in place; do not restart discovery or replace it with a smoke-test Event."
				: "A candidate remains available only in the active renderer because its @secret syntax was not safe to persist. Retry this board before closing the app; repair it in place."
			: "Retry flowpilot_board once from the fresh persisted board snapshot.",
	};
}

export function normalizeFlowScriptSnapshot(source: string) {
	return source
		.replace(/\r\n/g, "\n")
		.split("\n")
		.map((line) => line.replace(/\s+$/g, ""))
		.join("\n")
		.trim();
}

export function flowScriptSnapshotChanged(before: string, current: string) {
	return (
		normalizeFlowScriptSnapshot(before) !== normalizeFlowScriptSnapshot(current)
	);
}

export interface FlowScriptReadbackAssessment {
	ok: boolean;
	code?: "not_persisted" | "completeness_regression" | "scope_missing";
	message?: string;
}

/** Verify persisted board state, not merely the reconcile command count returned by apply. */
export function assessFlowScriptReadback(options: {
	before: string;
	expected: string;
	actual: string;
}): FlowScriptReadbackAssessment {
	if (!flowScriptSnapshotChanged(options.before, options.actual)) {
		return {
			ok: false,
			code: "not_persisted",
			message:
				"FlowScript apply returned commands, but the persisted board FlowScript did not change.",
		};
	}

	const expected = profileFlowScriptCandidate(options.expected);
	const actual = profileFlowScriptCandidate(options.actual);
	const regression = detectFlowScriptCandidateRegression(expected, actual);
	if (regression) {
		return {
			ok: false,
			code: "completeness_regression",
			message: `Persisted FlowScript collapsed from ${regression.previous_call_sites} expected call sites to ${regression.candidate_call_sites}.`,
		};
	}

	const expectedSymbols = [
		...expected.helperFunctions.map((name) => `function:${name}`),
		...expected.topLevelVariables.map((name) => `variable:${name}`),
		...expected.interfaces.map((name) => `interface:${name}`),
	];
	const actualSymbols = new Set([
		...actual.helperFunctions.map((name) => `function:${name}`),
		...actual.topLevelVariables.map((name) => `variable:${name}`),
		...actual.interfaces.map((name) => `interface:${name}`),
	]);
	const missingSymbols = expectedSymbols.filter(
		(symbol) => !actualSymbols.has(symbol),
	);
	if (
		missingSymbols.length > 0 ||
		actual.eventEntries < expected.eventEntries ||
		actual.callSites < expected.callSites
	) {
		return {
			ok: false,
			code: "scope_missing",
			message: `Persisted FlowScript is missing expected workflow scope${missingSymbols.length > 0 ? ` (${missingSymbols.slice(0, 5).join(", ")})` : ""}.`,
		};
	}

	return { ok: true };
}

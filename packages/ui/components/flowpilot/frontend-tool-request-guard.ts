export type FrontendToolRequestInvalidationReason =
	| "cancelled"
	| "deadline"
	| "unmounted";

export interface FrontendToolRequestLease {
	readonly requestId: string;
	readonly generation: symbol;
	readonly signal: AbortSignal;
	isActive(): boolean;
	invalidationReason(): FrontendToolRequestInvalidationReason | undefined;
	assertActive(stage: string): void;
	settle(): void;
}

interface GuardEntry {
	requestId: string;
	generation: symbol;
	controller: AbortController;
	deadlineAtMs?: number;
	deadlineTimer?: ReturnType<typeof setTimeout>;
	reason?: FrontendToolRequestInvalidationReason;
	onInvalidated?: (reason: FrontendToolRequestInvalidationReason) => void;
}

export interface BeginFrontendToolRequestOptions {
	requestId: string;
	deadlineAtMs?: number;
	onInvalidated?: (reason: FrontendToolRequestInvalidationReason) => void;
}

function inactiveRequestError(requestId: string, stage: string) {
	const error = new Error(
		`Frontend tool request '${requestId}' was cancelled before ${stage}; late side effects were blocked.`,
	);
	error.name = "AbortError";
	return error;
}

/**
 * Tracks immutable request generations for the embedded FlowPilot frontend-tool bridge.
 *
 * Cancellation aborts a generation but deliberately leaves it registered until its handler
 * settles. A duplicate event therefore cannot replace the cancelled controller and revive a late
 * approval or mutation under the same request id.
 */
export class FrontendToolRequestGuard {
	private readonly entries = new Map<string, GuardEntry>();

	begin(
		options: BeginFrontendToolRequestOptions,
	): FrontendToolRequestLease | undefined {
		if (this.entries.has(options.requestId)) return undefined;

		const entry: GuardEntry = {
			requestId: options.requestId,
			generation: Symbol(options.requestId),
			controller: new AbortController(),
			deadlineAtMs: Number.isFinite(options.deadlineAtMs)
				? options.deadlineAtMs
				: undefined,
			onInvalidated: options.onInvalidated,
		};
		this.entries.set(options.requestId, entry);

		if (entry.deadlineAtMs !== undefined) {
			const remaining = entry.deadlineAtMs - Date.now();
			if (remaining <= 0) {
				this.invalidate(entry, "deadline");
			} else {
				entry.deadlineTimer = setTimeout(
					() => this.invalidate(entry, "deadline"),
					remaining,
				);
			}
		}

		const isActive = () => {
			if (
				entry.deadlineAtMs !== undefined &&
				Date.now() >= entry.deadlineAtMs
			) {
				this.invalidate(entry, "deadline");
			}
			return (
				this.entries.get(entry.requestId) === entry &&
				!entry.controller.signal.aborted
			);
		};

		return {
			requestId: entry.requestId,
			generation: entry.generation,
			signal: entry.controller.signal,
			isActive,
			invalidationReason: () => entry.reason,
			assertActive: (stage) => {
				if (!isActive()) throw inactiveRequestError(entry.requestId, stage);
			},
			settle: () => this.settle(entry),
		};
	}

	cancel(
		requestId: string,
		reason: FrontendToolRequestInvalidationReason = "cancelled",
	): boolean {
		const entry = this.entries.get(requestId);
		if (!entry) return false;
		this.invalidate(entry, reason);
		return true;
	}

	cancelAll(reason: FrontendToolRequestInvalidationReason = "unmounted") {
		for (const entry of this.entries.values()) this.invalidate(entry, reason);
	}

	private invalidate(
		entry: GuardEntry,
		reason: FrontendToolRequestInvalidationReason,
	) {
		if (this.entries.get(entry.requestId) !== entry || entry.reason) return;
		entry.reason = reason;
		if (entry.deadlineTimer !== undefined) {
			clearTimeout(entry.deadlineTimer);
			entry.deadlineTimer = undefined;
		}
		entry.controller.abort(reason);
		try {
			entry.onInvalidated?.(reason);
		} catch {
			// An observer cannot revive or prevent cancellation of the request generation.
		}
	}

	private settle(entry: GuardEntry) {
		if (this.entries.get(entry.requestId) !== entry) return;
		if (entry.deadlineTimer !== undefined) clearTimeout(entry.deadlineTimer);
		this.entries.delete(entry.requestId);
	}
}

/**
 * Deadlines for desktop HTTP calls.
 *
 * Desktop requests go through `@tauri-apps/plugin-http`, whose Rust side honours
 * only `connectTimeout` — `request.send()` on a half-open socket waits forever.
 * The resulting `invoke` promise then never *settles*: it neither resolves nor
 * rejects, so `finally` blocks never run and every JS guard flag awaiting it
 * latches for the lifetime of the process.
 *
 * `withRequestDeadline` closes that hole from both sides. It aborts the request
 * (which reaches Rust as `plugin:http|fetch_cancel` and frees the connection),
 * and independently races the caller's promise against the deadline so the await
 * settles even if the cancel path is itself stuck.
 */

/** Interactive API calls: generous, but always bounded. */
export const DEFAULT_REQUEST_TIMEOUT_MS = 30_000;
/** Time allowed to establish the TCP/TLS connection. */
export const DEFAULT_CONNECT_TIMEOUT_MS = 10_000;
/** Streams are unbounded once open; only reaching the first byte is bounded. */
export const STREAM_HEADER_TIMEOUT_MS = 30_000;

export class RequestTimeoutError extends Error {
	readonly timeoutMs: number;
	readonly target: string;

	constructor(target: string, timeoutMs: number) {
		super(`Request timed out after ${timeoutMs}ms: ${target}`);
		this.name = "RequestTimeoutError";
		this.timeoutMs = timeoutMs;
		this.target = target;
	}
}

export interface RequestDeadline {
	/** Pass to `tauriFetch` so a timeout cancels the request on the Rust side. */
	readonly signal: AbortSignal;
	/**
	 * Disarm the deadline. Call this once response headers have arrived on a
	 * streaming request, so a long-lived stream is not killed mid-flight.
	 */
	release(): void;
}

interface RequestDeadlineOptions {
	readonly timeoutMs?: number;
	readonly signal?: AbortSignal | null;
	/**
	 * Controller aborted when the deadline expires. Pass one when the caller must
	 * keep cancelling the request after the deadline is released — a stream, whose
	 * reader terminates it later. Otherwise the deadline owns its own.
	 */
	readonly controller?: AbortController;
}

/**
 * Runs `send` under a deadline. `target` only labels the timeout error.
 */
export async function withRequestDeadline<T>(
	target: string,
	send: (deadline: RequestDeadline) => Promise<T>,
	options?: RequestDeadlineOptions,
): Promise<T> {
	const timeoutMs = options?.timeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS;
	const controller = options?.controller ?? new AbortController();
	const upstream = options?.signal;

	let timer: ReturnType<typeof setTimeout> | undefined;
	let expire: (() => void) | undefined;
	const release = () => {
		if (timer !== undefined) {
			clearTimeout(timer);
			timer = undefined;
		}
		expire = undefined;
	};

	const forwardAbort = () => controller.abort();
	upstream?.addEventListener("abort", forwardAbort);
	if (upstream?.aborted) controller.abort();

	// The race is the guarantee: the caller's await settles at the deadline even
	// if aborting the underlying request does not unblock it.
	const expired = new Promise<never>((_, reject) => {
		expire = () => reject(new RequestTimeoutError(target, timeoutMs));
		timer = setTimeout(() => {
			controller.abort();
			expire?.();
		}, timeoutMs);
	});

	const sent = send({ signal: controller.signal, release });
	// The loser of the race must not surface as an unhandled rejection.
	sent.catch(() => {});

	try {
		return await Promise.race([sent, expired]);
	} finally {
		release();
		upstream?.removeEventListener("abort", forwardAbort);
	}
}

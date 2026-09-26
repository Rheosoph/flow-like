/**
 * Deadlines for API calls.
 *
 * A request on a half-open socket can wait forever. The awaiting promise then
 * never *settles*: it neither resolves nor rejects, so `finally` blocks never run
 * and every guard flag awaiting it latches for the lifetime of the page.
 *
 * `withRequestDeadline` closes that hole from both sides. It aborts the request,
 * and independently races the caller's promise against the deadline so the await
 * settles even if the abort path is itself stuck.
 */

/** Interactive API calls: generous, but always bounded. */
export const DEFAULT_REQUEST_TIMEOUT_MS = 30_000;
/** Time allowed to establish the TCP/TLS connection. */
export const DEFAULT_CONNECT_TIMEOUT_MS = 10_000;
/** Streams are unbounded once open; only reaching the first byte is bounded. */
export const STREAM_HEADER_TIMEOUT_MS = 30_000;
/**
 * How long a read waits for the hub when the device already holds a copy it can
 * use instead. A hub that accepts connections but never answers otherwise holds
 * opening and running a project for a full route deadline.
 */
export const HUB_REFRESH_TIMEOUT_MS = 15_000;

/**
 * JSON routes mirror the API's deadline classes (packages/api/src/middleware/
 * deadline.rs) with headroom, so the server's own 504 always arrives before the
 * client gives up: reads (10 s) and writes (30 s) share one bound, data routes
 * have 120 s, maintenance jobs 300 s, and dispatch routes — executor setup or a
 * tool result collected synchronously — the 870 s Lambda cap.
 */
export const WRITE_REQUEST_TIMEOUT_MS = 45_000;
export const DATA_REQUEST_TIMEOUT_MS = 135_000;
export const JOB_REQUEST_TIMEOUT_MS = 315_000;
export const DISPATCH_REQUEST_TIMEOUT_MS = 900_000;
/** The body is sent inside the deadline; a slow uplink earns a second per full unit. */
export const UPLOAD_FLOOR_BYTES_PER_SECOND = 64 * 1024;

const DATA_APP_SECTIONS = new Set([
	"board",
	"db",
	"graph",
	"analytics",
	"sales",
	"fork",
]);
const DISPATCH_EVENT_ACTIONS = new Set([
	"setup",
	"restore",
	"invoke",
	"mcp-operation",
	"mcp",
	"rest",
]);

function routeTimeoutMs(parts: readonly string[], method: string): number {
	const [root, second, section, , action, sub] = parts;
	if (root === "admin" || root === "maintenance") return JOB_REQUEST_TIMEOUT_MS;
	if (root === "r" || root === "m") return DISPATCH_REQUEST_TIMEOUT_MS;
	if (root === "sink" && second === "trigger")
		return DISPATCH_REQUEST_TIMEOUT_MS;
	if (root === "apps") {
		if (second === "fork") return DATA_REQUEST_TIMEOUT_MS;
		if (section === "events" && parts.length >= 4) {
			if (parts.length === 4) {
				return method === "PUT"
					? DISPATCH_REQUEST_TIMEOUT_MS
					: WRITE_REQUEST_TIMEOUT_MS;
			}
			if (action !== undefined && DISPATCH_EVENT_ACTIONS.has(action)) {
				return DISPATCH_REQUEST_TIMEOUT_MS;
			}
			if (action === "canary" && sub === "promote") {
				return DISPATCH_REQUEST_TIMEOUT_MS;
			}
			return WRITE_REQUEST_TIMEOUT_MS;
		}
		if (section === "board" && action === "invoke") {
			return DISPATCH_REQUEST_TIMEOUT_MS;
		}
		if (section === "graph" && action === "actions" && parts[6] === "invoke") {
			return DISPATCH_REQUEST_TIMEOUT_MS;
		}
		if (section !== undefined && DATA_APP_SECTIONS.has(section)) {
			return DATA_REQUEST_TIMEOUT_MS;
		}
		return WRITE_REQUEST_TIMEOUT_MS;
	}
	if (root === "registry" && second === "publish")
		return DATA_REQUEST_TIMEOUT_MS;
	if (root === "courses" && parts[2] === "assets" && parts[4] === "optimize") {
		return DATA_REQUEST_TIMEOUT_MS;
	}
	if (root === "execution" || root === "oauth") return DATA_REQUEST_TIMEOUT_MS;
	if (root === "audit" && second === "verify") return DATA_REQUEST_TIMEOUT_MS;
	return WRITE_REQUEST_TIMEOUT_MS;
}

/** `path` is the API path without host, `api/v1/` prefix or query string. */
export function requestTimeoutMs(
	path: string,
	method = "GET",
	bodyBytes = 0,
): number {
	const parts = path.split("/").filter(Boolean);
	const upload = Math.floor(bodyBytes / UPLOAD_FLOOR_BYTES_PER_SECOND) * 1000;
	return routeTimeoutMs(parts, method.toUpperCase()) + upload;
}

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
	/** Pass to the request so a timeout cancels it at the transport. */
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

/**
 * The work's result when it settles within `timeoutMs`, otherwise `fallback`.
 * The work is not cancelled: a refresh that lands later still updates the
 * device's copy for the next reader, while this caller proceeds without it.
 */
export function settleWithin<T>(
	work: Promise<T>,
	timeoutMs: number,
	fallback: T,
): Promise<T> {
	return new Promise<T>((resolve, reject) => {
		const timer = setTimeout(() => resolve(fallback), timeoutMs);
		work.then(
			(value) => {
				clearTimeout(timer);
				resolve(value);
			},
			(error: unknown) => {
				clearTimeout(timer);
				reject(error);
			},
		);
	});
}

import {
	REALTIME_ERROR_UNAVAILABLE,
	type RealtimeClient,
	type RealtimeClientOptions,
	type RealtimeConnectionSession,
	type RealtimeRunEvent,
	type RealtimeRunSnapshot,
	isTerminalRunStatus,
} from "../realtime/types";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import { ExecutionDetachedError, executionError } from "./types";

/** Replayed events enter the same execution effects and interaction claims as streamed events. */
export function realtimeExecutionEvent(
	runId: string,
	event: RealtimeRunEvent & { event_id?: string },
): IIntercomEvent {
	const millis = Date.parse(event.created_at);
	const timestamp = Number.isFinite(millis) ? millis : 0;
	return {
		event_id: event.event_id || `${runId}:${event.sequence}`,
		event_type: event.event_type,
		payload: event.payload,
		timestamp: {
			secs_since_epoch: Math.floor(timestamp / 1_000),
			nanos_since_epoch: (timestamp % 1_000) * 1_000_000,
		},
	};
}

export type ExecutionRealtimeClientFactory = (
	session: RealtimeConnectionSession,
	options?: RealtimeClientOptions,
) => Promise<RealtimeClient>;

export interface ExecutionReceipt {
	run_id: string;
	poll_token?: string;
}

export async function readExecutionReceipt(
	response: Response,
): Promise<ExecutionReceipt> {
	const reader = response.body?.getReader();
	if (!reader)
		throw new Error("The server did not return an execution receipt");
	const decoder = new TextDecoder();
	let body = "";
	let bytes = 0;
	try {
		for (;;) {
			const { value, done } = await reader.read();
			if (done) break;
			bytes += value.byteLength;
			if (bytes > 64 * 1024) throw new Error("Execution receipt is too large");
			body += decoder.decode(value, { stream: true });
		}
		body += decoder.decode();
	} finally {
		await reader.cancel().catch(() => undefined);
		reader.releaseLock();
	}
	const receipt: unknown = JSON.parse(body);
	const validText = (value: unknown, bytes: number): value is string =>
		typeof value === "string" &&
		value.length > 0 &&
		new TextEncoder().encode(value).byteLength <= bytes &&
		!/\p{Cc}/u.test(value);
	if (
		!receipt ||
		typeof receipt !== "object" ||
		!("run_id" in receipt) ||
		!validText(receipt.run_id, 256) ||
		("poll_token" in receipt &&
			receipt.poll_token != null &&
			!validText(receipt.poll_token, 8192))
	)
		throw new Error("The server returned an invalid execution receipt");
	return {
		run_id: receipt.run_id,
		...("poll_token" in receipt && typeof receipt.poll_token === "string"
			? { poll_token: receipt.poll_token }
			: {}),
	};
}

/**
 * The account bus owns networking; this port owns only one run's replay cursor. Retrying states
 * and temporarily unavailable replays keep waiting. Losing the bus detaches from the run, which
 * continues on the server; only a run the API cannot find fails here.
 */
export async function followRealtimeExecution(options: {
	createClient: ExecutionRealtimeClientFactory;
	session: RealtimeConnectionSession;
	receipt: ExecutionReceipt;
	signal: AbortSignal;
	onUnauthorized: NonNullable<RealtimeClientOptions["onUnauthorized"]>;
	onRun: (run: RealtimeRunSnapshot) => void | Promise<void>;
}): Promise<RealtimeRunSnapshot> {
	const runId = options.receipt.run_id;
	let client: RealtimeClient;
	try {
		client = await options.createClient(options.session, {
			onUnauthorized: options.onUnauthorized,
		});
	} catch (error) {
		throw new ExecutionDetachedError(
			runId,
			`Live updates for run ${runId} could not start: ${executionError(error).message}`,
		);
	}
	let unsubscribe: (() => void) | undefined;
	let abort: (() => void) | undefined;
	try {
		return await new Promise<RealtimeRunSnapshot>((resolve, reject) => {
			abort = () =>
				reject(options.signal.reason ?? new ExecutionDetachedError(runId));
			if (options.signal.aborted) return abort();
			options.signal.addEventListener("abort", abort, { once: true });
			unsubscribe = client.subscribe(async (update) => {
				if (options.signal.aborted || update.type === "provider") return;
				if (update.type === "state") {
					if (update.state === "closed")
						reject(
							new ExecutionDetachedError(
								runId,
								`Live updates for run ${runId} closed before it finished.`,
							),
						);
					return;
				}
				const error = update.snapshot.errors.find(
					(item) => item.run_id === runId,
				);
				if (error && error.code !== REALTIME_ERROR_UNAVAILABLE) {
					reject(
						new Error(
							`Run ${runId} cannot be followed: the server reported ${error.code}.`,
						),
					);
					return;
				}
				const run = update.snapshot.runs.find((item) => item.run_id === runId);
				if (!run) return;
				await options.onRun(run);
				if (!run.has_more && isTerminalRunStatus(run.status)) resolve(run);
			});
			client.setSubscriptions({
				notifications: false,
				runs: [{ ...options.receipt, after_sequence: -1 }],
			});
		});
	} finally {
		if (abort) options.signal.removeEventListener("abort", abort);
		unsubscribe?.();
		client.close();
	}
}

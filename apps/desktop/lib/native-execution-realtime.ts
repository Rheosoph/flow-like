import { realtimeExecutionEvent } from "@flow-like/flow-like-ui/lib/execution/realtime";
import { ExecutionDetachedError } from "@flow-like/flow-like-ui/lib/execution/types";
import type { RealtimeRunSubscription } from "@flow-like/flow-like-ui/lib/realtime/types";
import type { IIntercomEvent } from "@flow-like/flow-like-ui/lib/schema/events/intercom-event";
import { watchNativeRealtime } from "./native-realtime";

interface NativeExecutionSubscription {
	profileId: string;
	subject: string;
	issuer: string;
	run: RealtimeRunSubscription;
	signal: AbortSignal;
	onEvent: (event: IIntercomEvent) => void | Promise<void>;
}

const TERMINAL = new Set([
	"completed",
	"failed",
	"cancelled",
	"canceled",
	"timeout",
	"timed_out",
]);

/** Keep the invoking window attached to a queued run through the shared Rust owner. */
export function followNativeExecution(
	options: NativeExecutionSubscription,
): Promise<void> {
	return new Promise<void>((resolve, reject) => {
		const subscription: { close?: () => void } = {};
		let settled = false;
		let sequence = options.run.after_sequence ?? -1;
		let completed = false;
		const finish = (error?: Error, delivered = false) => {
			if (settled) return;
			settled = true;
			options.signal.removeEventListener("abort", abort);
			// The callback's successful return lets the bridge send its ACK first.
			if (delivered) setTimeout(() => subscription.close?.(), 0);
			else subscription.close?.();
			if (error) reject(error);
			else resolve();
		};
		const abort = () =>
			finish(new DOMException("Execution cancelled", "AbortError"));
		if (options.signal.aborted) {
			abort();
			return;
		}
		options.signal.addEventListener("abort", abort, { once: true });
		subscription.close = watchNativeRealtime({
			profileId: options.profileId,
			subject: options.subject,
			issuer: options.issuer,
			request: { runs: [options.run], timeout_ms: 20_000 },
			onError: (error) => finish(error),
			onMessage: async (message) => {
				if (settled) return;
				if (message.type === "state") {
					// The native owner retries blocked states; only a lost session ends the follow.
					if (message.state === "signed_out" || message.state === "closed")
						finish(new ExecutionDetachedError(options.run.run_id));
					return;
				}
				if (message.type === "reply") return;
				const failure = message.response.errors.find(
					(error) => error.run_id === options.run.run_id,
				);
				if (failure) {
					finish(
						new Error(`Execution updates are unavailable (${failure.code})`),
						true,
					);
					return;
				}
				const update = message.response.runs.find(
					(run) => run.run_id === options.run.run_id,
				);
				if (!update) return;
				for (const event of update.events) {
					if (settled) return;
					if (event.sequence <= sequence) continue;
					await options.onEvent(realtimeExecutionEvent(update.run_id, event));
					sequence = event.sequence;
					completed ||= event.event_type === "completed";
				}
				if (
					settled ||
					update.has_more ||
					!TERMINAL.has(update.status.toLowerCase())
				)
					return;
				if (!completed) {
					await options.onEvent({
						event_id: `${update.run_id}:terminal`,
						event_type: "completed",
						payload: {
							run_id: update.run_id,
							status: update.status,
							error: update.error,
						},
						timestamp: {
							secs_since_epoch: Math.floor(Date.now() / 1000),
							nanos_since_epoch: 0,
						},
					});
					completed = true;
				}
				const status = update.status.toLowerCase();
				const outcomeFailure =
					status === "completed"
						? undefined
						: status === "cancelled" || status === "canceled"
							? new DOMException("Execution cancelled", "AbortError")
							: new Error(
									update.error || `Execution ${status.replaceAll("_", " ")}`,
								);
				finish(outcomeFailure, true);
			},
		});
		if (settled) subscription.close();
	});
}

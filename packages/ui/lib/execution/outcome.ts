import type { IIntercomEvent } from "../schema/events/intercom-event";
import type { ILogMetadata } from "../schema/flow/log-metadata";

function failureText(payload: unknown): string | undefined {
	if (typeof payload === "string") return payload.trim() || undefined;
	if (!payload || typeof payload !== "object") return undefined;
	const record = payload as Record<string, unknown>;
	for (const key of ["error_message", "message", "error"]) {
		const value = record[key];
		if (typeof value === "string" && value.trim()) return value.trim();
	}
	return undefined;
}

/** Transports can resolve even when the workflow failed. Read its terminal outcome as well. */
export function completedRunFailure(
	events: IIntercomEvent[],
	metadata: ILogMetadata | undefined,
): Error | undefined {
	let completed: Record<string, unknown> | undefined;
	let hasError = false;
	let errorMessage: string | undefined;
	for (const event of events) {
		if (event.event_type === "error") {
			hasError = true;
			errorMessage ??= failureText(event.payload);
		} else if (
			event.event_type === "completed" &&
			event.payload &&
			typeof event.payload === "object"
		) {
			completed = event.payload;
		}
	}
	const status =
		typeof completed?.status === "string"
			? completed.status.trim().toLowerCase()
			: undefined;
	// A handled node error can leave error logs on a successful run.
	if (status === "completed" || status === "success" || status === "succeeded")
		return undefined;
	if (status === "cancelled" || status === "canceled") {
		return Object.assign(new Error("Execution cancelled"), {
			name: "AbortError",
		});
	}
	const reason =
		metadata?.error_message?.trim() || failureText(completed) || errorMessage;
	const timedOut = status === "timeout" || status === "timed_out";
	if (
		status === "failed" ||
		timedOut ||
		hasError ||
		reason ||
		(metadata?.log_level ?? 0) >= 3 ||
		(typeof completed?.log_level === "number" && completed.log_level >= 3)
	) {
		return new Error(
			reason ||
				(timedOut
					? "The workflow timed out before it could finish this response."
					: "The workflow failed before it could finish this response. Check the run logs for details."),
		);
	}
	return undefined;
}

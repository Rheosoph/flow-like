import { create } from "zustand";
import type {
	RealtimeProviderFailure,
	RealtimeProviderStatus,
	RealtimeStatus,
	RealtimeUpdate,
} from "./types";

/** How updates reach this tab: pushed over a live provider, polled, or not at all. */
export type RealtimeDelivery = "live" | "polling" | "paused" | "off";

export const useRealtimeStatus = create<{ status: RealtimeStatus | null }>(
	() => ({ status: null }),
);

export function realtimeDelivery(
	status: RealtimeStatus | null,
): RealtimeDelivery {
	if (!status) return "off";
	if (status.provider?.live) return "live";
	if (status.state === "paused") return "paused";
	return status.state === "connected" ||
		status.state === "connecting" ||
		status.state === "reconnecting"
		? "polling"
		: "off";
}

export function describeRealtimeFailure(
	failure: RealtimeProviderFailure,
	transport: RealtimeProviderStatus["transport"],
): string {
	return `${transport ?? "realtime"} connection failed at ${failure.stage} after ${failure.elapsedMs} ms: ${failure.message}`;
}

/** Logs each new provider failure once, and an API that grants no live channel. */
function logProviderChange(
	previous: RealtimeProviderStatus | undefined,
	next: RealtimeProviderStatus,
): void {
	if (next.failure && next.failure.at !== previous?.failure?.at)
		console.warn(
			`[realtime] ${describeRealtimeFailure(next.failure, next.transport)}; updates arrive by polling until it reconnects`,
		);
	if (next.transport === "http" && previous?.transport !== "http")
		console.warn(
			"[realtime] the API granted no live channel; updates arrive by polling",
		);
}

export function recordRealtimeUpdate(
	runner: RealtimeStatus["runner"],
	update: Exclude<RealtimeUpdate, { type: "snapshot" }>,
): void {
	const current = useRealtimeStatus.getState().status;
	const base: RealtimeStatus = current
		? { ...current, runner }
		: { runner, state: "connecting" };
	if (update.type === "state") {
		useRealtimeStatus.setState({ status: { ...base, state: update.state } });
		return;
	}
	logProviderChange(current?.provider, update.status);
	useRealtimeStatus.setState({ status: { ...base, provider: update.status } });
}

export function clearRealtimeStatus(): void {
	useRealtimeStatus.setState({ status: null });
}

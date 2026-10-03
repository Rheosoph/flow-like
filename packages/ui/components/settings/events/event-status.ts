export type EventStatus =
	| "live"
	| "paused"
	| "attention"
	| "unknown"
	| "device";

export function getEventStatus({
	active,
	blocking,
	requiresSink,
	sinkActive,
	deviceOnly = false,
}: {
	active: boolean;
	blocking: boolean;
	requiresSink: boolean;
	sinkActive?: boolean;
	deviceOnly?: boolean;
}): EventStatus {
	if (blocking) return "attention";
	if (!active) return "paused";
	if (deviceOnly) return "device";
	if (!requiresSink) return "live";
	if (sinkActive === undefined) return "unknown";
	return sinkActive ? "live" : "attention";
}

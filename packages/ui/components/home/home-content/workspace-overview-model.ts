import type {
	IExecutionUsageRecord,
	IPaginatedResponse,
} from "../../../lib/schema/usage/tracking";
import {
	hasAttentionSeverity,
	summarizeHomeExecutions,
} from "../home-activity-statistics";

export function workspaceProfileAppCount(
	availableIds: readonly string[] | undefined,
	profileIds: readonly string[] | undefined,
): number | undefined {
	if (availableIds === undefined || profileIds === undefined) return undefined;
	const visible = new Set(profileIds);
	return new Set(availableIds.filter((id) => visible.has(id))).size;
}

export function workspacePulseHistory(
	history: IPaginatedResponse<IExecutionUsageRecord> | undefined,
	days: unknown,
	now = Date.now(),
) {
	if (!history) return null;
	const statistics = summarizeHomeExecutions(history, days, now);
	const attention = statistics.rows.filter((row) =>
		hasAttentionSeverity(row.status),
	);
	return { ...statistics, attention, volume: statistics.rows.length };
}

/** A failed or disabled source must not turn cached history into current metrics. */
export function workspacePulseMetrics(
	history: ReturnType<typeof workspacePulseHistory>,
	enabled: boolean,
	error: boolean,
) {
	return enabled && !error ? history : null;
}

export function workspacePulseState({
	authenticated,
	supported,
	loading,
	error,
	volume,
}: {
	authenticated: boolean;
	supported: boolean;
	loading: boolean;
	error: boolean;
	volume: number | undefined;
}): "starter" | "loading" | "unavailable" | "activity" {
	if (!authenticated || !supported) return "starter";
	if (loading && volume === undefined) return "loading";
	if (error) return "unavailable";
	return volume ? "activity" : "starter";
}

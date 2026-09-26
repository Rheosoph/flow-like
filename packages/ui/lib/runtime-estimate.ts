import type { QuotaOverview, QuotaResource } from "./quota";
import { asArray } from "./response-shape";

export interface RuntimeSample {
	averageRuntimeMs: number;
	runtimeMs: number;
	cloudStarts: number;
}

/** Uses existing daily rollups; today's unsettled cohort is excluded. */
export function getRuntimeSample(
	overview: Pick<QuotaOverview, "usage" | "usageTruncated"> | undefined,
	now = new Date(),
): RuntimeSample | null {
	if (!overview || overview.usageTruncated || !Number.isFinite(now.getTime()))
		return null;
	const today = now.toISOString().slice(0, 10);
	const cutoff = new Date(now);
	cutoff.setUTCDate(cutoff.getUTCDate() - 30);
	const since = cutoff.toISOString().slice(0, 10);
	let runtimeMs = 0;
	let cloudStarts = 0;
	for (const row of asArray(overview.usage)) {
		if (
			row.fundingClass !== "cloud" ||
			!row.appId ||
			!["realtime", "async"].includes(row.executionMode ?? "") ||
			!/^\d{4}-\d{2}-\d{2}$/.test(row.day) ||
			row.day < since ||
			row.day >= today
		)
			continue;
		const day = new Date(`${row.day}T00:00:00Z`);
		if (
			!Number.isFinite(day.getTime()) ||
			day.toISOString().slice(0, 10) !== row.day
		)
			return null;
		if (
			!Number.isFinite(row.runtimeMs) ||
			row.runtimeMs < 0 ||
			!Number.isFinite(row.cloudStarts) ||
			row.cloudStarts < 0
		)
			return null;
		// Audited corrections can adjust runtime without adding another start.
		runtimeMs += row.runtimeMs;
		cloudStarts += row.cloudStarts;
	}
	if (
		cloudStarts < 10 ||
		runtimeMs <= 0 ||
		!Number.isFinite(runtimeMs) ||
		!Number.isFinite(cloudStarts)
	)
		return null;
	return { averageRuntimeMs: runtimeMs / cloudStarts, runtimeMs, cloudStarts };
}

export interface CloudRunEstimate {
	/** Null means both supplied allowances are explicitly unlimited. */
	runs: number | null;
	runtimeRuns: number | null;
	startLimited: boolean;
}

export function estimateCloudRuns(
	runtimeMs: number | undefined,
	cloudStarts: number | undefined,
	averageRuntimeMs: number,
): CloudRunEstimate | null {
	if (
		runtimeMs === undefined ||
		cloudStarts === undefined ||
		!Number.isFinite(runtimeMs) ||
		!Number.isFinite(cloudStarts) ||
		!Number.isFinite(averageRuntimeMs) ||
		averageRuntimeMs <= 0
	)
		return null;
	const runtimeRuns =
		runtimeMs < 0 ? Infinity : Math.floor(runtimeMs / averageRuntimeMs);
	// An overflowing calculation is unknown, never an unlimited allowance.
	if (runtimeMs >= 0 && !Number.isFinite(runtimeRuns)) return null;
	const starts = cloudStarts < 0 ? Infinity : Math.floor(cloudStarts);
	const runs = Math.min(runtimeRuns, starts);
	return {
		runs: Number.isFinite(runs) ? runs : null,
		runtimeRuns: Number.isFinite(runtimeRuns) ? runtimeRuns : null,
		startLimited: starts < runtimeRuns,
	};
}

export function availableRuntimeCapacity(
	resource: QuotaResource | undefined,
): number | undefined {
	if (
		!resource ||
		!Number.isFinite(resource.limit) ||
		!Number.isFinite(resource.used) ||
		!Number.isFinite(resource.reserved) ||
		resource.used < 0 ||
		resource.reserved < 0
	)
		return undefined;
	return resource.limit < 0
		? -1
		: Math.max(0, resource.limit - resource.used - resource.reserved);
}

export function formatRuntimeSeconds(runtimeMs: number): string {
	return (runtimeMs / 1000).toLocaleString(undefined, {
		maximumSignificantDigits: 3,
	});
}

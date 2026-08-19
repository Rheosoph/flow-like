"use client";

import { useTranslation } from "@flow-like/locales";
import { Layers } from "lucide-react";
import { Badge } from "../../../ui";

/**
 * Windows up to 48 hours are answered from raw rows; anything longer is served
 * from the daily rollups, which have no sub-day detail and carry percentiles
 * that were computed per day. The flag is read defensively so a response that
 * does not carry it yet renders exactly as before.
 */
export type ITelemetryGranularity = "raw" | "daily";

const GRANULARITY_KEYS = [
	"granularity",
	"dataGranularity",
	"data_granularity",
] as const;

export const DAILY_GRANULARITY_HINT =
	"Served from daily rollups: no sub-day detail, and percentiles are aggregated per day.";
export const DAILY_PERCENTILE_HINT =
	"Approximate — aggregated from daily percentiles.";
export const UNAVAILABLE_PERCENTILE_HINT =
	"Not available for a window served from daily rollups.";

export function readTelemetryGranularity(
	response: unknown,
): ITelemetryGranularity | undefined {
	if (typeof response !== "object" || response === null) return undefined;
	const record = response as Record<string, unknown>;
	for (const key of GRANULARITY_KEYS) {
		const value = record[key];
		if (value === "daily" || value === "raw") return value;
	}
	return undefined;
}

export function isDailyGranularity(response: unknown): boolean {
	return readTelemetryGranularity(response) === "daily";
}

/** Renders nothing unless the response was answered from daily rollups. */
export function TelemetryGranularityNotice({
	response,
	className,
}: {
	readonly response: unknown;
	readonly className?: string;
}) {
	const { t } = useTranslation("admin");
	if (!isDailyGranularity(response)) return null;
	return (
		<Badge
			variant="outline"
			className={`gap-1 text-[10px] font-normal text-muted-foreground ${className ?? ""}`}
			title={DAILY_GRANULARITY_HINT}
		>
			<Layers className="h-3 w-3" />
			{t("dailyAggregates", "Daily aggregates")}
		</Badge>
	);
}

/** Prefixes a value that was derived from daily percentiles. */
export function approximateValue(value: string, daily: boolean): string {
	return daily ? `≈ ${value}` : value;
}

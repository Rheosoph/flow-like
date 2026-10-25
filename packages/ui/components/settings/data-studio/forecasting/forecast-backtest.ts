import type { StudioHistoryPoint } from "../../../../state/backend-state/query-state";

/** Offset counts steps after the evaluation window, separately for each series. */
export function forecastBacktestRange(
	history: StudioHistoryPoint[],
	target: string,
	seriesId: string,
	horizon: number,
	offset: number,
) {
	const series = new Map<string, number[]>();
	for (const point of history) {
		if (point.target !== target) continue;
		const times = series.get(point.series_id) ?? [];
		times.push(point.timestamp_ms);
		series.set(point.series_id, times);
	}
	const times = [...(series.get(seriesId) ?? [])].sort((a, b) => a - b);
	const minimumLength = series.size
		? Math.min(...[...series.values()].map((values) => values.length))
		: 0;
	const validHorizon = Number.isSafeInteger(horizon) && horizon > 0;
	const maxOffset = validHorizon ? minimumLength - horizon - 2 : -1;
	const valid =
		validHorizon &&
		times.length > 0 &&
		Number.isSafeInteger(offset) &&
		offset >= 0 &&
		offset <= maxOffset;
	const train = times.length - horizon - offset;
	return {
		totalCount: times.length,
		contextCount: valid ? train : 0,
		maxHorizon: Math.max(0, minimumLength - 2),
		maxOffset: Math.max(0, maxOffset),
		available: maxOffset >= 0 && times.length > 0,
		valid,
		seriesCount: series.size,
		cutoff: valid ? times[train - 1] : undefined,
		start: valid ? times[train] : undefined,
		end: valid ? times[train + horizon - 1] : undefined,
		oldest:
			maxOffset >= 0
				? times[times.length - horizon - maxOffset - 1]
				: undefined,
		latest: maxOffset >= 0 ? times[times.length - horizon - 1] : undefined,
	};
}

import {
	type ModelStats,
	type ModelStatsInput,
	STATS_STEPS,
} from "../../../../../lib/device-management/models";
import type { Consumer, ConsumerContext } from "../models-view";
import { consumersOf } from "../models-view";

/*
 * What the statistics view draws from one `stats` answer (plan §3.6, §3.7).
 * Every range reads hourly rows (the device keeps them 90 days). Counts are
 * summed into columns anchored at the window's end; timings stay hourly,
 * because percentiles of different hours can't be added up.
 */

const HOUR = STATS_STEPS.hour;

export const STATS_RANGES = ["day", "week", "quarter"] as const;
export type StatsRange = (typeof STATS_RANGES)[number];

/** Hours a range covers and the hours one column sums; the last day refreshes every minute. */
export const RANGE_SPEC: Record<
	StatsRange,
	{ hours: number; bucketHours: number; pollS?: number }
> = {
	day: { hours: 24, bucketHours: 1, pollS: 60 },
	week: { hours: 7 * 24, bucketHours: 6 },
	quarter: { hours: 90 * 24, bucketHours: 24 },
};

/** The hourly request of a range: whole hours up to the end of the current one. */
export function rangeWindow(
	range: StatsRange,
	nowS: number,
	modelId?: string,
): ModelStatsInput {
	const to = (Math.floor(nowS / HOUR) + 1) * HOUR;
	return {
		...(modelId ? { modelId } : {}),
		from: to - RANGE_SPEC[range].hours * HOUR,
		to,
		step: "hour",
	};
}

/** Hours per column for an answer of `hours` points. */
export function bucketHoursOf(hours: number): number {
	if (hours <= RANGE_SPEC.day.hours * 2) return RANGE_SPEC.day.bucketHours;
	if (hours <= RANGE_SPEC.week.hours * 2) return RANGE_SPEC.week.bucketHours;
	return RANGE_SPEC.quarter.bucketHours;
}

const sum = (values: readonly number[]) => {
	let total = 0;
	for (const value of values) total += value;
	return total;
};

/** Consecutive groups of `size` values, summed; the last group may be shorter. */
export function bucketed(values: readonly number[], size: number): number[] {
	const buckets: number[] = [];
	for (let start = 0; start < values.length; start += size)
		buckets.push(sum(values.slice(start, start + size)));
	return buckets;
}

const highest = (values: readonly (number | null)[]) => {
	let high: number | undefined;
	for (const value of values)
		if (value !== null) high = Math.max(high ?? value, value);
	return high;
};

/** Generated tokens per second of decode time, per hour; `null` where nothing was generated. */
export function speedSeries(series: ModelStats["series"]): (number | null)[] {
	return series.decode_ms.map((decodeMs, hour) => {
		const tokens = series.completion_tokens[hour] ?? 0;
		return decodeMs > 0 && tokens > 0 ? (tokens * 1000) / decodeMs : null;
	});
}

export interface StatsTotals {
	requests: number;
	errors: number;
	tokensIn: number;
	tokensOut: number;
	/** Over the whole range: generated tokens per second of decode time. */
	tokensPerSecond?: number;
	/** The slowest hour's 95th percentile. */
	ttftP95Max?: number;
	queueP95Max?: number;
}

export interface StatsView {
	/** Unix seconds of the first hour. */
	from: number;
	/** Unix seconds the window ends; past now while its last hour still runs. */
	to: number;
	hours: number;
	columns: {
		step: number;
		tokensIn: number[];
		tokensOut: number[];
		answered: number[];
		failed: number[];
	};
	hourly: {
		ttftP50: (number | null)[];
		ttftP95: (number | null)[];
		queueP95: (number | null)[];
		tokensPerSecond: (number | null)[];
	};
	totals: StatsTotals;
}

function totalsOf(series: ModelStats["series"]): StatsTotals {
	const generated = sum(series.completion_tokens);
	const decodeMs = sum(series.decode_ms);
	const ttftP95Max = highest(series.ttft_p95_ms);
	const queueP95Max = highest(series.queue_wait_p95_ms);
	return {
		requests: sum(series.requests),
		errors: sum(series.errors),
		tokensIn: sum(series.prompt_tokens),
		tokensOut: generated,
		...(decodeMs > 0 && generated > 0
			? { tokensPerSecond: (generated * 1000) / decodeMs }
			: {}),
		...(ttftP95Max === undefined ? {} : { ttftP95Max }),
		...(queueP95Max === undefined ? {} : { queueP95Max }),
	};
}

/** Columns and hourly lines of one answer. */
export function statsView(stats: ModelStats): StatsView {
	const { series } = stats;
	const hours = series.requests.length;
	const size = bucketHoursOf(hours);
	const answered = series.requests.map((requests, hour) =>
		Math.max(0, requests - (series.errors[hour] ?? 0)),
	);
	return {
		from: stats.from,
		to: stats.from + hours * HOUR,
		hours,
		columns: {
			step: size * HOUR,
			tokensIn: bucketed(series.prompt_tokens, size),
			tokensOut: bucketed(series.completion_tokens, size),
			answered: bucketed(answered, size),
			failed: bucketed(series.errors, size),
		},
		hourly: {
			ttftP50: [...series.ttft_p50_ms],
			ttftP95: [...series.ttft_p95_ms],
			queueP95: [...series.queue_wait_p95_ms],
			tokensPerSecond: speedSeries(series),
		},
		totals: totalsOf(series),
	};
}

/** Whether a line has any reported hour. */
export const reported = (values: readonly (number | null)[]) =>
	values.some((value) => value !== null);

/** Whether any request reached the scope in the range. */
export const anyRequests = (stats: ModelStats) =>
	stats.series.requests.some((requests) => requests > 0);

export interface ConsumerRow {
	consumer: Consumer;
	requests: number;
	errors: number;
	tokensIn: number;
	tokensOut: number;
}

/** Callers of the range, most tokens first (then most requests). */
export function consumerRows(
	consumers: ModelStats["consumers"],
	context: ConsumerContext,
): ConsumerRow[] {
	const named = consumersOf(consumers, context);
	return consumers
		.map((row, index) => ({
			consumer: named[index] as Consumer,
			requests: row.requests,
			errors: row.errors,
			tokensIn: row.prompt_tokens,
			tokensOut: row.completion_tokens,
		}))
		.sort(
			(a, b) =>
				b.tokensIn + b.tokensOut - (a.tokensIn + a.tokensOut) ||
				b.requests - a.requests,
		);
}

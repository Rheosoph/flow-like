import type {
	QueryColumn,
	StudioForecastResult,
} from "../../../../state/backend-state/query-state";
import {
	inspectForecastTimestamps,
	numericTextPattern,
} from "./forecast-timestamps";
export { parseForecastTimestamp as timestampValue } from "./forecast-timestamps";

export interface ForecastPlotPoint {
	timestamp: number;
	history: number | null;
	actual: number | null;
	forecast: number | null;
	interval: [number, number] | null;
}

export interface ForecastObservation {
	timestamp: number;
	value: number | null;
}

export interface ForecastPrediction {
	timestamp: number;
	point: number;
	quantiles: number[];
}

/** Quote the entire table name, including dots, as one DataFusion identifier. */
export function tableForecastSql(table: string): string {
	return table ? `SELECT * FROM "${table.replaceAll('"', '""')}"` : "";
}

export function forecastingColumns(
	columns: QueryColumn[],
	rows: Record<string, unknown>[] = [],
) {
	const numericText = columns.filter((column) => {
		if (!/^(utf8|largeutf8|utf8view|string)$/i.test(column.type_name))
			return false;
		const values = rows
			.map((row) => row[column.name])
			.filter((value) => value !== null && value !== undefined);
		return (
			values.length > 0 &&
			values.every((value) => numericInputValue(value) !== null)
		);
	});
	const numeric = columns.filter(
		(column) =>
			/^(u?int(8|16|32|64)|float(16|32|64))$/i.test(column.type_name) ||
			numericText.includes(column),
	);
	const native = columns.filter((column) =>
		/^(timestamp|date32|date64)/i.test(column.type_name),
	);
	const temporal = columns.filter(
		(column) =>
			native.includes(column) ||
			/^(u?int(8|16|32|64)|float(16|32|64)|utf8|largeutf8|utf8view|string)$/i.test(
				column.type_name,
			),
	);
	const named = temporal.find((column) =>
		/time|date|^ts$|^ds$/i.test(column.name),
	);
	const detected = temporal.find((column) => {
		if (!/utf8|string/i.test(column.type_name) || numericText.includes(column))
			return false;
		const values = rows.map((row) => row[column.name]);
		return (
			values.length > 0 &&
			!inspectForecastTimestamps(values, column.type_name, "auto").error
		);
	});
	const timestamp = native[0]?.name ?? named?.name ?? detected?.name ?? "";
	return {
		numeric,
		numericText,
		temporal,
		timestamp,
		target:
			numeric.find(
				(column) =>
					column.name !== timestamp && !/(^id$|_id$)/i.test(column.name),
			)?.name ?? "",
	};
}

/** Forecast-only conversion; blank, locale-formatted and non-finite text stays invalid. */
export function numericInputValue(value: unknown): number | null {
	if (typeof value === "number")
		return Number.isFinite(value) && Number.isFinite(Math.fround(value))
			? value
			: null;
	if (typeof value !== "string" || !numericTextPattern.test(value.trim()))
		return null;
	const number = Number(value.trim());
	return Number.isFinite(number) && Number.isFinite(Math.fround(number))
		? number
		: null;
}

export function finiteValue(value: unknown): number | null {
	return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function forecastCovariateSupport(
	modelId: string,
	result?: {
		model: Pick<StudioForecastResult["model"], "bit_id">;
		capabilities: Pick<
			StudioForecastResult["capabilities"],
			"past_covariates" | "known_covariates"
		>;
	},
	modelType?: unknown,
): { past: boolean; known: boolean; verified: boolean } {
	if (result?.model.bit_id !== modelId)
		return modelType === "timesfm2_5" || modelType === "tiny_time_mixer"
			? { past: false, known: false, verified: false }
			: { past: true, known: true, verified: false };
	return {
		past: result.capabilities.past_covariates,
		known: result.capabilities.known_covariates,
		verified: true,
	};
}

export function forecastOutputLabel(
	quantileLevels: readonly number[] | undefined,
	pointOnly = false,
): string {
	if (pointOnly) return "Point forecast only";
	return quantileLevels?.length
		? quantileLevels.map((value) => `P${Math.round(value * 100)}`).join(", ")
		: "Model defaults";
}

export function visibleCovariatePoints<T extends { timestamp: number }>(
	points: readonly T[],
	role: "past" | "known",
	backtest: boolean,
	cutoff?: number,
): T[] {
	return backtest && role === "past" && cutoff !== undefined
		? points.filter((point) => point.timestamp <= cutoff)
		: [...points];
}

export function buildForecastPlot(
	history: readonly ForecastObservation[],
	predictions: readonly ForecastPrediction[],
	cutoff: number,
	quantileLevels: readonly number[],
): ForecastPlotPoint[] {
	const points = new Map<number, ForecastPlotPoint>();
	const at = (timestamp: number): ForecastPlotPoint => {
		let point = points.get(timestamp);
		if (!point) {
			point = {
				timestamp,
				history: null,
				actual: null,
				forecast: null,
				interval: null,
			};
			points.set(timestamp, point);
		}
		return point;
	};
	for (const sample of history) {
		if (!Number.isFinite(sample.timestamp)) continue;
		const point = at(sample.timestamp);
		if (sample.timestamp <= cutoff) point.history = finiteValue(sample.value);
		else point.actual = finiteValue(sample.value);
	}
	const sortedQuantiles = quantileLevels
		.map((level, index) => ({ level, index }))
		.sort((a, b) => a.level - b.level);
	const lower = sortedQuantiles[0]?.index;
	const upper = sortedQuantiles.at(-1)?.index;
	for (const prediction of predictions) {
		if (
			!Number.isFinite(prediction.timestamp) ||
			prediction.timestamp <= cutoff
		)
			continue;
		const point = at(prediction.timestamp);
		point.forecast = finiteValue(prediction.point);
		const low =
			lower === undefined ? null : finiteValue(prediction.quantiles[lower]);
		const high =
			upper === undefined ? null : finiteValue(prediction.quantiles[upper]);
		if (low !== null && high !== null && low <= high && lower !== upper)
			point.interval = [low, high];
	}
	return [...points.values()].sort((a, b) => a.timestamp - b.timestamp);
}

/** Join the series visually without adding an observed value to the predictions. */
export function forecastBoundarySegment(
	points: readonly ForecastPlotPoint[],
	cutoff?: number,
): [{ x: number; y: number }, { x: number; y: number }] | undefined {
	const index = points.findIndex((point) => point.timestamp === cutoff);
	if (index < 0) return;
	const observed = points[index];
	const predicted = points[index + 1];
	if (
		observed.history === null ||
		!Number.isFinite(observed.history) ||
		!predicted ||
		predicted.timestamp <= observed.timestamp ||
		predicted.forecast === null ||
		!Number.isFinite(predicted.forecast)
	)
		return;
	const gap = predicted.timestamp - observed.timestamp;
	const previous = points[index - 1];
	const next = points[index + 2];
	// A shorter neighboring interval indicates missing steps at the boundary.
	if (
		(previous && gap > observed.timestamp - previous.timestamp) ||
		(next && gap > next.timestamp - predicted.timestamp)
	)
		return;
	return [
		{ x: observed.timestamp, y: observed.history },
		{ x: predicted.timestamp, y: predicted.forecast },
	];
}

/** Keep endpoints, missing values and bucket extrema for long histories. */
export function sampleTimeline<T extends { timestamp: number }>(
	points: readonly T[],
	value: (point: T) => number | null,
	limit = 600,
): T[] {
	if (points.length <= limit || limit < 4) return [...points];
	const selected = new Set<number>([0, points.length - 1]);
	const bucketSize = Math.ceil(points.length / Math.floor(limit / 4));
	for (let start = 0; start < points.length; start += bucketSize) {
		let min = start;
		let max = start;
		let minValue = Number.POSITIVE_INFINITY;
		let maxValue = Number.NEGATIVE_INFINITY;
		let gap: number | undefined;
		for (
			let index = start;
			index < Math.min(start + bucketSize, points.length);
			index++
		) {
			const current = value(points[index]);
			if (current === null) {
				gap = index;
				continue;
			}
			if (current < minValue) {
				min = index;
				minValue = current;
			}
			if (current > maxValue) {
				max = index;
				maxValue = current;
			}
		}
		selected.add(min);
		selected.add(max);
		if (gap !== undefined) selected.add(gap);
	}
	return [...selected].sort((a, b) => a - b).map((index) => points[index]);
}

export function sampleForecastPlot(
	points: readonly ForecastPlotPoint[],
): ForecastPlotPoint[] {
	if (points.length <= 1600) return [...points];
	const retained = new Set<number>();
	const fields = ["history", "actual", "forecast"] as const;
	for (const field of fields) {
		const start = points.findIndex((point) => point[field] !== null);
		const end = points.findLastIndex((point) => point[field] !== null);
		if (start < 0) continue;
		// Keep adjacent steps and missing rows at each series boundary.
		for (const index of [start - 1, start + 1, end - 1, end + 1]) {
			if (points[index]) retained.add(points[index].timestamp);
		}
		for (const point of sampleTimeline(
			points.slice(start, end + 1),
			(point) => point[field],
			400,
		))
			retained.add(point.timestamp);
	}
	for (const boundary of [0, 1] as const) {
		const band = points.filter((point) => point.interval !== null);
		for (const point of sampleTimeline(
			band,
			(point) => point.interval?.[boundary] ?? null,
			400,
		))
			retained.add(point.timestamp);
	}
	return points.filter(
		(point, index) =>
			index === 0 ||
			index === points.length - 1 ||
			retained.has(point.timestamp),
	);
}

export function formatForecastTick(timestamp: number, span: number): string {
	if (!Number.isFinite(timestamp) || Math.abs(timestamp) > 8.64e15)
		return String(timestamp);
	return new Intl.DateTimeFormat(undefined, {
		timeZone: "UTC",
		...(span < 2 * 86400000
			? { hour: "2-digit", minute: "2-digit" }
			: { month: "short", day: "numeric" }),
	}).format(timestamp);
}

export function formatForecastNumber(value: number | null | undefined): string {
	if (value === null || value === undefined || !Number.isFinite(value))
		return "Unavailable";
	return new Intl.NumberFormat(undefined, {
		maximumFractionDigits: 3,
		notation: Math.abs(value) >= 100_000 ? "compact" : "standard",
	}).format(value);
}

export function formatForecastTime(
	timestamp: number,
	detailed = false,
): string {
	if (!Number.isFinite(timestamp) || Math.abs(timestamp) > 8.64e15)
		return String(timestamp);
	return new Intl.DateTimeFormat(undefined, {
		month: "short",
		day: "numeric",
		timeZone: "UTC",
		...(detailed
			? {
					year: "numeric",
					hour: "2-digit",
					minute: "2-digit",
					...(timestamp % 60000 !== 0 ? { second: "2-digit" as const } : {}),
					...(timestamp % 1000 !== 0
						? { fractionalSecondDigits: 3 as const }
						: {}),
				}
			: {}),
	}).format(timestamp);
}

import { describe, expect, it } from "bun:test";
import {
	buildForecastPlot,
	finiteValue,
	forecastBoundarySegment,
	forecastCovariateSupport,
	forecastOutputLabel,
	forecastingColumns,
	formatForecastNumber,
	formatForecastTime,
	numericInputValue,
	sampleForecastPlot,
	sampleTimeline,
	tableForecastSql,
	timestampValue,
	visibleCovariatePoints,
} from "./forecast-data";

describe("forecast source preparation", () => {
	it("distinguishes historical cutoff labels for second and millisecond data", () => {
		const minute = Date.UTC(2026, 9, 18, 12, 30);
		expect(formatForecastTime(minute + 1000, true)).not.toBe(
			formatForecastTime(minute + 2000, true),
		);
		expect(formatForecastTime(minute + 1010, true)).not.toBe(
			formatForecastTime(minute + 1020, true),
		);
	});
	it("quotes complete table names, including embedded quotes and dots", () => {
		expect(tableForecastSql('daily.sales"2026')).toBe(
			'SELECT * FROM "daily.sales""2026"',
		);
		expect(tableForecastSql("orders; DELETE FROM orders")).toBe(
			'SELECT * FROM "orders; DELETE FROM orders"',
		);
		expect(tableForecastSql("")).toBe("");
	});
	it("infers timestamp and numeric metrics without treating IDs or dates as measures", () => {
		const columns = [
			{ name: "store", type_name: "Utf8", position: 0 },
			{
				name: "timestamp",
				type_name: "Timestamp(Millisecond, None)",
				position: 1,
			},
			{ name: "sales", type_name: "Float64", position: 2 },
			{ name: "promotion", type_name: "Boolean", position: 3 },
			{ name: "money", type_name: "Decimal128(20, 2)", position: 4 },
		];
		const result = forecastingColumns(columns);
		expect(result.timestamp).toBe("timestamp");
		expect(result.target).toBe("sales");
		expect(result.numeric.map((column) => column.name)).toEqual(["sales"]);
	});
	it("offers only numeric Arrow types supported by the forecasting service", () => {
		const supported = [
			"Int8",
			"UInt16",
			"Int32",
			"UInt64",
			"Float16",
			"Float32",
			"Float64",
		];
		const types = [
			...supported,
			"Decimal256(20, 2)",
			"Float128",
			"Int128",
			"List(Float64)",
		];
		expect(
			forecastingColumns(
				types.map((type_name, position) => ({
					name: type_name,
					type_name,
					position,
				})),
			).numeric.map((column) => column.type_name),
		).toEqual(supported);
	});
	it("uses integer timestamps as epoch milliseconds", () => {
		expect(timestampValue(1729000000000)).toBe(1729000000000);
		expect(timestampValue("1729000000000")).toBe(1729000000000);
		expect(timestampValue("2026-10-18T00:00:00Z")).toBe(Date.UTC(2026, 9, 18));
		expect(timestampValue(1.5)).toBeNull();
		expect(timestampValue("not a date")).toBeNull();
		expect(timestampValue(null)).toBeNull();
	});
	it("normalizes Arrow date and timestamp units to whole milliseconds", () => {
		const epochMs = Date.UTC(2026, 9, 18);
		expect(timestampValue(epochMs / 86400000, "Date32")).toBe(epochMs);
		expect(timestampValue(epochMs, "Date64")).toBe(epochMs);
		expect(
			timestampValue(epochMs / 1000, 'Timestamp(Second, Some("UTC"))'),
		).toBe(epochMs);
		expect(timestampValue(epochMs, "Timestamp(Millisecond, None)")).toBe(
			epochMs,
		);
		expect(timestampValue(epochMs * 1000, "Timestamp(Microsecond, None)")).toBe(
			epochMs,
		);
		expect(
			timestampValue(
				String(BigInt(epochMs) * 1000000n),
				"Timestamp(Nanosecond, None)",
			),
		).toBe(epochMs);
		expect(timestampValue(-86400000, "Timestamp(Nanosecond, None)")).toBeNull();
		expect(timestampValue(-2000000, "Timestamp(Nanosecond, None)")).toBe(-2);
		expect(timestampValue(-1, "Date32")).toBe(-86400000);
	});
	it("rejects imprecise, overflowing and fractional-millisecond timestamps", () => {
		expect(timestampValue(1001, "Timestamp(Microsecond, None)")).toBeNull();
		expect(
			timestampValue("-1000001", "Timestamp(Nanosecond, None)"),
		).toBeNull();
		expect(
			timestampValue(
				Number.MAX_SAFE_INTEGER + 1,
				"Timestamp(Nanosecond, None)",
			),
		).toBeNull();
		expect(timestampValue(Number.MAX_SAFE_INTEGER, "Date32")).toBeNull();
		expect(
			timestampValue(Number.MAX_SAFE_INTEGER, "Timestamp(Second, None)"),
		).toBeNull();
		expect(timestampValue("9007199254740992", "Int64")).toBeNull();
		expect(
			timestampValue("9007199254740991000000", "Timestamp(Nanosecond, None)"),
		).toBeNull();
		expect(
			timestampValue("9223372036854000000", "Timestamp(Nanosecond, None)"),
		).toBe(9223372036854);
		expect(timestampValue(1, "Timestamp(Unknown, None)")).toBeNull();
	});
	it("does not coerce missing observations into zero", () => {
		expect(finiteValue(null)).toBeNull();
		expect(finiteValue(Number.NaN)).toBeNull();
		expect(finiteValue(Number.POSITIVE_INFINITY)).toBeNull();
		expect(finiteValue("12")).toBeNull();
		expect(finiteValue(0)).toBe(0);
		expect(formatForecastNumber(null)).toBe("Unavailable");
	});
});

describe("checkpoint covariate support", () => {
	it("disables unsupported inputs before running TimesFM or TinyTimeMixer", () => {
		for (const family of ["timesfm2_5", "tiny_time_mixer"])
			expect(forecastCovariateSupport("model", undefined, family)).toEqual({
				past: false,
				known: false,
				verified: false,
			});
	});
	it("permits either role until the selected checkpoint has reported capabilities", () => {
		for (const modelId of ["timestral-bit", "chronos-bit", "toto-bit"])
			expect(forecastCovariateSupport(modelId)).toEqual({
				past: true,
				known: true,
				verified: false,
			});
	});
	it("uses checkpoint capabilities independently for past and known inputs", () => {
		const result = {
			model: { bit_id: "timestral-bit" },
			capabilities: { past_covariates: true, known_covariates: false },
		};
		expect(forecastCovariateSupport("timestral-bit", result)).toEqual({
			past: true,
			known: false,
			verified: true,
		});
		expect(forecastCovariateSupport("another-bit", result)).toEqual({
			past: true,
			known: true,
			verified: false,
		});
		expect(
			forecastCovariateSupport("toto-bit", {
				model: { bit_id: "toto-bit" },
				capabilities: { past_covariates: false, known_covariates: false },
			}),
		).toEqual({ past: false, known: false, verified: true });
	});
});

describe("forecast chart", () => {
	it("joins history to the next prediction without inventing a forecast at the cutoff", () => {
		const points = buildForecastPlot(
			[{ timestamp: 1000, value: 10 }],
			[{ timestamp: 2000, point: 12, quantiles: [11, 14] }],
			1000,
			[0.1, 0.9],
		);
		const original = structuredClone(points);
		expect(forecastBoundarySegment(points, 1000)).toEqual([
			{ x: 1000, y: 10 },
			{ x: 2000, y: 12 },
		]);
		expect(points).toEqual(original);
		expect(points[0]).toMatchObject({ forecast: null, interval: null });
		expect(points[1]).toMatchObject({ forecast: 12, interval: [11, 14] });
	});
	it("leaves missing observations, missing predictions and longer time gaps unconnected", () => {
		const build = (lastValue: number | null, firstPrediction = 3000) =>
			buildForecastPlot(
				[
					{ timestamp: 1000, value: 8 },
					{ timestamp: 2000, value: lastValue },
				],
				[{ timestamp: firstPrediction, point: 12, quantiles: [] }],
				2000,
				[],
			);
		expect(forecastBoundarySegment(build(null), 2000)).toBeUndefined();
		expect(forecastBoundarySegment(build(10, 4000), 2000)).toBeUndefined();
		expect(forecastBoundarySegment(build(10), 1500)).toBeUndefined();
		expect(forecastBoundarySegment(build(10))).toBeUndefined();
		expect(forecastBoundarySegment([], 2000)).toBeUndefined();
		const missingForecast = build(10);
		missingForecast[2].forecast = null;
		missingForecast.push({
			timestamp: 4000,
			history: null,
			actual: null,
			forecast: 12,
			interval: null,
		});
		expect(forecastBoundarySegment(missingForecast, 2000)).toBeUndefined();
	});
	it("preserves boundary gaps and cadence when reducing long charts", () => {
		const history = Array.from({ length: 4000 }, (_, timestamp) => ({
			timestamp,
			value: timestamp % 10,
		}));
		const predictions = Array.from({ length: 2000 }, (_, index) => ({
			timestamp: 4000 + index,
			point: 5,
			quantiles: [],
		}));
		const points = buildForecastPlot(history, predictions, 3999, []);
		expect(forecastBoundarySegment(sampleForecastPlot(points), 3999)).toEqual([
			{ x: 3999, y: 9 },
			{ x: 4000, y: 5 },
		]);
		points[4000].forecast = null;
		expect(
			forecastBoundarySegment(sampleForecastPlot(points), 3999),
		).toBeUndefined();
		points[3999].history = null;
		points[4000].forecast = 5;
		expect(
			forecastBoundarySegment(sampleForecastPlot(points), 3999),
		).toBeUndefined();
		points[3999].history = 9;
		points.splice(4000, 10);
		expect(
			forecastBoundarySegment(sampleForecastPlot(points), 3999),
		).toBeUndefined();
	});
	it("preserves point-only forecasts without inventing quantile bounds", () => {
		const plot = buildForecastPlot(
			[{ timestamp: 1, value: 10 }],
			[{ timestamp: 2, point: 12.5, quantiles: [] }],
			1,
			[],
		);
		expect(plot[1]).toMatchObject({ forecast: 12.5, interval: null });
		expect(forecastOutputLabel([], true)).toBe("Point forecast only");
		expect(forecastOutputLabel([])).toBe("Model defaults");
		expect(forecastOutputLabel([0.1, 0.5, 0.9])).toBe("P10, P50, P90");
	});
	it("withholds past covariate values from the evaluation horizon", () => {
		const points = [
			{ timestamp: 1, value: 3 },
			{ timestamp: 2, value: 4 },
			{ timestamp: 3, value: 5 },
		];
		expect(visibleCovariatePoints(points, "past", true, 2)).toEqual(
			points.slice(0, 2),
		);
		expect(visibleCovariatePoints(points, "known", true, 2)).toEqual(points);
		expect(visibleCovariatePoints(points, "past", false, 2)).toEqual(points);
		expect(points.length).toBe(3);
	});
	it("sorts timestamps and preserves missing observations", () => {
		const result = buildForecastPlot(
			[
				{ timestamp: 2, value: null },
				{ timestamp: 1, value: 0 },
			],
			[{ timestamp: 3, point: 10, quantiles: [8, 10, 12] }],
			2,
			[0.1, 0.5, 0.9],
		);
		expect(result).toEqual([
			{
				timestamp: 1,
				history: 0,
				actual: null,
				forecast: null,
				interval: null,
			},
			{
				timestamp: 2,
				history: null,
				actual: null,
				forecast: null,
				interval: null,
			},
			{
				timestamp: 3,
				history: null,
				actual: null,
				forecast: 10,
				interval: [8, 12],
			},
		]);
	});
	it("separates held-out actuals after the cutoff and never forecasts inside history", () => {
		const result = buildForecastPlot(
			[
				{ timestamp: 1, value: 5 },
				{ timestamp: 2, value: 7 },
				{ timestamp: 3, value: null },
			],
			[
				{ timestamp: 1, point: 99, quantiles: [] },
				{ timestamp: 2, point: 6, quantiles: [4, 8] },
				{ timestamp: 3, point: 8, quantiles: [5, 9] },
			],
			1,
			[0.1, 0.9],
		);
		expect(result[0].forecast).toBeNull();
		expect(result[1]).toMatchObject({ history: null, actual: 7, forecast: 6 });
		expect(result[2]).toMatchObject({ actual: null, forecast: 8 });
	});
	it("uses quantile levels to find bounds and avoids an invented band", () => {
		const result = buildForecastPlot(
			[],
			[{ timestamp: 1, point: 10, quantiles: [12, 8, 10] }],
			0,
			[0.9, 0.1, 0.5],
		);
		expect(result[0].interval).toEqual([8, 12]);
		expect(
			buildForecastPlot(
				[],
				[{ timestamp: 1, point: 10, quantiles: [10] }],
				0,
				[0.5],
			)[0].interval,
		).toBeNull();
		expect(
			buildForecastPlot(
				[],
				[{ timestamp: 1, point: 10, quantiles: [12, 8] }],
				0,
				[0.1, 0.9],
			)[0].interval,
		).toBeNull();
	});
	it("retains both sides of the forecast boundary and uncertainty spikes when reducing long charts", () => {
		const history = Array.from({ length: 4000 }, (_, index) => ({
			timestamp: index,
			value: index % 10,
		}));
		const forecast = Array.from({ length: 2000 }, (_, index) => ({
			timestamp: 4000 + index,
			point: 5,
			quantiles: [0, index === 913 ? 1000 : 10],
		}));
		const points = sampleForecastPlot(
			buildForecastPlot(history, forecast, 3999, [0.1, 0.9]),
		);
		expect(points.length).toBeLessThan(1600);
		expect(points[0].timestamp).toBe(0);
		expect(points.at(-1)?.timestamp).toBe(5999);
		expect(
			points.some((point) => point.timestamp === 3999 && point.history === 9),
		).toBe(true);
		expect(
			points.some((point) => point.timestamp === 4000 && point.forecast === 5),
		).toBe(true);
		expect(points.some((point) => point.interval?.[1] === 1000)).toBe(true);
	});
	it("keeps timeline endpoints and a missing value in sampled buckets", () => {
		const points = Array.from({ length: 5000 }, (_, index) => ({
			timestamp: index,
			value: index === 2500 ? null : index === 1000 ? 9999 : index % 10,
		}));
		const sampled = sampleTimeline(points, (point) => point.value, 100);
		expect(sampled[0].timestamp).toBe(0);
		expect(sampled.at(-1)?.timestamp).toBe(4999);
		expect(sampled.some((point) => point.value === null)).toBe(true);
		expect(sampled.some((point) => point.value === 9999)).toBe(true);
	});
});

it("recognizes numeric text from sampled values without turning missing values or IDs into metrics", () => {
	const columns = [
		{ name: "timestamp_utc", type_name: 'Timestamp(ms, "UTC")', position: 0 },
		{ name: "machine_id", type_name: "LargeUtf8", position: 1 },
		{ name: "temperature_c", type_name: "LargeUtf8", position: 2 },
		{ name: "bad", type_name: "Utf8", position: 3 },
	];
	const inferred = forecastingColumns(columns, [
		{
			timestamp_utc: 1000,
			machine_id: "MACHINE_01",
			temperature_c: "62.43",
			bad: "12",
		},
		{
			timestamp_utc: 2000,
			machine_id: "MACHINE_02",
			temperature_c: null,
			bad: "offline",
		},
	]);
	expect(inferred.timestamp).toBe("timestamp_utc");
	expect(inferred.numeric.map((column) => column.name)).toEqual([
		"temperature_c",
	]);
	for (const invalid of [
		"",
		" ",
		"NaN",
		"Infinity",
		"1,234",
		"$12",
		"1e100",
		null,
	])
		expect(numericInputValue(invalid)).toBeNull();
	expect(numericInputValue(" 6.243e1 ")).toBe(62.43);
});

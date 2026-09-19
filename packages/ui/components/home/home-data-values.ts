import {
	looksLikeEpochNumber,
	parseTemporalValue,
	temporalUnitFromTypeName,
} from "../../lib/date";
import { resolveStorageFile } from "../../lib/storage-file";
import type { ExecuteSqlResult } from "../../state/backend-state/query-state";
import { accountIdFromValue } from "../../state/backend-state/user-state";
import {
	type ColumnKind,
	classifyResultColumn,
	formatNumber,
} from "../settings/data-studio/query-workbench/column-types";
import type { HomeDataConfig } from "./home-data-query";
import { HOME_DATA_EMPTY_LABEL, homeDataText } from "./home-data-text";

type HomeDataResult = Pick<ExecuteSqlResult, "columns" | "rows">;
export type HomeDataFormatConfig = Pick<
	HomeDataConfig,
	"appId" | "timeBucket" | "visualization" | "groupBy" | "seriesBy" | "measures"
>;

/**
 * How a result column reads, judged under the name of the source column it came
 * from. Aggregates alias every group to `__group`, which says nothing about
 * whether it holds dates or people, so the name gates look at `sourceName`.
 */
export function homeDataColumnKind(
	result: HomeDataResult,
	resultName: string,
	sourceName = resultName,
	appId?: string,
): ColumnKind {
	const column = result.columns.find((item) => item.name === resultName);
	if (!column) return "text";
	if (!sourceName || sourceName === resultName)
		return classifyResultColumn(column, result.rows, appId);
	return classifyResultColumn(
		{ ...column, name: sourceName },
		result.rows.slice(0, 100).map((row) => ({ [sourceName]: row[resultName] })),
		appId,
	);
}

/**
 * A result value read as an instant. Declared instants are read in the unit
 * their Arrow type names; a bare number only once it is large enough to be an
 * epoch, so an unset `0` or a duration never reads as 1970.
 */
export function homeDataTemporalValue(
	value: unknown,
	typeName = "",
): Date | null {
	if (value === null || value === undefined) return null;
	const unit = temporalUnitFromTypeName(typeName);
	const numeric =
		typeof value === "number" ||
		typeof value === "bigint" ||
		(typeof value === "string" && /^-?\d+(\.\d+)?$/.test(value.trim()));
	if (!numeric) return parseTemporalValue(value, unit);
	if (unit) return parseTemporalValue(Number(value), unit);
	return looksLikeEpochNumber(value) ? parseTemporalValue(Number(value)) : null;
}

export interface HomeDataLabelFormat {
	kind: ColumnKind;
	typeName: string;
	/** The source column the value came from, which the name gates judge. */
	sourceName: string;
	appId?: string;
	metadata?: Record<string, string>;
	bucket: HomeDataConfig["timeBucket"];
	/** Whether the dates on screen span more than the current year. */
	showYear: boolean;
	/** Whether every date on screen is a whole UTC day. */
	dateOnly: boolean;
	/** Whether the column is an aggregate the widget computed, not a stored value. */
	measure: boolean;
	userNames?: ReadonlyMap<string, string>;
}

const MEASURE_COLUMN = /^__measure_(\d+)$/;
const STATISTIC_COLUMNS = new Set([
	"__share",
	"__min",
	"__q1",
	"__q3",
	"__max",
]);
/** Aggregations whose result is one of the field's own values. */
const VALUE_AGGREGATIONS = new Set(["min", "max", "median"]);

function isUtcMidnight(date: Date): boolean {
	return (
		date.getUTCHours() === 0 &&
		date.getUTCMinutes() === 0 &&
		date.getUTCSeconds() === 0 &&
		date.getUTCMilliseconds() === 0
	);
}

/** The source column and kind behind an aggregate alias, or null for plain columns. */
function aggregateColumn(
	result: HomeDataResult,
	resultName: string,
	config: HomeDataFormatConfig,
): {
	sourceName: string;
	kind: ColumnKind;
	bucket: HomeDataLabelFormat["bucket"];
} | null {
	if (resultName === "__group") {
		if (config.visualization === "histogram")
			return { sourceName: config.groupBy, kind: "number", bucket: "none" };
		if (config.timeBucket !== "none")
			return {
				sourceName: config.groupBy,
				kind: "temporal",
				bucket: config.timeBucket,
			};
		return {
			sourceName: config.groupBy,
			kind: homeDataColumnKind(
				result,
				resultName,
				config.groupBy,
				config.appId,
			),
			bucket: "none",
		};
	}
	if (resultName === "__series")
		return {
			sourceName: config.seriesBy,
			kind: homeDataColumnKind(
				result,
				resultName,
				config.seriesBy,
				config.appId,
			),
			bucket: "none",
		};
	if (STATISTIC_COLUMNS.has(resultName))
		return { sourceName: resultName, kind: "number", bucket: "none" };
	const index = MEASURE_COLUMN.exec(resultName)?.[1];
	if (index === undefined) return null;
	const measure = config.measures[Number(index)];
	const valued =
		measure?.field &&
		VALUE_AGGREGATIONS.has(measure.aggregation) &&
		config.visualization !== "boxplot";
	return {
		sourceName: measure?.field || resultName,
		kind: valued
			? homeDataColumnKind(result, resultName, measure.field, config.appId)
			: "number",
		bucket: "none",
	};
}

/** Everything a reading of one result column needs, measured once over its rows. */
export function homeDataLabelFormat(
	result: HomeDataResult,
	resultName: string,
	config: HomeDataFormatConfig,
	userNames?: ReadonlyMap<string, string>,
): HomeDataLabelFormat {
	const column = result.columns.find((item) => item.name === resultName);
	const typeName = column?.type_name ?? "";
	const aggregate = aggregateColumn(result, resultName, config);
	const format: HomeDataLabelFormat = {
		kind:
			aggregate?.kind ??
			homeDataColumnKind(result, resultName, resultName, config.appId),
		typeName,
		sourceName: aggregate?.sourceName ?? resultName,
		appId: config.appId,
		metadata: column?.metadata,
		bucket: aggregate?.bucket ?? "none",
		showYear: false,
		dateOnly: (aggregate?.bucket ?? "none") !== "none",
		measure:
			MEASURE_COLUMN.test(resultName) || STATISTIC_COLUMNS.has(resultName),
		userNames,
	};
	if (format.kind !== "temporal") return format;
	const dates = result.rows.flatMap((row) => {
		const date = homeDataTemporalValue(row[resultName], typeName);
		return date ? [date] : [];
	});
	const years = new Set(dates.map((date) => date.getUTCFullYear()));
	format.showYear =
		years.size > 1 ||
		(years.size === 1 && !years.has(new Date().getUTCFullYear()));
	format.dateOnly =
		format.dateOnly ||
		temporalUnitFromTypeName(typeName) === "day" ||
		(dates.length > 0 && dates.every(isUtcMidnight));
	return format;
}

type DateLabelFormat = Pick<
	HomeDataLabelFormat,
	"bucket" | "showYear" | "dateOnly"
>;
const dateLabelFormatters = new Map<string, Intl.DateTimeFormat>();

function dateLabelOptions(format: DateLabelFormat): Intl.DateTimeFormatOptions {
	const year = format.showYear ? { year: "numeric" as const } : {};
	if (format.bucket === "year") return { year: "numeric", timeZone: "UTC" };
	if (format.bucket === "month")
		return { month: "short", year: "numeric", timeZone: "UTC" };
	if (format.dateOnly)
		return { month: "short", day: "numeric", ...year, timeZone: "UTC" };
	return {
		month: "short",
		day: "numeric",
		...year,
		hour: "2-digit",
		minute: "2-digit",
	};
}

/** A calendar reading of an instant at the precision its bucket implies. */
export function homeDataDateLabel(date: Date, format: DateLabelFormat): string {
	if (format.bucket === "quarter")
		return `Q${Math.floor(date.getUTCMonth() / 3) + 1} ${date.getUTCFullYear()}`;
	const key = `${format.bucket}|${format.showYear}|${format.dateOnly}`;
	let formatter = dateLabelFormatters.get(key);
	if (!formatter) {
		formatter = new Intl.DateTimeFormat(undefined, dateLabelOptions(format));
		dateLabelFormatters.set(key, formatter);
	}
	return formatter.format(date);
}

/** A result value as plain text, for axes, legends, tooltips and headers. */
export function homeDataValueLabel(
	value: unknown,
	format: HomeDataLabelFormat,
): string {
	if (value === null || value === undefined) return HOME_DATA_EMPTY_LABEL;
	if (format.kind === "temporal") {
		const date = homeDataTemporalValue(value, format.typeName);
		if (date) return homeDataDateLabel(date, format);
	}
	// Whole numbers are years, ids and codes far more often than quantities.
	if (format.kind === "number")
		return Number.isInteger(Number(value))
			? homeDataText(value)
			: formatNumber(value);
	if (format.kind === "user") {
		const userId = accountIdFromValue(value);
		if (userId) return format.userNames?.get(userId) ?? userId;
	}
	if (format.kind === "file") {
		const file = resolveStorageFile(format.sourceName, value, format.appId);
		if (file) return file.fileName;
	}
	return homeDataText(value);
}
